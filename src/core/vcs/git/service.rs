use std::cmp::Ordering;

use git2::{
    BranchType, Cred, DiffFormat, DiffOptions, FetchOptions, ObjectType, Oid, RemoteCallbacks,
    Repository,
};
use serde::Serialize;

use crate::core::compare::spec::CompareMode;
use crate::core::error::{DiffyError, Result};
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
                        "single-commit mode requires a commit reference".to_owned(),
                    ));
                }
                let right_oid = self.resolve_commit_oid(commit_ref)?;
                let commit = repo.find_commit(right_oid)?;
                let parent = commit.parent(0).map_err(|_| {
                    DiffyError::Parse(
                        "cannot diff the root commit in single-commit mode yet".to_owned(),
                    )
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

    pub fn fetch_and_diff_pr(
        &self,
        repo_url: &str,
        pr_number: i32,
        base_ref: &str,
        head_ref: &str,
    ) -> Result<String> {
        let repo = self.repo()?;
        let base_target = format!("refs/diffy/pull/{pr_number}/base");
        let head_target = format!("refs/diffy/pull/{pr_number}/head");
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
        remote.fetch(
            &[
                &format!("+{base_ref}:{base_target}"),
                &format!("+{head_ref}:{head_target}"),
            ],
            Some(&mut fetch_options),
            None,
        )?;
        self.diff_between_refs(&base_target, &head_target)
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
        let base_ref = if info.base_sha.is_empty() {
            format!("refs/heads/{}", info.base_branch)
        } else {
            info.base_sha.clone()
        };
        let head_ref = if info.head_sha.is_empty() {
            format!("refs/heads/{}", info.head_branch)
        } else {
            info.head_sha.clone()
        };
        let _ = self.fetch_and_diff_pr(&repo_url, parsed.number, &base_ref, &head_ref)?;
        let left = self.resolve_ref(&format!("refs/diffy/pull/{}/base", parsed.number))?;
        let right = self.resolve_ref(&format!("refs/diffy/pull/{}/head", parsed.number))?;
        Ok((left, right))
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
}

#[cfg(test)]
mod tests {
    use git2::CredentialType;

    use super::{RemoteCredentialKind, select_remote_credential};

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
}
