use std::cmp::Ordering;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::core::compare::backends::compare_output_from_raw_patch;
use crate::core::compare::service::CompareOutput;
use crate::core::compare::spec::CompareMode;
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::status::{StatusBits, StatusItem, StatusOperation, StatusScope};
use crate::core::vcs::github::{GitHubApi, parse_pr_url};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullError {
    NoUpstream,
    NonFastForward { ahead: usize, behind: usize },
    DirtyWorkdir,
    Other(String),
}

impl std::fmt::Display for PullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullError::NoUpstream => f.write_str("branch has no upstream configured"),
            PullError::NonFastForward { ahead, behind } => write!(
                f,
                "branch has diverged from upstream ({ahead} ahead, {behind} behind); merge/rebase not yet supported",
            ),
            PullError::DirtyWorkdir => {
                f.write_str("working tree has uncommitted changes; commit or stash first")
            }
            PullError::Other(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PullError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullOutcome {
    AlreadyUpToDate,
    FastForwarded { behind: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchApplyTarget {
    Index,
    Workdir,
}

fn workdir_is_dirty(repo_path: &Path) -> Result<bool> {
    let output = run_system_git_capture(
        repo_path,
        &[
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("--untracked-files=no"),
        ],
    )?;
    Ok(!output.stdout.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_head: bool,
    /// Shorthand name of the upstream tracking branch (e.g. `origin/main`) for
    /// local branches that have one configured. `None` for remote branches and
    /// for local branches without an upstream.
    pub upstream: Option<String>,
    /// `(ahead, behind)` commits for local branches relative to their upstream.
    /// `None` when no upstream is configured or the counts could not be computed.
    pub ahead_behind: Option<(usize, usize)>,
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
/// scheme we used to use. Uses a prefix filter so branch names with slashes are
/// handled exactly instead of relying on glob semantics.
fn prune_stale_pr_refs(repo_path: &Path, pr_number: i32, keep_base: &str, keep_head: &str) {
    let prefixes = [
        format!("{PR_REF_PREFIX}{pr_number}/"),
        format!("refs/diffy/pull/{pr_number}/"),
    ];
    let Ok(output) = run_system_git_capture(
        repo_path,
        &[
            OsString::from("for-each-ref"),
            OsString::from("--format=%(refname)"),
        ],
    ) else {
        return;
    };
    let stale: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .filter(|name| {
            name != keep_base && name != keep_head && prefixes.iter().any(|p| name.starts_with(p))
        })
        .collect();
    for name in stale {
        let _ = run_system_git(
            repo_path,
            &[
                OsString::from("update-ref"),
                OsString::from("-d"),
                name.into(),
            ],
        );
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

fn git_workdir(repo: &gix::Repository) -> Result<PathBuf> {
    repo.workdir()
        .map(Path::to_path_buf)
        .or_else(|| repo.git_dir().parent().map(Path::to_path_buf))
        .ok_or_else(|| DiffyError::General("repository has no working directory".to_owned()))
}

struct GitOutput {
    stdout: Vec<u8>,
}

fn run_system_git(repo_path: &Path, args: &[OsString]) -> Result<()> {
    run_system_git_inner(repo_path, args, false).map(|_| ())
}

fn run_system_git_allow_diff(repo_path: &Path, args: &[OsString]) -> Result<GitOutput> {
    run_system_git_inner(repo_path, args, true)
}

fn run_system_git_capture(repo_path: &Path, args: &[OsString]) -> Result<GitOutput> {
    run_system_git_inner(repo_path, args, false)
}

fn run_system_git_inner(
    repo_path: &Path,
    args: &[OsString],
    allow_diff_exit: bool,
) -> Result<GitOutput> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| DiffyError::General(format!("failed to run git: {e}")))?;

    if output.status.success() || (allow_diff_exit && output.status.code() == Some(1)) {
        return Ok(GitOutput {
            stdout: output.stdout,
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = stderr
        .trim()
        .lines()
        .last()
        .or_else(|| stdout.trim().lines().last())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("git exited with {}", output.status));
    let command = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    Err(DiffyError::General(format!(
        "git {command} failed: {detail}"
    )))
}

fn gix_error(error: impl std::fmt::Display) -> DiffyError {
    DiffyError::General(format!("Gitoxide error: {error}"))
}

fn github_repo_key_from_remote_url(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim();
    let path = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://github.com/") {
        rest
    } else {
        let (_, rest) = trimmed.split_once("://")?;
        let (authority, path) = rest.split_once('/')?;
        if !github_authority_matches(authority) {
            return None;
        }
        path
    };

    let path = path.split(['?', '#']).next().unwrap_or(path);
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_ascii_lowercase(), repo.to_ascii_lowercase()))
}

fn github_authority_matches(authority: &str) -> bool {
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    host.eq_ignore_ascii_case("github.com")
}

fn local_remote_names_by_priority(repo_path: &Path) -> Option<Vec<String>> {
    let output = run_system_git_capture(repo_path, &[OsString::from("remote")]).ok()?;
    let mut candidates = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|name| match name.as_str() {
        "origin" => 0,
        "upstream" => 1,
        _ => 2,
    });
    Some(candidates)
}

fn remote_url(repo_path: &Path, name: &str) -> Option<String> {
    let output = run_system_git_capture(
        repo_path,
        &[
            OsString::from("remote"),
            OsString::from("get-url"),
            OsString::from(name),
        ],
    )
    .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn local_remote_for_github_repo(repo_path: &Path, owner: &str, repo_name: &str) -> Option<String> {
    let target = (owner.to_ascii_lowercase(), repo_name.to_ascii_lowercase());
    let candidates = local_remote_names_by_priority(repo_path)?;
    for name in candidates {
        if let Some(url) = remote_url(repo_path, &name)
            && github_repo_key_from_remote_url(&url).as_ref() == Some(&target)
        {
            return Some(name);
        }
    }
    None
}

fn github_repo_url_from_remote_transport(url: &str, owner: &str, repo: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.strip_prefix("git@github.com:").is_some() {
        return Some(format!("git@github.com:{owner}/{repo}.git"));
    }

    let (scheme, rest) = trimmed.split_once("://")?;
    let (authority, _) = rest.split_once('/')?;
    if !github_authority_matches(authority) {
        return None;
    }

    let scheme = scheme.to_ascii_lowercase();
    if scheme == "ssh" {
        Some(format!("ssh://{authority}/{owner}/{repo}.git"))
    } else {
        Some(format!("{scheme}://github.com/{owner}/{repo}.git"))
    }
}

fn local_github_url_for_repo(repo_path: &Path, owner: &str, repo_name: &str) -> Option<String> {
    let candidates = local_remote_names_by_priority(repo_path)?;
    for name in candidates {
        if let Some(url) = remote_url(repo_path, &name)
            && let Some(fetch_url) = github_repo_url_from_remote_transport(&url, owner, repo_name)
        {
            return Some(fetch_url);
        }
    }
    None
}

fn fallback_pull_request_repo_url(owner: &str, repo: &str, api_clone_url: &str) -> String {
    if api_clone_url.is_empty() {
        format!("https://github.com/{owner}/{repo}.git")
    } else {
        api_clone_url.to_owned()
    }
}

fn github_fetch_source_for_repo(
    repo_path: &Path,
    owner: &str,
    repo_name: &str,
    api_clone_url: &str,
) -> String {
    local_remote_for_github_repo(repo_path, owner, repo_name)
        .or_else(|| local_github_url_for_repo(repo_path, owner, repo_name))
        .unwrap_or_else(|| fallback_pull_request_repo_url(owner, repo_name, api_clone_url))
}

#[derive(Default)]
pub struct GitService {
    repo: Option<gix::Repository>,
    repo_path: String,
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

    pub fn new_with_open(path: &Path) -> Result<Self> {
        let mut git = Self::default();
        git.open(path.to_string_lossy().as_ref())?;
        Ok(git)
    }

    pub fn open(&mut self, path: &str) -> Result<()> {
        self.close();
        let mut repo = gix::open(path).map_err(gix_error)?;
        repo.object_cache_size_if_unset(64 * 1024 * 1024);
        let repo_path = git_workdir(&repo).unwrap_or_else(|_| PathBuf::from(path));
        self.repo = Some(repo);
        self.repo_path = repo_path.to_string_lossy().into_owned();
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
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("for-each-ref"),
                OsString::from("--format=%(refname:short)"),
            ],
        )?;
        let mut refs = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        refs.sort();
        refs.dedup();
        Ok(refs)
    }

    pub fn branches(&self) -> Result<Vec<BranchInfo>> {
        let repo_path = self.repo_path_ref()?;
        let head = self.head_branch_name()?.unwrap_or_default();
        let output = run_system_git_capture(
            repo_path,
            &[
                OsString::from("for-each-ref"),
                OsString::from(
                    "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(upstream:short)",
                ),
                OsString::from("refs/heads"),
                OsString::from("refs/remotes"),
            ],
        )?;
        let mut branches = Vec::new();
        for line in output.stdout.split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let fields = line.split(|byte| *byte == 0).collect::<Vec<_>>();
            if fields.len() < 4 {
                continue;
            }
            let full_name = String::from_utf8_lossy(fields[0]);
            let name = String::from_utf8_lossy(fields[1]).to_string();
            if name.ends_with("/HEAD") {
                continue;
            }
            let is_remote = full_name.starts_with("refs/remotes/");
            let upstream =
                (!fields[3].is_empty()).then(|| String::from_utf8_lossy(fields[3]).to_string());
            let ahead_behind = if is_remote {
                None
            } else if let Some(upstream) = upstream.as_ref() {
                graph_ahead_behind(repo_path, &name, upstream).ok()
            } else {
                None
            };
            let is_head = !is_remote && name == head;
            branches.push(BranchInfo {
                name,
                is_remote,
                is_head,
                upstream,
                ahead_behind,
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
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("for-each-ref"),
                OsString::from("--format=%(refname:short)%00%(*objectname)%00%(objectname)"),
                OsString::from("refs/tags"),
            ],
        )?;
        let mut tags = output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                if line.is_empty() {
                    return None;
                }
                let fields = line.split(|byte| *byte == 0).collect::<Vec<_>>();
                if fields.len() < 3 {
                    return None;
                }
                let peeled = if fields[1].is_empty() {
                    fields[2]
                } else {
                    fields[1]
                };
                Some(TagInfo {
                    name: String::from_utf8_lossy(fields[0]).to_string(),
                    target_oid: String::from_utf8_lossy(peeled).to_string(),
                })
            })
            .collect::<Vec<_>>();
        tags.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tags)
    }

    pub fn commits(&self, reference: &str, max_count: usize) -> Result<Vec<CommitInfo>> {
        self.git_log_commits(&[
            OsString::from(format!("-n{max_count}")),
            OsString::from(reference),
        ])
    }

    pub fn commits_in_range(
        &self,
        left: &str,
        right: &str,
        max_count: usize,
    ) -> Result<Vec<CommitInfo>> {
        self.git_log_commits(&[
            OsString::from(format!("-n{max_count}")),
            OsString::from(format!("{left}..{right}")),
        ])
    }

    pub fn search_commits(&self, hex_prefix: &str) -> Result<Vec<CommitInfo>> {
        if hex_prefix.len() < 4 {
            return Ok(Vec::new());
        }
        let prefix = hex_prefix.to_ascii_lowercase();
        let mut commits =
            self.git_log_commits(&[OsString::from("-n500"), OsString::from("HEAD")])?;
        commits.retain(|commit| commit.oid.starts_with(&prefix));
        commits.truncate(50);
        Ok(commits)
    }

    pub fn resolve_ref(&self, reference: &str) -> Result<String> {
        self.resolve_commit_oid(reference)
    }

    pub(crate) fn read_file_bytes_at(&self, reference: &str, path: &str) -> Result<Vec<u8>> {
        let bytes = if reference == WORKDIR_REF {
            let full = self.repo_path_ref()?.join(path);
            std::fs::read(&full)?
        } else {
            let spec = if reference == INDEX_REF {
                format!(":{path}")
            } else {
                format!("{}:{path}", self.resolve_commit_oid(reference)?)
            };
            run_system_git_capture(
                self.repo_path_ref()?,
                &[OsString::from("show"), OsString::from(spec)],
            )
            .map_err(|_| DiffyError::General(format!("path {path} is not present at {reference}")))?
            .stdout
        };
        Ok(bytes)
    }

    fn validate_text_bytes(reference: &str, path: &str, bytes: &[u8]) -> Result<()> {
        if bytes.contains(&0u8) {
            return Err(DiffyError::General(format!(
                "path {path} is binary at {reference}",
            )));
        }

        std::str::from_utf8(bytes).map_err(|e| {
            DiffyError::General(format!(
                "path {path} at {reference} is not valid UTF-8: {e}"
            ))
        })?;
        Ok(())
    }

    pub fn read_file_text_store_at(
        &self,
        reference: &str,
        path: &str,
    ) -> Result<carbon::TextStore> {
        let bytes = self.read_file_bytes_at(reference, path)?;
        Self::validate_text_bytes(reference, path, &bytes)?;
        Ok(carbon::TextStore::from_bytes(bytes))
    }

    pub fn read_file_lines_at(&self, reference: &str, path: &str) -> Result<Vec<String>> {
        let bytes = self.read_file_bytes_at(reference, path)?;
        Self::validate_text_bytes(reference, path, &bytes)?;
        let text = std::str::from_utf8(&bytes).unwrap_or_default();

        Ok(split_lines(text))
    }

    /// Unified-diff patch text against HEAD suitable for feeding to an LLM
    /// (staged index when `has_staged` is true, else the worktree).
    pub fn diff_for_commit(&self, has_staged: bool) -> Result<String> {
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
        ];
        if has_staged {
            args.push(OsString::from("--cached"));
        }
        let output = run_system_git_allow_diff(self.repo_path_ref()?, &args)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn diff_status_item(&self, item: &StatusItem) -> Result<CompareOutput> {
        let patch = self.status_item_patch(item)?;
        compare_output_from_raw_patch(&patch)
    }

    pub fn status_item_patch(&self, item: &StatusItem) -> Result<String> {
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
        ];
        match item.scope {
            StatusScope::Staged => args.push(OsString::from("--cached")),
            StatusScope::Unstaged => {}
            StatusScope::Untracked => {
                args.push(OsString::from("--no-index"));
                args.push(OsString::from("--"));
                args.push(OsString::from("/dev/null"));
                args.push(OsString::from(self.repo_path_ref()?.join(&item.path)));
                let output = run_system_git_allow_diff(self.repo_path_ref()?, &args)?;
                return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
            }
        }
        args.push(OsString::from("--"));
        args.push(OsString::from(&item.path));
        let output = run_system_git_allow_diff(self.repo_path_ref()?, &args)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
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
        for item in items {
            self.apply_status_operation(item, operation)?;
        }
        Ok(())
    }

    pub fn abbreviate_oid(&self, full_oid: &str) -> Result<String> {
        Ok(fixed_short_oid(full_oid).to_owned())
    }

    pub fn resolve_oid_to_branch_name(&self, oid_hex: &str) -> Result<String> {
        if oid_hex.len() != 40 {
            return Ok(String::new());
        }
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("for-each-ref"),
                OsString::from("--format=%(refname:short)%00%(objectname)"),
                OsString::from("refs/heads"),
            ],
        )?;
        for line in output.stdout.split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            let fields = line.split(|byte| *byte == 0).collect::<Vec<_>>();
            if fields.len() < 2 {
                continue;
            }
            if fields[1] == oid_hex.as_bytes() {
                return Ok(String::from_utf8_lossy(fields[0]).to_string());
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
                let parent = first_parent(self.repo()?, &right_oid).ok_or_else(|| {
                    DiffyError::Parse("cannot diff the root commit in commit mode yet".to_owned())
                })?;
                Ok((parent, right_oid))
            }
            CompareMode::TwoDot | CompareMode::ThreeDot => {
                if left_ref.is_empty() || right_ref.is_empty() {
                    return Err(DiffyError::Parse(
                        "comparison requires both left and right references".to_owned(),
                    ));
                }
                if right_ref == WORKDIR_REF {
                    let left_oid = self.resolve_commit_oid(left_ref)?;
                    return Ok((left_oid, WORKDIR_REF.to_owned()));
                }
                let mut left_oid = self.resolve_commit_oid(left_ref)?;
                let right_oid = self.resolve_commit_oid(right_ref)?;
                if mode == CompareMode::ThreeDot {
                    left_oid = merge_base(self.repo()?, &left_oid, &right_oid)?;
                }
                Ok((left_oid, right_oid))
            }
        }
    }

    pub fn diff_two_refs(&self, left: &str, right: &str) -> Result<String> {
        self.diff_between_refs(left, right)
    }

    #[cfg_attr(not(feature = "difftastic"), allow(dead_code))]
    pub(crate) fn diff_name_status(
        &self,
        left: &str,
        right: &str,
        only_path: Option<&str>,
    ) -> Result<Vec<(String, Option<String>, Option<String>)>> {
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--name-status"),
            OsString::from("-z"),
            OsString::from("--find-renames"),
            OsString::from(self.resolve_commit_oid(left)?),
        ];
        if right != WORKDIR_REF {
            args.push(OsString::from(self.resolve_commit_oid(right)?));
        }
        if let Some(path) = only_path {
            args.push(OsString::from("--"));
            args.push(OsString::from(path));
        }
        let output = run_system_git_allow_diff(self.repo_path_ref()?, &args)?;
        Ok(parse_name_status(&output.stdout))
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
        let mut args = Vec::with_capacity(refspecs.len() + 2);
        args.push(OsString::from("fetch"));
        args.push(OsString::from(repo_url));
        args.extend(refspecs.iter().map(OsString::from));
        run_system_git(self.repo_path_ref()?, &args)
    }

    /// Fetch a named remote using the remote's configured refspecs.
    pub fn fetch_remote<F>(&self, remote_name: &str, _progress: F) -> Result<()>
    where
        F: FnMut(usize, usize, usize) + Send + 'static,
    {
        run_system_git(
            self.repo_path_ref()?,
            &[OsString::from("fetch"), OsString::from(remote_name)],
        )
    }

    /// List the names of configured remotes (e.g. `["origin"]`).
    pub fn remote_names(&self) -> Result<Vec<String>> {
        let output = run_system_git_capture(self.repo_path_ref()?, &[OsString::from("remote")])?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect())
    }

    /// Name of the current HEAD branch (no `refs/heads/` prefix), if HEAD is on
    /// a branch (not detached).
    pub fn head_branch_name(&self) -> Result<Option<String>> {
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("symbolic-ref"),
                OsString::from("--quiet"),
                OsString::from("--short"),
                OsString::from("HEAD"),
            ],
        );
        Ok(output
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|name| !name.is_empty()))
    }

    /// Returns `(upstream_remote, upstream_branch)` for a local branch if
    /// configured. e.g. `("origin", "main")`.
    pub fn upstream_for(&self, local_branch: &str) -> Result<Option<(String, String)>> {
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("rev-parse"),
                OsString::from("--abbrev-ref"),
                OsString::from(format!("{local_branch}@{{upstream}}")),
            ],
        );
        let Ok(output) = output else {
            return Ok(None);
        };
        let upstream_name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        // Upstream shorthand is `<remote>/<branch>`. Split on first `/`.
        let Some((remote, branch)) = upstream_name.split_once('/') else {
            return Ok(None);
        };
        Ok(Some((remote.to_owned(), branch.to_owned())))
    }

    /// Push a refspec to a remote using system Git so SSH and credential
    /// handling match the user's CLI setup.
    pub fn push<F>(
        &self,
        remote_name: &str,
        refspec: &str,
        force_with_lease: bool,
        _progress: F,
    ) -> Result<()>
    where
        F: FnMut(usize, usize, usize) + Send + 'static,
    {
        let mut args = Vec::with_capacity(4);
        args.push(OsString::from("push"));
        if force_with_lease {
            args.push(OsString::from("--force-with-lease"));
        }
        args.push(OsString::from(remote_name));
        args.push(OsString::from(refspec));
        run_system_git(self.repo_path_ref()?, &args)
    }

    /// Fast-forward the named local branch to match its upstream on the given
    /// remote. Fetches first, then updates the ref and checks out the new tip
    /// when HEAD is on that branch.
    ///
    /// Errors:
    /// - `PullError::NoUpstream` — branch has no upstream configured.
    /// - `PullError::NonFastForward` — local has diverged from upstream.
    /// - `PullError::DirtyWorkdir` — uncommitted changes block fast-forward.
    /// - `PullError::AlreadyUpToDate` — no-op signal (not strictly an error).
    pub fn pull_ff<F>(
        &self,
        remote_name: &str,
        local_branch: &str,
        progress: F,
    ) -> std::result::Result<PullOutcome, PullError>
    where
        F: FnMut(usize, usize, usize) + Send + 'static,
    {
        self.fetch_remote(remote_name, progress)
            .map_err(|e| PullError::Other(e.to_string()))?;
        let repo_path = self
            .repo_path_ref()
            .map_err(|e| PullError::Other(e.to_string()))?;
        let upstream_shorthand = format!("{remote_name}/{local_branch}");
        let upstream_oid = resolve_commit_oid_at(repo_path, &upstream_shorthand)
            .map_err(|_| PullError::NoUpstream)?;
        let local_oid = resolve_commit_oid_at(repo_path, local_branch)
            .map_err(|e| PullError::Other(e.to_string()))?;

        if local_oid == upstream_oid {
            return Ok(PullOutcome::AlreadyUpToDate);
        }

        let (ahead, behind) = graph_ahead_behind(repo_path, local_branch, &upstream_shorthand)
            .map_err(|e| PullError::Other(e.to_string()))?;
        if ahead > 0 {
            return Err(PullError::NonFastForward { ahead, behind });
        }

        let head_is_this_branch = self
            .head_branch_name()
            .map_err(|e| PullError::Other(e.to_string()))?
            .as_deref()
            == Some(local_branch);

        if head_is_this_branch {
            let dirty = workdir_is_dirty(repo_path).map_err(|e| PullError::Other(e.to_string()))?;
            if dirty {
                return Err(PullError::DirtyWorkdir);
            }
            run_system_git(
                repo_path,
                &[
                    OsString::from("merge"),
                    OsString::from("--ff-only"),
                    OsString::from(&upstream_shorthand),
                ],
            )
            .map_err(|e| PullError::Other(e.to_string()))?;
        } else {
            run_system_git(
                repo_path,
                &[
                    OsString::from("branch"),
                    OsString::from("-f"),
                    OsString::from(local_branch),
                    OsString::from(&upstream_shorthand),
                ],
            )
            .map_err(|e| PullError::Other(e.to_string()))?;
        }

        Ok(PullOutcome::FastForwarded { behind })
    }

    pub fn resolve_pull_request_comparison(
        &self,
        pull_request_url: &str,
        github_token: &str,
    ) -> Result<(String, String)> {
        let parsed = parse_pr_url(pull_request_url)
            .ok_or_else(|| DiffyError::Parse("not a valid GitHub pull request URL".to_owned()))?;
        let api = GitHubApi::with_token(github_token.to_owned());
        let info = api.fetch_pull_request(&parsed.owner, &parsed.repo, parsed.number)?;
        let fetch_source = github_fetch_source_for_repo(
            self.repo_path_ref()?,
            &parsed.owner,
            &parsed.repo,
            &info.base_repo_url,
        );
        let base_source = if info.base_sha.is_empty() {
            format!("refs/heads/{}", info.base_branch)
        } else {
            info.base_sha.clone()
        };
        let head_source = format!("refs/pull/{}/head", parsed.number);
        let base_target = pr_ref_path(parsed.number, &info.base_branch);
        let head_target = pr_ref_path(parsed.number, &info.head_branch);
        self.fetch_refspecs(
            &fetch_source,
            &[
                format!("+{base_source}:{base_target}"),
                format!("+{head_source}:{head_target}"),
            ],
        )?;
        prune_stale_pr_refs(
            self.repo_path_ref()?,
            parsed.number,
            &base_target,
            &head_target,
        );
        Ok((base_target, head_target))
    }

    fn diff_between_refs(&self, left: &str, right: &str) -> Result<String> {
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--find-renames"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
            OsString::from(self.resolve_commit_oid(left)?),
        ];
        if right != WORKDIR_REF {
            args.push(OsString::from(self.resolve_commit_oid(right)?));
        }
        let output = run_system_git_allow_diff(self.repo_path_ref()?, &args)?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn resolve_commit_oid(&self, reference: &str) -> Result<String> {
        let id = self
            .repo()?
            .rev_parse_single(reference)
            .map_err(gix_error)?
            .object()
            .map_err(gix_error)?
            .peel_to_commit()
            .map_err(gix_error)?
            .id
            .to_string();
        Ok(id)
    }

    pub fn repo(&self) -> Result<&gix::Repository> {
        self.repo
            .as_ref()
            .ok_or_else(|| DiffyError::General("repository is not open".to_owned()))
    }

    fn repo_path_ref(&self) -> Result<&Path> {
        if self.repo_path.is_empty() {
            return Err(DiffyError::General("repository is not open".to_owned()));
        }
        Ok(Path::new(&self.repo_path))
    }

    pub fn commit(&self, message: &str) -> Result<String> {
        run_system_git(
            self.repo_path_ref()?,
            &[
                OsString::from("commit"),
                OsString::from("-m"),
                OsString::from(message),
            ],
        )?;
        self.resolve_ref("HEAD")
    }

    fn stage_path(&self, path: &str) -> Result<()> {
        run_system_git(
            self.repo_path_ref()?,
            &[
                OsString::from("add"),
                OsString::from("--"),
                OsString::from(path),
            ],
        )
    }

    fn unstage_path(&self, path: &str) -> Result<()> {
        run_system_git(
            self.repo_path_ref()?,
            &[
                OsString::from("reset"),
                OsString::from("--"),
                OsString::from(path),
            ],
        )
    }

    fn discard_path(&self, path: &str) -> Result<()> {
        let absolute = self.repo_path_ref()?.join(path);
        if !is_path_tracked(self.repo_path_ref()?, path)? {
            match std::fs::remove_file(&absolute) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        run_system_git(
            self.repo_path_ref()?,
            &[
                OsString::from("restore"),
                OsString::from("--staged"),
                OsString::from("--worktree"),
                OsString::from("--"),
                OsString::from(path),
            ],
        )
    }

    pub fn apply_patch(&self, patch_text: &str, target: PatchApplyTarget) -> Result<()> {
        let mut child = Command::new("git")
            .args(match target {
                PatchApplyTarget::Index => ["apply", "--cached"].as_slice(),
                PatchApplyTarget::Workdir => ["apply"].as_slice(),
            })
            .current_dir(self.repo_path_ref()?)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| DiffyError::General(format!("failed to run git apply: {e}")))?;
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| DiffyError::General("failed to open git apply stdin".to_owned()))?
            .write_all(patch_text.as_bytes())?;
        let output = child.wait_with_output()?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(DiffyError::General(format!(
                "git apply failed: {}",
                stderr.trim()
            )))
        }
    }

    pub fn status_entries(&self) -> Result<Vec<(String, StatusBits)>> {
        let output = run_system_git_capture(
            self.repo_path_ref()?,
            &[
                OsString::from("status"),
                OsString::from("--porcelain=v1"),
                OsString::from("-z"),
                OsString::from("--untracked-files=all"),
            ],
        )?;
        Ok(parse_porcelain_status(&output.stdout))
    }

    fn git_log_commits(&self, rev_args: &[OsString]) -> Result<Vec<CommitInfo>> {
        let mut args = vec![
            OsString::from("log"),
            OsString::from("--date-order"),
            OsString::from("--format=%H%x00%h%x00%s%x00%an%x00%ct"),
        ];
        args.extend_from_slice(rev_args);
        let output = run_system_git_capture(self.repo_path_ref()?, &args)?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_commit_info_line)
            .collect())
    }
}

fn parse_commit_info_line(line: &str) -> Option<CommitInfo> {
    let mut fields = line.split('\0');
    let oid = fields.next()?.to_owned();
    let short_oid = fields
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| fixed_short_oid(&oid).to_owned());
    let summary = fields.next().unwrap_or_default().to_owned();
    let author_name = fields.next().unwrap_or_default().to_owned();
    let timestamp = fields
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    Some(CommitInfo {
        oid,
        short_oid,
        summary,
        author_name,
        timestamp,
    })
}

fn parse_porcelain_status(bytes: &[u8]) -> Vec<(String, StatusBits)> {
    let mut out = Vec::new();
    let mut fields = bytes.split(|byte| *byte == 0);
    while let Some(entry) = fields.next() {
        if entry.is_empty() || entry.len() < 4 {
            continue;
        }
        let x = entry[0] as char;
        let y = entry[1] as char;
        let path = String::from_utf8_lossy(&entry[3..]).to_string();
        if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
            let _old_path = fields.next();
        }
        out.push((path, status_bits_from_xy(x, y)));
    }
    out
}

#[cfg_attr(not(feature = "difftastic"), allow(dead_code))]
fn parse_name_status(bytes: &[u8]) -> Vec<(String, Option<String>, Option<String>)> {
    let mut out = Vec::new();
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    while let Some(status_bytes) = fields.next() {
        let status = String::from_utf8_lossy(status_bytes).to_string();
        let Some(first_path) = fields.next() else {
            break;
        };
        let first = String::from_utf8_lossy(first_path).to_string();
        if status.starts_with('R') || status.starts_with('C') {
            let Some(second_path) = fields.next() else {
                break;
            };
            out.push((
                "R".to_owned(),
                Some(first),
                Some(String::from_utf8_lossy(second_path).to_string()),
            ));
        } else {
            let label = status.chars().next().unwrap_or('M').to_string();
            let old_path = (label != "A").then(|| first.clone());
            let new_path = (label != "D").then_some(first);
            out.push((label, old_path, new_path));
        }
    }
    out
}

fn status_bits_from_xy(x: char, y: char) -> StatusBits {
    let mut bits = StatusBits::default();
    match x {
        'A' => bits |= StatusBits::INDEX_NEW,
        'M' => bits |= StatusBits::INDEX_MODIFIED,
        'D' => bits |= StatusBits::INDEX_DELETED,
        'R' | 'C' => bits |= StatusBits::INDEX_RENAMED,
        'T' => bits |= StatusBits::INDEX_TYPECHANGE,
        'U' => bits |= StatusBits::CONFLICTED,
        '?' => bits |= StatusBits::WT_NEW,
        _ => {}
    }
    match y {
        '?' => bits |= StatusBits::WT_NEW,
        'M' => bits |= StatusBits::WT_MODIFIED,
        'D' => bits |= StatusBits::WT_DELETED,
        'R' | 'C' => bits |= StatusBits::WT_RENAMED,
        'T' => bits |= StatusBits::WT_TYPECHANGE,
        'U' => bits |= StatusBits::CONFLICTED,
        _ => {}
    }
    bits
}

fn graph_ahead_behind(repo_path: &Path, left: &str, right: &str) -> Result<(usize, usize)> {
    let output = run_system_git_capture(
        repo_path,
        &[
            OsString::from("rev-list"),
            OsString::from("--left-right"),
            OsString::from("--count"),
            OsString::from(format!("{left}...{right}")),
        ],
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let ahead = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let behind = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok((ahead, behind))
}

fn resolve_commit_oid_at(repo_path: &Path, reference: &str) -> Result<String> {
    let output = run_system_git_capture(
        repo_path,
        &[
            OsString::from("rev-parse"),
            OsString::from(format!("{reference}^{{commit}}")),
        ],
    )?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn merge_base(repo: &gix::Repository, left: &str, right: &str) -> Result<String> {
    let left = gix::ObjectId::from_hex(left.as_bytes()).map_err(gix_error)?;
    let right = gix::ObjectId::from_hex(right.as_bytes()).map_err(gix_error)?;
    Ok(repo.merge_base(left, right).map_err(gix_error)?.to_string())
}

fn first_parent(repo: &gix::Repository, oid: &str) -> Option<String> {
    let oid = gix::ObjectId::from_hex(oid.as_bytes()).ok()?;
    let commit = repo.find_commit(oid).ok()?;
    commit.parent_ids().next().map(|id| id.to_string())
}

fn is_path_tracked(repo_path: &Path, path: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", path])
        .current_dir(repo_path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| DiffyError::General(format!("failed to run git: {e}")))?;
    Ok(output.status.success())
}

fn fixed_short_oid(oid: &str) -> &str {
    oid.get(..8).unwrap_or(oid)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{BranchType, Repository, Signature, Status, StatusOptions};
    use tempfile::TempDir;

    use super::{
        INDEX_REF, PR_REF_PREFIX, WORKDIR_REF, github_fetch_source_for_repo,
        github_repo_key_from_remote_url, github_repo_url_from_remote_transport,
        local_remote_for_github_repo, pr_ref_path,
    };
    use crate::core::vcs::git::{GitService, StatusItem, StatusOperation, StatusScope};

    fn commit_file(repo: &Repository, relative_path: &str, content: &str, message: &str) -> String {
        // Pin `core.autocrlf=false` on the test repo so LF content written
        // from these tests survives index → workdir round-trips unchanged.
        // On Windows, the default global `core.autocrlf=true` causes git to
        // normalize LF in the index but emit CRLF on checkout, which breaks
        // byte-exact assertions in the Discard path. Idempotent: safe to
        // call once per commit_file invocation.
        repo.config()
            .and_then(|mut cfg| cfg.set_bool("core.autocrlf", false))
            .expect("disable autocrlf on test repo");

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
    fn github_remote_url_parser_accepts_local_protocols() {
        assert_eq!(
            github_repo_key_from_remote_url("git@github.com:Owner/Repo.git"),
            Some(("owner".to_owned(), "repo".to_owned()))
        );
        assert_eq!(
            github_repo_key_from_remote_url("ssh://git@github.com/owner/repo.git"),
            Some(("owner".to_owned(), "repo".to_owned()))
        );
        assert_eq!(
            github_repo_key_from_remote_url("https://token@github.com/owner/repo"),
            Some(("owner".to_owned(), "repo".to_owned()))
        );
        assert_eq!(
            github_repo_key_from_remote_url("https://example.com/owner/repo"),
            None
        );
    }

    #[test]
    fn github_remote_transport_rewrites_repo_without_forcing_https() {
        assert_eq!(
            github_repo_url_from_remote_transport("git@github.com:me/fork.git", "owner", "repo"),
            Some("git@github.com:owner/repo.git".to_owned())
        );
        assert_eq!(
            github_repo_url_from_remote_transport(
                "ssh://git@github.com:22/me/fork.git",
                "owner",
                "repo",
            ),
            Some("ssh://git@github.com:22/owner/repo.git".to_owned())
        );
        assert_eq!(
            github_repo_url_from_remote_transport(
                "https://token@github.com/me/fork.git",
                "owner",
                "repo",
            ),
            Some("https://github.com/owner/repo.git".to_owned())
        );
    }

    #[test]
    fn local_remote_for_github_repo_prefers_matching_remote_name() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:me/fork.git").unwrap();
        repo.remote("upstream", "git@github.com:Owner/Repo.git")
            .unwrap();

        let remote = local_remote_for_github_repo(&repo, "owner", "repo").unwrap();

        assert_eq!(remote, "upstream");
    }

    #[test]
    fn github_fetch_source_uses_local_remote_protocol_when_base_remote_is_missing() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:me/fork.git").unwrap();

        let source = github_fetch_source_for_repo(
            &repo,
            "owner",
            "repo",
            "https://github.com/owner/repo.git",
        );

        assert_eq!(source, "git@github.com:owner/repo.git");
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

    /// Build two repos: a bare "remote" and a local clone with a tracking
    /// branch. Used to exercise fetch/push/pull-ff end-to-end.
    fn make_remote_and_clone() -> (TempDir, TempDir) {
        let remote_dir = TempDir::new().unwrap();
        let remote = Repository::init_bare(remote_dir.path()).unwrap();

        let seed_dir = TempDir::new().unwrap();
        {
            let seed = Repository::init(seed_dir.path()).unwrap();
            commit_file(&seed, "README.md", "hello\n", "initial");
            seed.remote("origin", remote_dir.path().to_str().unwrap())
                .unwrap()
                .push::<&str>(&["+HEAD:refs/heads/main"], None)
                .unwrap();
        }
        let _ = remote.set_head("refs/heads/main");

        // Clone into "local" working copy.
        let local_dir = TempDir::new().unwrap();
        let _local =
            Repository::clone(remote_dir.path().to_str().unwrap(), local_dir.path()).unwrap();

        (remote_dir, local_dir)
    }

    #[test]
    fn fetch_remote_updates_tracking_branch() {
        let (remote_dir, local_dir) = make_remote_and_clone();

        // Advance the remote via a separate working clone.
        let advance_dir = TempDir::new().unwrap();
        let advance =
            Repository::clone(remote_dir.path().to_str().unwrap(), advance_dir.path()).unwrap();
        commit_file(&advance, "README.md", "hello\nworld\n", "second");
        let mut advance_remote = advance.find_remote("origin").unwrap();
        advance_remote
            .push::<&str>(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();

        let mut git = GitService::new();
        git.open(local_dir.path().to_str().unwrap()).unwrap();

        let before = git.branches().unwrap();
        let main_before = before.iter().find(|b| b.is_head).unwrap();
        assert_eq!(main_before.ahead_behind, Some((0, 0)));

        git.fetch_remote("origin", |_, _, _| {}).unwrap();

        let after = git.branches().unwrap();
        let main_after = after.iter().find(|b| b.is_head).unwrap();
        assert_eq!(main_after.ahead_behind, Some((0, 1)));
    }

    #[test]
    fn pull_ff_fast_forwards_when_possible() {
        let (remote_dir, local_dir) = make_remote_and_clone();

        let advance_dir = TempDir::new().unwrap();
        let advance =
            Repository::clone(remote_dir.path().to_str().unwrap(), advance_dir.path()).unwrap();
        commit_file(&advance, "README.md", "hello\nworld\n", "second");
        advance
            .find_remote("origin")
            .unwrap()
            .push::<&str>(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();

        let mut git = GitService::new();
        git.open(local_dir.path().to_str().unwrap()).unwrap();

        let outcome = git.pull_ff("origin", "main", |_, _, _| {}).unwrap();
        assert_eq!(outcome, super::PullOutcome::FastForwarded { behind: 1 });

        let branches = git.branches().unwrap();
        let main = branches.iter().find(|b| b.is_head).unwrap();
        assert_eq!(main.ahead_behind, Some((0, 0)));
    }

    #[test]
    fn pull_ff_refuses_when_diverged() {
        let (remote_dir, local_dir) = make_remote_and_clone();

        // Diverge remote.
        let advance_dir = TempDir::new().unwrap();
        let advance =
            Repository::clone(remote_dir.path().to_str().unwrap(), advance_dir.path()).unwrap();
        commit_file(&advance, "README.md", "hello\nremote\n", "remote-change");
        advance
            .find_remote("origin")
            .unwrap()
            .push::<&str>(&["refs/heads/main:refs/heads/main"], None)
            .unwrap();

        // Diverge local.
        let local = Repository::open(local_dir.path()).unwrap();
        commit_file(&local, "NOTES.md", "local\n", "local-change");

        let mut git = GitService::new();
        git.open(local_dir.path().to_str().unwrap()).unwrap();

        let err = git
            .pull_ff("origin", "main", |_, _, _| {})
            .expect_err("diverged branch must refuse");
        assert!(matches!(err, super::PullError::NonFastForward { .. }));
    }

    #[test]
    fn push_updates_remote_ref() {
        let (remote_dir, local_dir) = make_remote_and_clone();

        let local = Repository::open(local_dir.path()).unwrap();
        commit_file(&local, "NOTES.md", "local\n", "local-change");

        let mut git = GitService::new();
        git.open(local_dir.path().to_str().unwrap()).unwrap();

        git.push(
            "origin",
            "refs/heads/main:refs/heads/main",
            false,
            |_, _, _| {},
        )
        .unwrap();

        // Reopen the bare remote and confirm HEAD advanced.
        let remote = Repository::open(remote_dir.path()).unwrap();
        let remote_main = remote
            .find_branch("main", BranchType::Local)
            .unwrap()
            .get()
            .target()
            .unwrap();
        let local_head = local.head().unwrap().target().unwrap();
        assert_eq!(remote_main, local_head);
    }
}
