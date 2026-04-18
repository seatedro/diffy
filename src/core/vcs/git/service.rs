use std::cmp::Ordering;
use std::path::Path;

use git2::{
    ApplyLocation, BranchType, Cred, Diff, DiffFormat, DiffOptions, FetchOptions, ObjectType, Oid,
    RemoteCallbacks, Repository, build::CheckoutBuilder,
};
use serde::Serialize;

use crate::core::compare::backends::compare_output_from_diff;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareMode;
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::status::{StatusItem, StatusOperation, StatusScope};
use crate::core::vcs::github::{GitHubApi, parse_pr_url};

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteCredentialKind {
    UserPassPlaintext { username: String, password: String },
    SshKey { username: String },
    Username { username: String },
    Default,
}

fn select_remote_credential(
    remote_url: &str,
    username: Option<&str>,
    allowed: git2::CredentialType,
    github_token: &str,
) -> RemoteCredentialKind {
    if remote_url.starts_with("http")
        && !github_token.is_empty()
        && allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
    {
        return RemoteCredentialKind::UserPassPlaintext {
            username: username.unwrap_or("x-access-token").to_owned(),
            password: github_token.to_owned(),
        };
    }

    if allowed.contains(git2::CredentialType::SSH_KEY) {
        return RemoteCredentialKind::SshKey {
            username: username.unwrap_or("git").to_owned(),
        };
    }

    if allowed.contains(git2::CredentialType::USERNAME) {
        return RemoteCredentialKind::Username {
            username: username
                .unwrap_or(if remote_url.starts_with("http") {
                    "x-access-token"
                } else {
                    "git"
                })
                .to_owned(),
        };
    }

    RemoteCredentialKind::Default
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagInfo {
    pub name: String,
    pub target_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitInfo {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author_name: String,
    pub timestamp: i64,
}

pub const WORKDIR_REF: &str = "@workdir";
pub const INDEX_REF: &str = "@index";
pub const PR_REF_PREFIX: &str = "refs/diffy/pr/";

pub fn pr_ref_path(pr_number: i32, branch: &str) -> String {
    format!("{PR_REF_PREFIX}{pr_number}/{branch}")
}

/// Remove stale refs from prior fetches for this PR. Keeps only the targets
/// the latest fetch wrote, and also cleans up the old `refs/diffy/pull/{N}/*`
/// scheme we used to use. Uses a prefix filter rather than `references_glob`
/// because libgit2's glob `*` does not span `/`, which would miss branches
/// that contain slashes.
fn prune_stale_pr_refs(
    repo: &Repository,
    pr_number: i32,
    keep_base: &str,
    keep_head: &str,
) {
    let prefixes = [
        format!("{PR_REF_PREFIX}{pr_number}/"),
        format!("refs/diffy/pull/{pr_number}/"),
    ];
    let Ok(iter) = repo.references() else {
        return;
    };
    let stale: Vec<String> = iter
        .filter_map(|r| r.ok()?.name().map(str::to_owned))
        .filter(|name| {
            name != keep_base
                && name != keep_head
                && prefixes.iter().any(|p| name.starts_with(p))
        })
        .collect();
    for name in stale {
        if let Ok(mut r) = repo.find_reference(&name) {
            let _ = r.delete();
        }
    }
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find('\n') {
        let (head, tail) = rest.split_at(idx);
        let line = head.strip_suffix('\r').unwrap_or(head);
        out.push(line.to_owned());
        rest = &tail[1..];
    }
    if !rest.is_empty() {
        let line = rest.strip_suffix('\r').unwrap_or(rest);
        out.push(line.to_owned());
    }
    out
}

#[derive(Default)]
pub struct GitService {
    repo: Option<Repository>,
    repo_path: String,
    github_token: String,
}

impl std::fmt::Debug for GitService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitService")
            .field("repo_path", &self.repo_path)
            .field("is_open", &self.is_open())
            .finish()
    }
}

impl GitService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_github_token(&mut self, token: impl Into<String>) {
        self.github_token = token.into();
    }

    pub fn open(&mut self, path: &str) -> Result<()> {
        self.close();
        let repo = Repository::open(path)?;
        self.repo = Some(repo);
        self.repo_path = path.to_owned();
        Ok(())
    }

    pub fn close(&mut self) {
        self.repo = None;
        self.repo_path.clear();
    }

    pub fn is_open(&self) -> bool {
        self.repo.is_some()
    }

    pub fn repo_path(&self) -> &str {
        &self.repo_path
    }

    pub fn refs(&self) -> Result<Vec<String>> {
        let mut refs = self
            .repo()?
            .references()?
            .flatten()
            .filter_map(|reference| reference.shorthand().map(str::to_owned))
            .collect::<Vec<_>>();
        refs.sort();
        refs.dedup();
        Ok(refs)
    }

    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let mut branches = Vec::new();
        for branch in self.repo()?.branches(None)? {
            let (branch, branch_type) = branch?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            branches.push(BranchInfo {
                name,
                is_remote: branch_type == BranchType::Remote,
                is_head: branch.is_head(),
            });
        }
        branches.sort_by(|left, right| match right.is_head.cmp(&left.is_head) {
            Ordering::Equal => match left.is_remote.cmp(&right.is_remote) {
                Ordering::Equal => left.name.cmp(&right.name),
                other => other,
            },
            other => other,
        });
        Ok(branches)
    }

    pub fn tags(&self) -> Result<Vec<TagInfo>> {
        let repo = self.repo()?;
        let mut tags = repo
            .tag_names(None)?
            .iter()
            .flatten()
            .map(|name| TagInfo {
                name: name.to_owned(),
                target_oid: repo
                    .revparse_single(&format!("refs/tags/{name}"))
                    .ok()
                    .and_then(|object| object.peel(ObjectType::Commit).ok())
                    .map(|object| object.id().to_string())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        tags.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tags)
    }

    pub fn commits(&self, reference: &str, max_count: usize) -> Result<Vec<CommitInfo>> {
        let repo = self.repo()?;
        let start_oid = self.resolve_commit_oid(reference)?;
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.push(start_oid)?;

        walk.take(max_count)
            .map(|entry| {
                entry
                    .map_err(Into::into)
                    .and_then(|oid| self.commit_info(repo, oid))
            })
            .collect()
    }

    pub fn commits_in_range(
        &self,
        left: &str,
        right: &str,
        max_count: usize,
    ) -> Result<Vec<CommitInfo>> {
        let repo = self.repo()?;
        let right_oid = self.resolve_commit_oid(right)?;
        let left_oid = self.resolve_commit_oid(left)?;
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.push(right_oid)?;
        walk.hide(left_oid)?;

        walk.take(max_count)
            .map(|entry| {
                entry
                    .map_err(Into::into)
                    .and_then(|oid| self.commit_info(repo, oid))
            })
            .collect()
    }

    pub fn search_commits(&self, hex_prefix: &str) -> Result<Vec<CommitInfo>> {
        if hex_prefix.len() < 4 {
            return Ok(Vec::new());
        }
        let repo = self.repo()?;
        let mut walk = repo.revwalk()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.push_head()?;
        let prefix = hex_prefix.to_ascii_lowercase();
        let mut results = Vec::new();
        for oid in walk.flatten() {
            if oid.to_string().starts_with(&prefix) {
                results.push(self.commit_info(repo, oid)?);
                if results.len() >= 50 {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn resolve_ref(&self, reference: &str) -> Result<String> {
        Ok(self.resolve_commit_oid(reference)?.to_string())
    }

    pub fn read_file_lines_at(&self, reference: &str, path: &str) -> Result<Vec<String>> {
        let repo = self.repo()?;
        let bytes: Vec<u8> = if reference == WORKDIR_REF {
            let full = Path::new(&self.repo_path).join(path);
            std::fs::read(&full)?
        } else if reference == INDEX_REF {
            let index = repo.index()?;
            let entry = index.get_path(Path::new(path), 0).ok_or_else(|| {
                DiffyError::General(format!("path {path} is not present in the index"))
            })?;
            repo.find_blob(entry.id)?.content().to_vec()
        } else {
            let oid = self.resolve_commit_oid(reference)?;
            let commit = repo.find_commit(oid)?;
            let tree = commit.tree()?;
            let entry = tree.get_path(Path::new(path)).map_err(|_| {
                DiffyError::General(format!("path {path} is not present at {reference}"))
            })?;
            let object = entry.to_object(repo)?;
            let blob = object.as_blob().ok_or_else(|| {
                DiffyError::General(format!("path {path} at {reference} is not a blob"))
            })?;
            blob.content().to_vec()
        };

        if bytes.contains(&0u8) {
            return Err(DiffyError::General(format!(
                "path {path} is binary at {reference}",
            )));
        }

        let text = std::str::from_utf8(&bytes).map_err(|e| {
            DiffyError::General(format!(
                "path {path} at {reference} is not valid UTF-8: {e}"
            ))
        })?;

        Ok(split_lines(text))
    }

    pub fn diff_status_item(&self, item: &StatusItem) -> Result<CompareOutput> {
        let repo = self.repo()?;
        let mut options = DiffOptions::new();
        options.context_lines(3);
        options.pathspec(&item.path);

        let mut diff = match item.scope {
            StatusScope::Staged => {
                let mut index = repo.index()?;
                let head_tree = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
                repo.diff_tree_to_index(head_tree.as_ref(), Some(&mut index), Some(&mut options))?
            }
            StatusScope::Unstaged | StatusScope::Untracked => {
                options.include_untracked(true);
                options.recurse_untracked_dirs(true);
                let mut index = repo.index()?;
                repo.diff_index_to_workdir(Some(&mut index), Some(&mut options))?
            }
        };

        compare_output_from_diff(&mut diff)
    }

    pub fn apply_status_operation(
        &self,
        item: &StatusItem,
        operation: StatusOperation,
    ) -> Result<()> {
        match operation {
            StatusOperation::Stage => self.stage_path(&item.path),
            StatusOperation::Unstage => self.unstage_path(&item.path),
            StatusOperation::Discard => self.discard_path(&item.path),
        }
    }

    pub fn apply_batch_status_operation(
        &self,
        items: &[StatusItem],
        operation: StatusOperation,
    ) -> Result<()> {
        match operation {
            StatusOperation::Stage => {
                let repo = self.repo()?;
                let mut index = repo.index()?;
                for item in items {
                    index.add_path(Path::new(&item.path))?;
                }
                index.write()?;
                Ok(())
            }
            StatusOperation::Unstage => {
                let repo = self.repo()?;
                let head = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
                let head_object = head.as_ref().map(|c| c.as_object());
                let paths: Vec<&Path> = items.iter().map(|i| Path::new(i.path.as_str())).collect();
                repo.reset_default(head_object, paths)?;
                Ok(())
            }
            StatusOperation::Discard => {
                for item in items {
                    self.discard_path(&item.path)?;
                }
                Ok(())
            }
        }
    }

    pub fn abbreviate_oid(&self, full_oid: &str) -> Result<String> {
        let oid = Oid::from_str(full_oid)?;
        let short = self.repo()?.find_object(oid, None)?.short_id()?;
        Ok(short.as_str().unwrap_or(full_oid).to_owned())
    }

    pub fn resolve_oid_to_branch_name(&self, oid_hex: &str) -> Result<String> {
        if oid_hex.len() != 40 {
            return Ok(String::new());
        }
        let target = Oid::from_str(oid_hex)?;
        for branch in self.repo()?.branches(Some(BranchType::Local))? {
            let (branch, _) = branch?;
            let Some(name) = branch.name()?.map(str::to_owned) else {
                continue;
            };
            let Some(branch_oid) = branch.into_reference().resolve()?.target() else {
                continue;
            };
            if branch_oid == target {
                return Ok(name);
            }
        }
        Ok(String::new())
    }

    pub fn resolve_comparison(
        &self,
        left_ref: &str,
        right_ref: &str,
        mode: CompareMode,
    ) -> Result<(String, String)> {
        let repo = self.repo()?;
        match mode {
            CompareMode::SingleCommit => {
                let commit_ref = if right_ref.is_empty() {
                    left_ref
                } else {
                    right_ref
                };
                if commit_ref.is_empty() {
                    return Err(DiffyError::Parse(
                        "commit mode requires a commit reference".to_owned(),
                    ));
                }
                let right_oid = self.resolve_commit_oid(commit_ref)?;
                let commit = repo.find_commit(right_oid)?;
                let parent = commit.parent(0).map_err(|_| {
                    DiffyError::Parse("cannot diff the root commit in commit mode yet".to_owned())
                })?;
                Ok((parent.id().to_string(), commit.id().to_string()))
            }
            CompareMode::TwoDot | CompareMode::ThreeDot => {
                if left_ref.is_empty() || right_ref.is_empty() {
                    return Err(DiffyError::Parse(
                        "comparison requires both left and right references".to_owned(),
                    ));
                }
                if right_ref == WORKDIR_REF {
                    let left_oid = self.resolve_commit_oid(left_ref)?;
                    return Ok((left_oid.to_string(), WORKDIR_REF.to_owned()));
                }
                let mut left_oid = self.resolve_commit_oid(left_ref)?;
                let right_oid = self.resolve_commit_oid(right_ref)?;
                if mode == CompareMode::ThreeDot {
                    left_oid = repo.merge_base(left_oid, right_oid)?;
                }
                Ok((left_oid.to_string(), right_oid.to_string()))
            }
        }
    }

    pub fn diff_two_refs(&self, left: &str, right: &str) -> Result<String> {
        self.diff_between_refs(left, right)
    }

    pub fn diff_three_refs(&self, left: &str, right: &str) -> Result<String> {
        let (resolved_left, resolved_right) =
            self.resolve_comparison(left, right, CompareMode::ThreeDot)?;
        self.diff_between_refs(&resolved_left, &resolved_right)
    }

    pub fn diff_single_commit(&self, reference: &str) -> Result<String> {
        let (left, right) = self.resolve_comparison(reference, "", CompareMode::SingleCommit)?;
        self.diff_between_refs(&left, &right)
    }

    fn fetch_refspecs(&self, repo_url: &str, refspecs: &[String]) -> Result<()> {
        let repo = self.repo()?;
        let mut remote = repo.remote_anonymous(repo_url)?;
        let mut callbacks = RemoteCallbacks::new();
        let github_token = self.github_token.clone();
        callbacks.credentials(move |url, username, allowed| {
            match select_remote_credential(url, username, allowed, &github_token) {
                RemoteCredentialKind::UserPassPlaintext { username, password } => {
                    Cred::userpass_plaintext(&username, &password)
                }
                RemoteCredentialKind::SshKey { username } => Cred::ssh_key_from_agent(&username),
                RemoteCredentialKind::Username { username } => Cred::username(&username),
                RemoteCredentialKind::Default => Cred::default(),
            }
        });
        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);
        let specs: Vec<&str> = refspecs.iter().map(String::as_str).collect();
        remote.fetch(&specs, Some(&mut fetch_options), None)?;
        Ok(())
    }

    pub fn resolve_pull_request_comparison(
        &self,
        pull_request_url: &str,
    ) -> Result<(String, String)> {
        let parsed = parse_pr_url(pull_request_url)
            .ok_or_else(|| DiffyError::Parse("not a valid GitHub pull request URL".to_owned()))?;
        let api = GitHubApi::with_token(self.github_token.clone());
        let info = api.fetch_pull_request(&parsed.owner, &parsed.repo, parsed.number)?;
        let repo_url = if info.base_repo_url.is_empty() {
            format!("https://github.com/{}/{}.git", parsed.owner, parsed.repo)
        } else {
            info.base_repo_url.clone()
        };
        let base_source = if info.base_sha.is_empty() {
            format!("refs/heads/{}", info.base_branch)
        } else {
            info.base_sha.clone()
        };
        let head_source = if info.head_sha.is_empty() {
            format!("refs/heads/{}", info.head_branch)
        } else {
            info.head_sha.clone()
        };
        let base_target = pr_ref_path(parsed.number, &info.base_branch);
        let head_target = pr_ref_path(parsed.number, &info.head_branch);
        self.fetch_refspecs(
            &repo_url,
            &[
                format!("+{base_source}:{base_target}"),
                format!("+{head_source}:{head_target}"),
            ],
        )?;
        prune_stale_pr_refs(self.repo()?, parsed.number, &base_target, &head_target);
        Ok((base_target, head_target))
    }

    fn diff_between_refs(&self, left: &str, right: &str) -> Result<String> {
        let repo = self.repo()?;
        let left_commit = repo.find_commit(self.resolve_commit_oid(left)?)?;
        let left_tree = left_commit.tree()?;

        let mut options = DiffOptions::new();
        options.context_lines(3);

        let diff = if right == WORKDIR_REF {
            let mut diff =
                repo.diff_tree_to_workdir_with_index(Some(&left_tree), Some(&mut options))?;
            diff.find_similar(None)?;
            diff
        } else {
            let right_commit = repo.find_commit(self.resolve_commit_oid(right)?)?;
            let right_tree = right_commit.tree()?;
            let mut diff =
                repo.diff_tree_to_tree(Some(&left_tree), Some(&right_tree), Some(&mut options))?;
            diff.find_similar(None)?;
            diff
        };

        let mut patch = String::new();
        diff.print(DiffFormat::Patch, |_delta, _hunk, line| {
            patch.push_str(std::str::from_utf8(line.content()).unwrap_or_default());
            true
        })?;
        Ok(patch)
    }

    pub fn resolve_commit_oid(&self, reference: &str) -> Result<Oid> {
        let object = self.repo()?.revparse_single(reference)?;
        Ok(object.peel(ObjectType::Commit)?.id())
    }

    fn commit_info(&self, repo: &Repository, oid: Oid) -> Result<CommitInfo> {
        let commit = repo.find_commit(oid)?;
        Ok(CommitInfo {
            oid: oid.to_string(),
            short_oid: self.abbreviate_oid(&oid.to_string())?,
            summary: commit.summary().unwrap_or_default().to_owned(),
            author_name: commit.author().name().unwrap_or_default().to_owned(),
            timestamp: commit.time().seconds(),
        })
    }

    pub fn repo(&self) -> Result<&Repository> {
        self.repo
            .as_ref()
            .ok_or_else(|| DiffyError::General("repository is not open".to_owned()))
    }

    pub fn commit(&self, message: &str) -> Result<Oid> {
        let repo = self.repo()?;
        let mut index = repo.index()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = repo.signature()?;
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid))
            .transpose()?;
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )?;
        Ok(oid)
    }

    fn stage_path(&self, path: &str) -> Result<()> {
        let repo = self.repo()?;
        let mut index = repo.index()?;
        index.add_path(Path::new(path))?;
        index.write()?;
        Ok(())
    }

    fn unstage_path(&self, path: &str) -> Result<()> {
        let repo = self.repo()?;
        let head = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let head_object = head.as_ref().map(|commit| commit.as_object());
        repo.reset_default(head_object, [Path::new(path)])?;
        Ok(())
    }

    fn discard_path(&self, path: &str) -> Result<()> {
        let repo = self.repo()?;
        let mut checkout = CheckoutBuilder::new();
        checkout.path(path).force().remove_untracked(true);

        let head = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
        let head_object = head.as_ref().map(|commit| commit.as_object());
        repo.reset_default(head_object, [Path::new(path)])?;

        let mut index = repo.index()?;
        repo.checkout_index(Some(&mut index), Some(&mut checkout))?;
        Ok(())
    }

    pub fn apply_patch(&self, patch_text: &str, location: ApplyLocation) -> Result<()> {
        let repo = self.repo()?;
        let diff = Diff::from_buffer(patch_text.as_bytes())?;
        repo.apply(&diff, location, None)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::CredentialType;
    use git2::{Repository, Signature, Status, StatusOptions};
    use tempfile::TempDir;

    use super::{
        INDEX_REF, PR_REF_PREFIX, RemoteCredentialKind, WORKDIR_REF, pr_ref_path,
        select_remote_credential,
    };
    use crate::core::vcs::git::{GitService, StatusItem, StatusOperation, StatusScope};

    fn commit_file(repo: &Repository, relative_path: &str, content: &str, message: &str) -> String {
        let workdir = repo.workdir().expect("repo workdir");
        let full_path = workdir.join(relative_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full_path, content).unwrap();

        let mut index = repo.index().unwrap();
        index.add_path(Path::new(relative_path)).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = Signature::now("Diffy", "diffy@example.com").unwrap();
        let parents = repo
            .head()
            .ok()
            .and_then(|head| head.target())
            .map(|oid| repo.find_commit(oid).unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )
        .unwrap()
        .to_string()
    }

    fn statuses(repo: &Repository) -> Vec<(String, Status)> {
        let mut options = StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);
        repo.statuses(Some(&mut options))
            .unwrap()
            .iter()
            .map(|entry| (entry.path().unwrap_or_default().to_owned(), entry.status()))
            .collect()
    }

    #[test]
    fn pr_ref_path_embeds_branch_with_slash() {
        assert_eq!(pr_ref_path(12, "main"), "refs/diffy/pr/12/main");
        assert_eq!(
            pr_ref_path(77, "feat/new-thing"),
            "refs/diffy/pr/77/feat/new-thing"
        );
        assert!(pr_ref_path(1, "x").starts_with(PR_REF_PREFIX));
    }

    #[test]
    fn prefers_https_token_for_github_remotes() {
        let allowed = CredentialType::USER_PASS_PLAINTEXT | CredentialType::USERNAME;
        let selected = select_remote_credential(
            "https://github.com/owner/repo.git",
            Some("git"),
            allowed,
            "secret",
        );
        assert_eq!(
            selected,
            RemoteCredentialKind::UserPassPlaintext {
                username: "git".to_owned(),
                password: "secret".to_owned(),
            }
        );
    }

    #[test]
    fn falls_back_to_ssh_for_non_http_remotes() {
        let selected = select_remote_credential(
            "git@github.com:owner/repo.git",
            Some("git"),
            CredentialType::SSH_KEY,
            "secret",
        );
        assert_eq!(
            selected,
            RemoteCredentialKind::SshKey {
                username: "git".to_owned(),
            }
        );
    }

    #[test]
    fn can_stage_unstage_and_discard_status_items() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");

        fs::write(repo_dir.path().join("src/lib.rs"), "new\n").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        git.apply_status_operation(
            &StatusItem {
                path: "src/lib.rs".to_owned(),
                scope: StatusScope::Unstaged,
                status: "M".to_owned(),
            },
            StatusOperation::Stage,
        )
        .unwrap();

        let staged = statuses(&repo);
        assert!(staged[0].1.contains(Status::INDEX_MODIFIED));

        git.apply_status_operation(
            &StatusItem {
                path: "src/lib.rs".to_owned(),
                scope: StatusScope::Staged,
                status: "M".to_owned(),
            },
            StatusOperation::Unstage,
        )
        .unwrap();

        let unstaged = statuses(&repo);
        assert!(unstaged[0].1.contains(Status::WT_MODIFIED));

        git.apply_status_operation(
            &StatusItem {
                path: "src/lib.rs".to_owned(),
                scope: StatusScope::Unstaged,
                status: "M".to_owned(),
            },
            StatusOperation::Discard,
        )
        .unwrap();

        assert!(statuses(&repo).is_empty());
        assert_eq!(
            fs::read_to_string(repo_dir.path().join("src/lib.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn can_discard_untracked_file() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        fs::write(repo_dir.path().join("src/new.rs"), "hello\n").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        git.apply_status_operation(
            &StatusItem {
                path: "src/new.rs".to_owned(),
                scope: StatusScope::Untracked,
                status: "U".to_owned(),
            },
            StatusOperation::Discard,
        )
        .unwrap();

        assert!(!repo_dir.path().join("src/new.rs").exists());
    }

    #[test]
    fn can_stage_hunk_via_patch() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "line1\nline2\nline3\n", "initial");
        fs::write(
            repo_dir.path().join("src/lib.rs"),
            "line1\nchanged\nline3\nextra\n",
        )
        .unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        let patch = concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,3 +1,4 @@\n",
            " line1\n",
            "-line2\n",
            "+changed\n",
            " line3\n",
            "+extra\n",
        );
        git.apply_patch(patch, git2::ApplyLocation::Index).unwrap();

        let st = statuses(&repo);
        assert!(st[0].1.contains(Status::INDEX_MODIFIED));
    }

    #[test]
    fn read_file_lines_at_reads_commit_blob() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let head = commit_file(&repo, "src/lib.rs", "one\ntwo\nthree\n", "initial");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let lines = git.read_file_lines_at(&head, "src/lib.rs").unwrap();
        assert_eq!(lines, vec!["one", "two", "three"]);
    }

    #[test]
    fn read_file_lines_at_reads_workdir() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        fs::write(repo_dir.path().join("src/lib.rs"), "a\nb\nc").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let lines = git.read_file_lines_at(WORKDIR_REF, "src/lib.rs").unwrap();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn read_file_lines_at_reads_index() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "a\nb\n", "initial");
        fs::write(repo_dir.path().join("src/lib.rs"), "a\nb\nc\n").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        git.stage_path("src/lib.rs").unwrap();

        let lines = git.read_file_lines_at(INDEX_REF, "src/lib.rs").unwrap();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn read_file_lines_at_handles_crlf_and_missing_trailing_newline() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        fs::write(repo_dir.path().join("src/lib.rs"), "a\r\nb\r\nc").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let lines = git.read_file_lines_at(WORKDIR_REF, "src/lib.rs").unwrap();
        assert_eq!(lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn read_file_lines_at_rejects_binary_file() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "old\n", "initial");
        fs::write(repo_dir.path().join("img.bin"), [0u8, 1, 2, 0, 3]).unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let err = git
            .read_file_lines_at(WORKDIR_REF, "img.bin")
            .expect_err("binary file should fail");
        assert!(err.to_string().contains("binary"));
    }

    #[test]
    fn read_file_lines_at_returns_error_when_path_missing_at_ref() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let head = commit_file(&repo, "src/lib.rs", "x\n", "initial");

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();
        let err = git
            .read_file_lines_at(&head, "src/nope.rs")
            .expect_err("missing path should fail");
        assert!(err.to_string().contains("not present"));
    }

    #[test]
    fn can_unstage_hunk_via_reverse_patch() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", "aaa\nbbb\n", "initial");
        fs::write(repo_dir.path().join("src/lib.rs"), "aaa\nccc\n").unwrap();

        let mut git = GitService::new();
        git.open(repo_dir.path().to_str().unwrap()).unwrap();

        git.stage_path("src/lib.rs").unwrap();
        let st = statuses(&repo);
        assert!(st[0].1.contains(Status::INDEX_MODIFIED));

        let reverse_patch = concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,2 +1,2 @@\n",
            " aaa\n",
            "-ccc\n",
            "+bbb\n",
        );
        git.apply_patch(reverse_patch, git2::ApplyLocation::Index)
            .unwrap();

        let st2 = statuses(&repo);
        assert!(st2[0].1.contains(Status::WT_MODIFIED));
        assert!(!st2[0].1.contains(Status::INDEX_MODIFIED));
    }
}
