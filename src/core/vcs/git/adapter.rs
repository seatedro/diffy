use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::backends::{DifftasticBackend, GitDiffBackend};
use crate::core::compare::{
    CompareMode, ComparePhase, CompareService, CompareSpec, ProgressSink, RendererKind,
};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::git::status::StatusBits;
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, GitService, PatchApplyTarget, PullOutcome, StatusItem, StatusOperation,
    StatusScope, TagInfo, WORKDIR_REF, status::status_items_from_entry,
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
            .map_err(|error| DiffyError::General(error.to_string()))?;
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
        let summary = self
            .service
            .commits(&oid, 1)
            .ok()
            .and_then(|mut commits| commits.pop())
            .map(|commit| commit.summary)
            .unwrap_or_default();
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
        let branches = self.service.branches()?;
        let tags = self.service.tags()?;
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
        let spec = git_compare_spec(request);
        CompareService::default().compare(&spec, &self.service, reporter)
    }

    fn compare_stats(&mut self, request: &VcsCompareRequest) -> Result<(i32, i32)> {
        let spec = git_compare_spec(request);
        GitDiffBackend
            .compare_stats(&spec, &self.service)?
            .ok_or_else(|| DiffyError::General("compare stats returned no result".to_owned()))
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

    fn compare_file_stats(&mut self, files: &[&carbon::FileDiff]) -> Result<Vec<(i32, i32)>> {
        let repo_path = self.location.workspace_root.to_string_lossy();
        let file_stats =
            GitDiffBackend.deferred_file_line_stats_batch_for_repo_path(files, &repo_path);
        Ok(files
            .iter()
            .zip(file_stats)
            .map(|(file, stat)| {
                stat.unwrap_or((
                    u32_to_i32_saturating(file.additions),
                    u32_to_i32_saturating(file.deletions),
                ))
            })
            .collect())
    }

    fn compare_path(
        &mut self,
        request: &VcsCompareRequest,
        path: &str,
        deferred_file: Option<&carbon::FileDiff>,
    ) -> Result<crate::core::compare::CompareOutput> {
        let spec = git_compare_spec(request);
        match request.renderer {
            crate::core::compare::RendererKind::Builtin => {
                let output = deferred_file
                    .map(|file| GitDiffBackend.compare_deferred_file(file, &self.service))
                    .transpose()?
                    .flatten();
                match output {
                    Some(output) => Ok(output),
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
                    .map(|file| GitDiffBackend.compare_deferred_file(file, &self.service))
                    .transpose()?
                    .flatten();
                match output {
                    Some(output) => Ok(output),
                    None => {
                        let mut output = GitDiffBackend
                            .compare_path(&spec, path, &self.service)?
                            .ok_or_else(|| {
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
            .ok_or_else(|| DiffyError::General("No current branch to push.".to_owned()))?;
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
            _ => Err(DiffyError::General(
                "Git cannot run this publish action".to_owned(),
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
            .map_err(|error| DiffyError::General(error.to_string()))
    }

    fn resolve_pull_request_comparison(
        &mut self,
        pull_request_url: &str,
        github_token: &str,
    ) -> Result<(String, String)> {
        self.service
            .resolve_pull_request_comparison(pull_request_url, github_token)
    }

    fn compare_working_file(&mut self, path: &str) -> Result<crate::core::compare::CompareOutput> {
        Err(DiffyError::General(format!(
            "Git working-file compare requires a status scope for {path}"
        )))
    }

    fn read_file_text(&mut self, revision: &RevisionId, path: &str) -> Result<TextStore> {
        self.service.read_file_text_store_at(&revision.id, path)
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

fn u32_to_i32_saturating(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
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
        .unwrap_or_else(|| RepoLocation {
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
            old_path: None,
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
        .flat_map(|(path, status)| {
            status_items_from_entry(path.clone(), sanitize_status_bits(*status))
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
