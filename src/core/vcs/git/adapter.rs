use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use carbon::TextStore;

use crate::core::compare::backends::{DifftasticBackend, GitDiffBackend};
use crate::core::compare::{
    CompareFileStatsTarget, CompareFileSummary, CompareMode, ComparePhase, CompareService,
    CompareSpec, ProgressSink, RendererKind,
};
use crate::core::error::{DiffyError, Result, VcsBackendKind};
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::cache::VcsReadCache;
use crate::core::vcs::git::service::is_full_hex_oid;
use crate::core::vcs::git::status::StatusBits;
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, GitService, PatchApplyTarget, PullOutcome, StatusItem, StatusOperation,
    StatusScope, TagInfo, WORKDIR_REF, status::status_items_from_entry_with_old_path,
};
use crate::core::vcs::model::{
    ChangeBucket, ChangeFlags, FileChange, FileChangeStatus, FileOperation, PublishAction,
    PublishActionKind, PublishOutcome, PublishPlan, PullFastForwardOutcome, RefKind,
    RepoCapabilities, RepoLocation, RevisionId, VCS_PROFILE_GIT, VcsChange, VcsCompareRequest,
    VcsCompareSpec, VcsKind, VcsRef, VcsSnapshot,
};
use crate::events::{RepositoryChangeKind, RepositorySyncReason};

#[derive(Debug, Clone, Copy, Default)]
pub struct GitBackend;

impl VcsBackend for GitBackend {
    fn kind(&self) -> VcsKind {
        VcsKind::GIT
    }

    fn detect(&self, path: &Path) -> Result<Option<RepoLocation>> {
        let repo = match gix::open(path) {
            Ok(repo) => repo,
            Err(_) => return Ok(None),
        };
        let workspace_root = repo
            .workdir()
            .map(Path::to_path_buf)
            .or_else(|| repo.git_dir().parent().map(Path::to_path_buf))
            .unwrap_or_else(|| path.to_path_buf());
        Ok(Some(RepoLocation {
            kind: VcsKind::GIT,
            profile: VCS_PROFILE_GIT,
            workspace_root,
            store_root: Some(repo.git_dir().to_path_buf()),
        }))
    }

    fn open(&self, location: RepoLocation) -> Result<Box<dyn VcsRepository>> {
        Ok(Box::new(GitRepository::open(location)?))
    }

    fn watch_paths(&self, location: &RepoLocation) -> Result<VcsWatchPaths> {
        let repo = gix::open(&location.workspace_root)
            .map_err(|error| DiffyError::vcs(VcsBackendKind::Git, "open", error.to_string()))?;
        let metadata_dir = repo.git_dir().to_path_buf();
        let workdir = repo.workdir().map(Path::to_path_buf);
        let watched_paths = match workdir.as_ref() {
            Some(workdir) if metadata_dir.starts_with(workdir) => vec![workdir.clone()],
            Some(workdir) => vec![workdir.clone(), metadata_dir.clone()],
            None => vec![metadata_dir.clone()],
        };
        Ok(VcsWatchPaths {
            worktree_metadata_paths: vec![
                metadata_dir.join("index"),
                metadata_dir.join("index.lock"),
            ],
            metadata_dir,
            workdir,
            watched_paths,
        })
    }
}

/// Process-wide cache for Git reads whose results are immutable: compares
/// and file reads addressed entirely by full commit OIDs. Such results are
/// content-addressed by the object store and can never go stale — not even
/// across writes, fetches, or ref updates — so no invalidation is needed and
/// it is safe to share across the short-lived `GitRepository` instances the
/// runtime opens per operation. The cache epoch slot carries the workspace
/// root so entries never leak across repositories. Anything involving
/// movable refs, the index, or the working tree is deliberately not cached
/// (see the staleness note in `VcsReadCache`).
static IMMUTABLE_READ_CACHE: LazyLock<Mutex<VcsReadCache>> =
    LazyLock::new(|| Mutex::new(VcsReadCache::new()));

/// True when every revision in the request is a full hex OID, making the
/// compare result content-addressed: parents, trees, and merge bases of
/// fixed commits are themselves fixed.
fn compare_request_is_immutable(request: &VcsCompareRequest) -> bool {
    match &request.spec {
        VcsCompareSpec::WorkingCopy => false,
        VcsCompareSpec::Change { revision } => is_full_hex_oid(revision),
        VcsCompareSpec::Range { from, to } => is_full_hex_oid(from) && is_full_hex_oid(to),
        VcsCompareSpec::MergeBaseRange { base, head } => {
            is_full_hex_oid(base) && is_full_hex_oid(head)
        }
    }
}

pub struct GitRepository {
    service: GitService,
    location: RepoLocation,
}

impl GitRepository {
    pub fn open(location: RepoLocation) -> Result<Self> {
        let mut service = GitService::new();
        service.open(location.workspace_root.to_string_lossy().as_ref())?;
        Ok(Self { service, location })
    }

    /// Epoch for [`IMMUTABLE_READ_CACHE`]: scopes entries to this repository.
    fn immutable_cache_epoch(&self) -> String {
        self.location.workspace_root.to_string_lossy().into_owned()
    }

    fn compare_path_uncached(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        deferred_file: Option<&CompareFileSummary>,
    ) -> Result<crate::core::compare::CompareOutput> {
        let spec = git_compare_spec(request);
        let deferred_file = deferred_file.map(CompareFileSummary::to_file_diff);
        let summary_fallback = deferred_file.is_some();
        match request.renderer {
            crate::core::compare::RendererKind::Builtin => {
                let output = deferred_file
                    .as_ref()
                    .map(|file| GitDiffBackend.compare_deferred_file(file, &self.service))
                    .transpose()?
                    .flatten();
                match output {
                    Some(output) => Ok(output),
                    None if summary_fallback => GitDiffBackend
                        .compare_path_no_renames(&spec, path, &self.service)?
                        .ok_or_else(|| {
                            DiffyError::General("compare file returned no result".to_owned())
                        }),
                    None => GitDiffBackend
                        .compare_path(&spec, path, &self.service)?
                        .ok_or_else(|| {
                            DiffyError::General("compare file returned no result".to_owned())
                        }),
                }
            }
            crate::core::compare::RendererKind::Difftastic if DifftasticBackend::is_available() => {
                DifftasticBackend
                    .compare_path(&spec, path, &self.service)?
                    .ok_or_else(|| {
                        DiffyError::General("compare file returned no result".to_owned())
                    })
            }
            crate::core::compare::RendererKind::Difftastic => {
                let output = deferred_file
                    .as_ref()
                    .map(|file| GitDiffBackend.compare_deferred_file(file, &self.service))
                    .transpose()?
                    .flatten();
                match output {
                    Some(output) => Ok(output),
                    None => {
                        let path_output = if summary_fallback {
                            GitDiffBackend.compare_path_no_renames(&spec, path, &self.service)?
                        } else {
                            GitDiffBackend.compare_path(&spec, path, &self.service)?
                        };
                        let mut output = path_output.ok_or_else(|| {
                            DiffyError::General("compare file returned no result".to_owned())
                        })?;
                        output.used_fallback = true;
                        output.fallback_message =
                            "difftastic not compiled in, used built-in backend".to_owned();
                        Ok(output)
                    }
                }
            }
        }
    }
}

impl VcsRepository for GitRepository {
    fn location(&self) -> &RepoLocation {
        &self.location
    }

    fn capabilities(&self) -> RepoCapabilities {
        git_capabilities()
    }

    fn resolve_ref(&mut self, reference: &str) -> Result<(String, String)> {
        let normalized;
        let reference =
            if reference == "@" || reference.starts_with("@~") || reference.starts_with("@^") {
                normalized = format!("HEAD{}", &reference[1..]);
                &normalized
            } else {
                reference
            };
        let oid = self.service.resolve_commit_oid(reference)?;
        let short_oid = self
            .service
            .abbreviate_oid(&oid)
            .unwrap_or_else(|_| oid[..7].to_owned());
        let summary = self.service.commit_summary(&oid).unwrap_or_default();
        Ok((short_oid, summary))
    }

    fn snapshot(
        &mut self,
        reason: RepositorySyncReason,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<VcsSnapshot> {
        if let Some(reporter) = reporter {
            reporter.phase(ComparePhase::ResolvingRefs);
        }
        let (branches, tags) = self.service.branches_and_tags()?;
        if let Some(reporter) = reporter {
            reporter.phase(ComparePhase::FetchingHistory);
        }
        let commits = self.service.commits("HEAD", 200).unwrap_or_default();
        let status_items = git_status_items(&self.service)?;
        Ok(git_snapshot_from_parts(
            self.location.workspace_root.clone(),
            reason,
            None,
            &branches,
            &tags,
            &commits,
            &status_items,
        ))
    }

    fn resolve_compare_request(&mut self, request: &VcsCompareRequest) -> Result<(String, String)> {
        let spec = git_compare_spec(request);
        self.service
            .resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)
    }

    fn compare(
        &mut self,
        request: &VcsCompareRequest,
        reporter: Option<&dyn ProgressSink>,
    ) -> Result<crate::core::compare::CompareOutput> {
        let cacheable = compare_request_is_immutable(request);
        let epoch = self.immutable_cache_epoch();
        if cacheable
            && let Ok(cache) = IMMUTABLE_READ_CACHE.lock()
            && let Some(output) = cache.cached_diff(Some(&epoch), request, None)
        {
            return Ok(output);
        }
        let spec = git_compare_spec(request);
        let output = CompareService::default().compare(&spec, &self.service, reporter)?;
        if cacheable && let Ok(mut cache) = IMMUTABLE_READ_CACHE.lock() {
            cache.insert_diff(Some(epoch), request.clone(), None, output.clone());
        }
        Ok(output)
    }

    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)> {
        let cacheable = compare_request_is_immutable(request);
        let epoch = self.immutable_cache_epoch();
        if cacheable
            && let Ok(cache) = IMMUTABLE_READ_CACHE.lock()
            && let Some(stats) = cache.cached_stats(Some(&epoch), request)
        {
            return Ok(stats);
        }
        let spec = git_compare_spec(request);
        let stats = GitDiffBackend
            .compare_stats(&spec, &self.service)?
            .ok_or_else(|| DiffyError::General("compare stats returned no result".to_owned()))?;
        if cacheable && let Ok(mut cache) = IMMUTABLE_READ_CACHE.lock() {
            cache.insert_stats(Some(epoch), request.clone(), stats);
        }
        Ok(stats)
    }

    fn compare_history(
        &mut self,
        left_ref: &str,
        right_ref: &str,
        limit: usize,
    ) -> Result<Vec<VcsChange>> {
        let commits = self
            .service
            .commits_in_range(left_ref, right_ref, limit)
            .unwrap_or_default();
        Ok(git_changes(&commits, &[]))
    }

    fn compare_file_stats(
        &mut self,
        request: &VcsCompareRequest,
        files: &[CompareFileStatsTarget],
    ) -> Result<Vec<(i32, i32)>> {
        let spec = git_compare_spec(request);
        let file_stats =
            GitDiffBackend.deferred_file_line_stats_batch_for_request(&spec, &self.service, files);
        Ok(files
            .iter()
            .zip(file_stats)
            .map(|(file, stat)| stat.unwrap_or_else(|| file.fallback_stats()))
            .collect())
    }

    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        deferred_file: Option<&CompareFileSummary>,
    ) -> Result<crate::core::compare::CompareOutput> {
        // Only the summary-less path shells out to `git diff`; deferred-file
        // compares already run in-process on gix blobs, and their rename
        // handling differs, so they are not folded into the same cache key.
        let cacheable = deferred_file.is_none() && compare_request_is_immutable(request);
        let epoch = self.immutable_cache_epoch();
        if cacheable
            && let Ok(cache) = IMMUTABLE_READ_CACHE.lock()
            && let Some(output) = cache.cached_diff(Some(&epoch), request, Some(path))
        {
            return Ok(output);
        }
        let output = self.compare_path_uncached(request, path, deferred_file)?;
        if cacheable && let Ok(mut cache) = IMMUTABLE_READ_CACHE.lock() {
            cache.insert_diff(
                Some(epoch),
                request.clone(),
                Some(path.to_owned()),
                output.clone(),
            );
        }
        Ok(output)
    }

    fn file_change_diff(
        &mut self,
        change: &FileChange,
        renderer: RendererKind,
    ) -> Result<crate::core::compare::CompareOutput> {
        let item = status_item_from_file_change(change);
        let mut output = match renderer {
            RendererKind::Builtin => self.service.diff_status_item(&item)?,
            RendererKind::Difftastic if DifftasticBackend::is_available() => {
                compare_status_item_with_difftastic(&item, &self.service)?
            }
            RendererKind::Difftastic => self.service.diff_status_item(&item)?,
        };
        if renderer == RendererKind::Difftastic && !DifftasticBackend::is_available() {
            output.used_fallback = true;
            output.fallback_message =
                "difftastic not compiled in, used built-in backend".to_owned();
        }
        Ok(output)
    }

    fn commit_diff(&mut self, has_staged: bool) -> Result<String> {
        self.service.diff_for_commit(has_staged)
    }

    fn apply_file_operation(
        &mut self,
        change: &FileChange,
        operation: FileOperation,
    ) -> Result<()> {
        let item = status_item_from_file_change(change);
        self.service
            .apply_status_operation(&item, status_operation_from_file_operation(operation))
    }

    fn apply_batch_file_operation(
        &mut self,
        changes: &[FileChange],
        operation: FileOperation,
    ) -> Result<()> {
        let items = changes
            .iter()
            .map(status_item_from_file_change)
            .collect::<Vec<_>>();
        self.service
            .apply_batch_status_operation(&items, status_operation_from_file_operation(operation))
    }

    fn apply_patch_operation(&mut self, patch: &str, operation: FileOperation) -> Result<()> {
        let target = match operation {
            FileOperation::Discard => PatchApplyTarget::Workdir,
            FileOperation::Stage | FileOperation::Unstage => PatchApplyTarget::Index,
        };
        self.service.apply_patch(patch, target)
    }

    fn create_commit(&mut self, message: &str) -> Result<()> {
        self.service.commit(message).map(|_| ())
    }

    fn fetch_remote(&mut self, remote: &str) -> Result<()> {
        self.service.fetch_remote(remote, |_, _, _| {})
    }

    fn push(&mut self, remote: &str, refspec: &str, force_with_lease: bool) -> Result<()> {
        self.service
            .push(remote, refspec, force_with_lease, |_, _, _| {})
    }

    fn publish_plan(&mut self) -> Result<PublishPlan> {
        let branches = self.service.branches()?;
        let branch = branches
            .iter()
            .find(|branch| branch.is_head && !branch.is_remote)
            .ok_or_else(|| {
                DiffyError::vcs(VcsBackendKind::Git, "publish", "no current branch to push")
            })?;
        let (remote, upstream_branch) = branch
            .upstream
            .as_deref()
            .and_then(upstream_pair)
            .unwrap_or_else(|| {
                let remote = preferred_git_remote(&branches).unwrap_or_else(|| "origin".to_owned());
                (remote, branch.name.clone())
            });
        let refspec = format!("refs/heads/{}:refs/heads/{upstream_branch}", branch.name);
        Ok(PublishPlan {
            primary: PublishAction {
                label: format!("Push {}", branch.name),
                description: format!("Push {} to {remote}/{upstream_branch}", branch.name),
                kind: PublishActionKind::PushRef {
                    remote,
                    refspec,
                    force_with_lease: false,
                },
                disabled_reason: None,
                change_id_token: None,
            },
            alternatives: Vec::new(),
        })
    }

    fn publish(&mut self, action: &PublishAction) -> Result<PublishOutcome> {
        match &action.kind {
            PublishActionKind::PushRef {
                remote,
                refspec,
                force_with_lease,
            } => {
                self.push(remote, refspec, *force_with_lease)?;
                Ok(PublishOutcome {
                    label: completed_publish_label(&action.label),
                })
            }
            _ => Err(DiffyError::vcs_fatal(
                VcsBackendKind::Git,
                "publish",
                "Git cannot run this publish action",
            )),
        }
    }

    fn pull_fast_forward(&mut self, remote: &str, branch: &str) -> Result<PullFastForwardOutcome> {
        self.service
            .pull_ff(remote, branch, |_, _, _| {})
            .map(|outcome| match outcome {
                PullOutcome::AlreadyUpToDate => PullFastForwardOutcome::AlreadyUpToDate,
                PullOutcome::FastForwarded { behind } => {
                    PullFastForwardOutcome::FastForwarded { behind }
                }
            })
            .map_err(|error| DiffyError::vcs(VcsBackendKind::Git, "pull", error.to_string()))
    }

    fn resolve_pull_request_comparison(
        &mut self,
        pull_request_url: &str,
        github_token: &str,
    ) -> Result<(crate::core::forge::github::PullRequestInfo, String, String)> {
        self.service
            .resolve_pull_request_comparison(pull_request_url, github_token)
    }

    fn compare_working_file(&mut self, path: &str) -> Result<crate::core::compare::CompareOutput> {
        Err(DiffyError::General(format!(
            "Git working-file compare requires a status scope for {path}"
        )))
    }

    fn read_file_text(&mut self, revision: &RevisionId, path: &str) -> Result<TextStore> {
        // Blob content at a fixed commit OID is immutable; workdir, index,
        // and symbolic refs are not and bypass the cache.
        let cacheable = is_full_hex_oid(&revision.id);
        let epoch = self.immutable_cache_epoch();
        if cacheable
            && let Ok(cache) = IMMUTABLE_READ_CACHE.lock()
            && let Some(text) = cache.cached_file_text(Some(&epoch), revision, path)
        {
            return Ok(text);
        }
        let text = self.service.read_file_text_store_at(&revision.id, path)?;
        if cacheable && let Ok(mut cache) = IMMUTABLE_READ_CACHE.lock() {
            cache.insert_file_text(Some(epoch), revision.clone(), path.to_owned(), text.clone());
        }
        Ok(text)
    }
}

fn git_compare_spec(request: &VcsCompareRequest) -> CompareSpec {
    let (left_ref, right_ref, mode) = match &request.spec {
        VcsCompareSpec::WorkingCopy => (
            "HEAD".to_owned(),
            WORKDIR_REF.to_owned(),
            CompareMode::TwoDot,
        ),
        VcsCompareSpec::Change { revision } => {
            (String::new(), revision.clone(), CompareMode::SingleCommit)
        }
        VcsCompareSpec::Range { from, to } => (from.clone(), to.clone(), CompareMode::TwoDot),
        VcsCompareSpec::MergeBaseRange { base, head } => {
            (base.clone(), head.clone(), CompareMode::ThreeDot)
        }
    };
    CompareSpec {
        left_ref,
        right_ref,
        mode,
        layout: request.layout,
        renderer: request.renderer,
    }
}

fn upstream_pair(upstream: &str) -> Option<(String, String)> {
    upstream
        .split_once('/')
        .map(|(remote, branch)| (remote.to_owned(), branch.to_owned()))
}

fn preferred_git_remote(branches: &[BranchInfo]) -> Option<String> {
    let mut remotes = branches
        .iter()
        .filter(|branch| branch.is_remote)
        .filter_map(|branch| {
            branch
                .name
                .split_once('/')
                .map(|(remote, _)| remote.to_owned())
        })
        .collect::<Vec<_>>();
    remotes.sort();
    remotes.dedup();
    remotes
        .iter()
        .find(|remote| remote.as_str() == "origin")
        .cloned()
        .or_else(|| remotes.into_iter().next())
}

fn completed_publish_label(label: &str) -> String {
    label
        .strip_prefix("Push ")
        .map(|suffix| format!("Pushed {suffix}"))
        .unwrap_or_else(|| label.to_owned())
}

pub fn detect_git_location(path: &Path) -> Result<Option<RepoLocation>> {
    GitBackend.detect(path)
}

pub fn git_capabilities() -> RepoCapabilities {
    RepoCapabilities::git()
}

fn status_item_from_file_change(change: &FileChange) -> StatusItem {
    let scope = match change.bucket {
        ChangeBucket::Staged => StatusScope::Staged,
        ChangeBucket::Untracked => StatusScope::Untracked,
        ChangeBucket::WorkingCopy | ChangeBucket::Unstaged | ChangeBucket::Conflicted => {
            StatusScope::Unstaged
        }
    };
    StatusItem {
        path: change.path.clone(),
        old_path: change.old_path.clone(),
        scope,
        status: file_change_status_label(change.status, change.bucket).to_owned(),
    }
}

fn status_operation_from_file_operation(operation: FileOperation) -> StatusOperation {
    match operation {
        FileOperation::Stage => StatusOperation::Stage,
        FileOperation::Unstage => StatusOperation::Unstage,
        FileOperation::Discard => StatusOperation::Discard,
    }
}

fn file_change_status_label(status: FileChangeStatus, bucket: ChangeBucket) -> &'static str {
    match (status, bucket) {
        (FileChangeStatus::Added, _) => "A",
        (FileChangeStatus::Deleted, _) => "D",
        (FileChangeStatus::Renamed, _) => "R",
        (FileChangeStatus::Copied, _) => "C",
        (FileChangeStatus::Untracked, _) => "U",
        (FileChangeStatus::Conflicted, _) | (_, ChangeBucket::Conflicted) => "!",
        (FileChangeStatus::TypeChanged, _) => "T",
        (FileChangeStatus::Binary, _) => "B",
        (FileChangeStatus::Modified, _) => "M",
    }
}

#[cfg(feature = "difftastic")]
fn compare_status_item_with_difftastic(
    item: &StatusItem,
    git: &GitService,
) -> Result<crate::core::compare::CompareOutput> {
    DifftasticBackend.compare_status_item(item, git)
}

#[cfg(not(feature = "difftastic"))]
fn compare_status_item_with_difftastic(
    _item: &StatusItem,
    _git: &GitService,
) -> Result<crate::core::compare::CompareOutput> {
    unreachable!("difftastic status compare is gated by DifftasticBackend::is_available()")
}

pub fn git_snapshot_from_parts(
    path: PathBuf,
    reason: RepositorySyncReason,
    change_kind: Option<RepositoryChangeKind>,
    branches: &[BranchInfo],
    tags: &[TagInfo],
    commits: &[CommitInfo],
    status_items: &[StatusItem],
) -> VcsSnapshot {
    let location = detect_git_location(&path)
        .ok()
        .flatten()
        .unwrap_or(RepoLocation {
            kind: VcsKind::GIT,
            profile: VCS_PROFILE_GIT,
            workspace_root: path,
            store_root: None,
        });
    VcsSnapshot {
        location,
        reason,
        change_kind,
        capabilities: git_capabilities(),
        refs: git_refs(branches, tags),
        changes: git_changes(commits, branches),
        operation_log: Vec::new(),
        file_changes: git_file_changes(status_items),
    }
}

pub fn git_refs(branches: &[BranchInfo], tags: &[TagInfo]) -> Vec<VcsRef> {
    let mut refs = Vec::with_capacity(branches.len() + tags.len() + 1);
    refs.push(VcsRef {
        name: WORKDIR_REF.to_owned(),
        kind: RefKind::WorkingCopy,
        target: RevisionId::git(WORKDIR_REF),
        active: false,
        upstream: None,
        ahead_behind: None,
    });
    refs.extend(branches.iter().map(|branch| VcsRef {
        name: branch.name.clone(),
        kind: if branch.is_remote {
            RefKind::RemoteBranch
        } else {
            RefKind::Branch
        },
        target: RevisionId::git(branch.target_oid.clone()),
        active: branch.is_head,
        upstream: branch.upstream.clone(),
        ahead_behind: branch.ahead_behind,
    }));
    refs.extend(tags.iter().map(|tag| VcsRef {
        name: tag.name.clone(),
        kind: RefKind::Tag,
        target: RevisionId::git(tag.target_oid.clone()),
        active: false,
        upstream: None,
        ahead_behind: None,
    }));
    refs
}

pub fn git_changes(commits: &[CommitInfo], branches: &[BranchInfo]) -> Vec<VcsChange> {
    let head_oid = branches
        .iter()
        .find(|branch| branch.is_head)
        .map(|branch| branch.target_oid.as_str());
    commits
        .iter()
        .map(|commit| VcsChange {
            revision: RevisionId::git(commit.oid.clone()),
            change_id: None,
            short_change_id: None,
            short_change_id_prefix_len: None,
            short_revision: commit.short_oid.clone(),
            summary: commit.summary.clone(),
            author_name: commit.author_name.clone(),
            timestamp: commit.timestamp,
            flags: ChangeFlags {
                current: head_oid == Some(commit.oid.as_str()),
                working_copy: false,
                divergent: false,
                immutable: false,
                conflicted: false,
            },
        })
        .collect()
}

pub fn git_file_changes(status_items: &[StatusItem]) -> Vec<FileChange> {
    status_items
        .iter()
        .map(|item| FileChange {
            path: item.path.clone(),
            old_path: item.old_path.clone(),
            status: git_file_status(item),
            bucket: git_change_bucket(item.scope),
        })
        .collect()
}

pub fn git_change_bucket(scope: StatusScope) -> ChangeBucket {
    match scope {
        StatusScope::Staged => ChangeBucket::Staged,
        StatusScope::Unstaged => ChangeBucket::Unstaged,
        StatusScope::Untracked => ChangeBucket::Untracked,
    }
}

pub fn git_file_status(item: &StatusItem) -> FileChangeStatus {
    match item.status.as_str() {
        "A" => FileChangeStatus::Added,
        "D" => FileChangeStatus::Deleted,
        "R" => FileChangeStatus::Renamed,
        "U" => FileChangeStatus::Untracked,
        "M" => FileChangeStatus::Modified,
        _ => FileChangeStatus::Modified,
    }
}

pub fn git_location_or_error(path: &Path) -> Result<RepoLocation> {
    detect_git_location(path)?
        .ok_or_else(|| DiffyError::General(format!("{} is not a Git repository", path.display())))
}

fn git_status_items(git: &GitService) -> Result<Vec<StatusItem>> {
    let mut status_items = git
        .status_entries()?
        .iter()
        .flat_map(|(path, old_path, status)| {
            status_items_from_entry_with_old_path(
                path.clone(),
                old_path.clone(),
                sanitize_status_bits(*status),
            )
        })
        .collect::<Vec<_>>();
    status_items.sort_by(|left, right| {
        left.scope
            .label()
            .cmp(right.scope.label())
            .then(left.path.cmp(&right.path))
    });
    Ok(status_items)
}

fn sanitize_status_bits(status: StatusBits) -> StatusBits {
    status
        & (StatusBits::INDEX_NEW
            | StatusBits::INDEX_MODIFIED
            | StatusBits::INDEX_DELETED
            | StatusBits::INDEX_RENAMED
            | StatusBits::INDEX_TYPECHANGE
            | StatusBits::WT_NEW
            | StatusBits::WT_MODIFIED
            | StatusBits::WT_DELETED
            | StatusBits::WT_TYPECHANGE
            | StatusBits::WT_RENAMED
            | StatusBits::CONFLICTED)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use git2::{Oid, Repository, Signature};
    use tempfile::TempDir;

    use super::{
        GitBackend, GitRepository, completed_publish_label, git_compare_spec,
        git_location_or_error, preferred_git_remote, status_item_from_file_change, upstream_pair,
    };
    use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
    use crate::core::vcs::backend::{VcsBackend, VcsRepository};
    use crate::core::vcs::git::{BranchInfo, StatusScope, WORKDIR_REF};
    use crate::core::vcs::model::{
        ChangeBucket, FileChange, FileChangeStatus, RefKind, RevisionId, VcsCompareRequest,
        VcsCompareSpec, VcsKind,
    };
    use crate::events::RepositorySyncReason;

    fn commit_file(
        repo: &Repository,
        relative_path: &str,
        content: &[u8],
        message: &str,
    ) -> String {
        // Pin `core.autocrlf=false` so LF content written from these tests
        // survives index round-trips unchanged on every platform.
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

    fn open_adapter(path: &Path) -> GitRepository {
        GitRepository::open(git_location_or_error(path).unwrap()).unwrap()
    }

    fn request(spec: VcsCompareSpec) -> VcsCompareRequest {
        VcsCompareRequest {
            spec,
            layout: LayoutMode::Unified,
            renderer: RendererKind::Builtin,
        }
    }

    fn remote_branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_owned(),
            is_remote: true,
            is_head: false,
            target_oid: "0".repeat(40),
            upstream: None,
            ahead_behind: None,
        }
    }

    #[test]
    fn detect_reports_repo_location_and_ignores_plain_directories() {
        let repo_dir = TempDir::new().unwrap();
        Repository::init(repo_dir.path()).unwrap();

        let location = GitBackend
            .detect(repo_dir.path())
            .unwrap()
            .expect("repository detected");
        assert_eq!(location.kind, VcsKind::GIT);
        assert_eq!(
            location.workspace_root.canonicalize().unwrap(),
            repo_dir.path().canonicalize().unwrap()
        );
        assert!(location.store_root.is_some());

        let plain_dir = TempDir::new().unwrap();
        assert!(GitBackend.detect(plain_dir.path()).unwrap().is_none());
    }

    #[test]
    fn resolve_ref_returns_short_oid_and_summary() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let oid = commit_file(&repo, "src/lib.rs", b"hello\n", "initial commit");

        let mut adapter = open_adapter(repo_dir.path());
        let (short_oid, summary) = adapter.resolve_ref("HEAD").unwrap();

        assert!(oid.starts_with(&short_oid), "{short_oid} prefixes {oid}");
        assert!(short_oid.len() < oid.len());
        assert_eq!(summary, "initial commit");
    }

    #[test]
    fn resolve_ref_normalizes_at_shorthand_to_head() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", b"one\n", "first");
        commit_file(&repo, "src/lib.rs", b"two\n", "second");

        let mut adapter = open_adapter(repo_dir.path());

        assert_eq!(
            adapter.resolve_ref("@").unwrap(),
            adapter.resolve_ref("HEAD").unwrap()
        );
        let (_, parent_summary) = adapter.resolve_ref("@~1").unwrap();
        assert_eq!(parent_summary, "first");
    }

    #[test]
    fn resolve_ref_rejects_unknown_reference() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", b"hello\n", "initial");

        let mut adapter = open_adapter(repo_dir.path());
        assert!(adapter.resolve_ref("does-not-exist").is_err());
    }

    #[test]
    fn snapshot_collects_refs_changes_and_file_changes() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let oid = commit_file(&repo, "src/lib.rs", b"hello\n", "initial commit");
        let target = repo
            .find_object(Oid::from_str(&oid).unwrap(), None)
            .unwrap();
        repo.tag_lightweight("v1", &target, false).unwrap();
        fs::write(repo_dir.path().join("src/lib.rs"), "changed\n").unwrap();
        fs::write(repo_dir.path().join("notes.txt"), "untracked\n").unwrap();
        let head_branch = repo.head().unwrap().shorthand().unwrap().to_owned();

        let mut adapter = open_adapter(repo_dir.path());
        let snapshot = adapter.snapshot(RepositorySyncReason::Open, None).unwrap();

        assert!(snapshot.capabilities.staging_area);
        assert_eq!(snapshot.refs[0].name, WORKDIR_REF);
        assert_eq!(snapshot.refs[0].kind, RefKind::WorkingCopy);

        let branch_ref = snapshot
            .refs
            .iter()
            .find(|vcs_ref| vcs_ref.name == head_branch)
            .expect("head branch listed");
        assert_eq!(branch_ref.kind, RefKind::Branch);
        assert!(branch_ref.active);
        assert_eq!(branch_ref.target, RevisionId::git(oid.clone()));

        let tag_ref = snapshot
            .refs
            .iter()
            .find(|vcs_ref| vcs_ref.name == "v1")
            .expect("tag listed");
        assert_eq!(tag_ref.kind, RefKind::Tag);
        assert_eq!(tag_ref.target, RevisionId::git(oid.clone()));

        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(snapshot.changes[0].revision, RevisionId::git(oid));
        assert_eq!(snapshot.changes[0].summary, "initial commit");
        assert!(snapshot.changes[0].flags.current);

        assert!(snapshot.file_changes.contains(&FileChange {
            path: "src/lib.rs".to_owned(),
            old_path: None,
            status: FileChangeStatus::Modified,
            bucket: ChangeBucket::Unstaged,
        }));
        assert!(snapshot.file_changes.contains(&FileChange {
            path: "notes.txt".to_owned(),
            old_path: None,
            status: FileChangeStatus::Untracked,
            bucket: ChangeBucket::Untracked,
        }));
    }

    #[test]
    fn snapshot_of_empty_repo_lists_untracked_files_without_history() {
        let repo_dir = TempDir::new().unwrap();
        Repository::init(repo_dir.path()).unwrap();
        fs::write(repo_dir.path().join("readme.md"), "hello\n").unwrap();

        let mut adapter = open_adapter(repo_dir.path());
        let snapshot = adapter.snapshot(RepositorySyncReason::Open, None).unwrap();

        assert_eq!(snapshot.refs.len(), 1);
        assert_eq!(snapshot.refs[0].name, WORKDIR_REF);
        assert!(snapshot.changes.is_empty());
        assert_eq!(
            snapshot.file_changes,
            vec![FileChange {
                path: "readme.md".to_owned(),
                old_path: None,
                status: FileChangeStatus::Untracked,
                bucket: ChangeBucket::Untracked,
            }]
        );
    }

    #[test]
    fn snapshot_with_detached_head_marks_no_branch_active() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", b"one\n", "first");
        let second = commit_file(&repo, "src/lib.rs", b"two\n", "second");
        repo.set_head_detached(Oid::from_str(&second).unwrap())
            .unwrap();

        let mut adapter = open_adapter(repo_dir.path());
        let snapshot = adapter
            .snapshot(RepositorySyncReason::Rescan, None)
            .unwrap();

        assert!(snapshot.refs.iter().all(|vcs_ref| !vcs_ref.active));
        assert!(
            snapshot
                .refs
                .iter()
                .any(|vcs_ref| vcs_ref.kind == RefKind::Branch)
        );
        assert_eq!(snapshot.changes.len(), 2);
        assert!(snapshot.changes.iter().all(|change| !change.flags.current));
    }

    #[test]
    fn snapshot_reports_staged_rename_with_old_path() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/old.rs", b"same content\n", "initial");
        fs::rename(
            repo_dir.path().join("src/old.rs"),
            repo_dir.path().join("src/new.rs"),
        )
        .unwrap();
        let mut index = repo.index().unwrap();
        index.remove_path(Path::new("src/old.rs")).unwrap();
        index.add_path(Path::new("src/new.rs")).unwrap();
        index.write().unwrap();

        let mut adapter = open_adapter(repo_dir.path());
        let snapshot = adapter.snapshot(RepositorySyncReason::Dirty, None).unwrap();

        assert_eq!(
            snapshot.file_changes,
            vec![FileChange {
                path: "src/new.rs".to_owned(),
                old_path: Some("src/old.rs".to_owned()),
                status: FileChangeStatus::Renamed,
                bucket: ChangeBucket::Staged,
            }]
        );
    }

    #[test]
    fn read_file_text_returns_content_at_revision() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/lib.rs", b"hello\nworld\n", "first");
        commit_file(&repo, "src/lib.rs", b"changed\n", "second");

        let mut adapter = open_adapter(repo_dir.path());

        let old_text = adapter
            .read_file_text(&RevisionId::git(first), "src/lib.rs")
            .unwrap();
        assert_eq!(old_text.as_str(), Some("hello\nworld\n"));

        let workdir_text = adapter
            .read_file_text(&RevisionId::git(WORKDIR_REF), "src/lib.rs")
            .unwrap();
        assert_eq!(workdir_text.as_str(), Some("changed\n"));

        assert!(
            adapter
                .read_file_text(&RevisionId::git("HEAD"), "src/missing.rs")
                .is_err()
        );
    }

    #[test]
    fn read_file_text_rejects_binary_content() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let oid = commit_file(&repo, "blob.bin", b"\x00\x01\x02binary", "add binary");

        let mut adapter = open_adapter(repo_dir.path());
        let error = adapter
            .read_file_text(&RevisionId::git(oid), "blob.bin")
            .expect_err("binary content rejected");
        assert!(error.to_string().contains("binary"), "{error}");
    }

    #[test]
    fn resolve_compare_request_resolves_each_spec() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/lib.rs", b"one\n", "first");
        let second = commit_file(&repo, "src/lib.rs", b"two\n", "second");

        let mut adapter = open_adapter(repo_dir.path());

        assert_eq!(
            adapter
                .resolve_compare_request(&request(VcsCompareSpec::WorkingCopy))
                .unwrap(),
            (second.clone(), WORKDIR_REF.to_owned())
        );
        assert_eq!(
            adapter
                .resolve_compare_request(&request(VcsCompareSpec::Range {
                    from: first.clone(),
                    to: second.clone(),
                }))
                .unwrap(),
            (first.clone(), second.clone())
        );
        // Single-commit mode diffs the commit against its first parent.
        assert_eq!(
            adapter
                .resolve_compare_request(&request(VcsCompareSpec::Change {
                    revision: second.clone(),
                }))
                .unwrap(),
            (first, second)
        );
    }

    #[test]
    fn compare_range_produces_file_diff() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        let first = commit_file(&repo, "src/lib.rs", b"old line\n", "first");
        let second = commit_file(&repo, "src/lib.rs", b"new line\n", "second");

        let mut adapter = open_adapter(repo_dir.path());
        let output = adapter
            .compare(
                &request(VcsCompareSpec::Range {
                    from: first,
                    to: second,
                }),
                None,
            )
            .unwrap();

        assert_eq!(output.file_count(), 1);
        let summary = output.summary_at(0).expect("file summary");
        assert_eq!(summary.paths.display_path(), "src/lib.rs");
        assert!(!output.used_fallback);
    }

    #[test]
    fn compare_working_file_requires_status_scope() {
        let repo_dir = TempDir::new().unwrap();
        let repo = Repository::init(repo_dir.path()).unwrap();
        commit_file(&repo, "src/lib.rs", b"hello\n", "initial");

        let mut adapter = open_adapter(repo_dir.path());
        assert!(adapter.compare_working_file("src/lib.rs").is_err());
    }

    #[test]
    fn git_compare_spec_maps_request_specs() {
        let working = git_compare_spec(&request(VcsCompareSpec::WorkingCopy));
        assert_eq!(working.left_ref, "HEAD");
        assert_eq!(working.right_ref, WORKDIR_REF);
        assert_eq!(working.mode, CompareMode::TwoDot);

        let change = git_compare_spec(&request(VcsCompareSpec::Change {
            revision: "abc123".to_owned(),
        }));
        assert_eq!(change.left_ref, "");
        assert_eq!(change.right_ref, "abc123");
        assert_eq!(change.mode, CompareMode::SingleCommit);

        let range = git_compare_spec(&request(VcsCompareSpec::Range {
            from: "left".to_owned(),
            to: "right".to_owned(),
        }));
        assert_eq!(
            (range.left_ref.as_str(), range.right_ref.as_str()),
            ("left", "right")
        );
        assert_eq!(range.mode, CompareMode::TwoDot);

        let merge_base = git_compare_spec(&request(VcsCompareSpec::MergeBaseRange {
            base: "main".to_owned(),
            head: "feature".to_owned(),
        }));
        assert_eq!(
            (merge_base.left_ref.as_str(), merge_base.right_ref.as_str()),
            ("main", "feature")
        );
        assert_eq!(merge_base.mode, CompareMode::ThreeDot);
    }

    #[test]
    fn status_item_from_file_change_maps_buckets_and_labels() {
        let staged_added = status_item_from_file_change(&FileChange {
            path: "src/new.rs".to_owned(),
            old_path: None,
            status: FileChangeStatus::Added,
            bucket: ChangeBucket::Staged,
        });
        assert_eq!(staged_added.scope, StatusScope::Staged);
        assert_eq!(staged_added.status, "A");

        let untracked = status_item_from_file_change(&FileChange {
            path: "notes.txt".to_owned(),
            old_path: None,
            status: FileChangeStatus::Untracked,
            bucket: ChangeBucket::Untracked,
        });
        assert_eq!(untracked.scope, StatusScope::Untracked);
        assert_eq!(untracked.status, "U");

        let conflicted = status_item_from_file_change(&FileChange {
            path: "src/lib.rs".to_owned(),
            old_path: None,
            status: FileChangeStatus::Modified,
            bucket: ChangeBucket::Conflicted,
        });
        assert_eq!(conflicted.scope, StatusScope::Unstaged);
        assert_eq!(conflicted.status, "!");

        let renamed = status_item_from_file_change(&FileChange {
            path: "src/new.rs".to_owned(),
            old_path: Some("src/old.rs".to_owned()),
            status: FileChangeStatus::Renamed,
            bucket: ChangeBucket::Unstaged,
        });
        assert_eq!(renamed.status, "R");
        assert_eq!(renamed.old_path.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn publish_helpers_pick_remote_and_label() {
        assert_eq!(
            upstream_pair("origin/main"),
            Some(("origin".to_owned(), "main".to_owned()))
        );
        assert_eq!(upstream_pair("main"), None);

        let with_origin = [remote_branch("upstream/main"), remote_branch("origin/main")];
        assert_eq!(
            preferred_git_remote(&with_origin).as_deref(),
            Some("origin")
        );
        let without_origin = [remote_branch("fork/main"), remote_branch("alt/dev")];
        assert_eq!(
            preferred_git_remote(&without_origin).as_deref(),
            Some("alt")
        );
        assert_eq!(preferred_git_remote(&[]), None);

        assert_eq!(completed_publish_label("Push main"), "Pushed main");
        assert_eq!(completed_publish_label("Publish"), "Publish");
    }
}
