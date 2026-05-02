use std::path::{Path, PathBuf};

use carbon::TextStore;

use crate::core::compare::backends::{DifftasticBackend, GitDiffBackend};
use crate::core::compare::{CompareMode, ComparePhase, CompareService, CompareSpec, ProgressSink};
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::backend::{VcsBackend, VcsRepository, VcsWatchPaths};
use crate::core::vcs::git::status::StatusBits;
use crate::core::vcs::git::{
    BranchInfo, CommitInfo, GitService, StatusItem, StatusScope, TagInfo, WORKDIR_REF,
    status::status_items_from_entry,
};
use crate::core::vcs::model::{
    ChangeBucket, ChangeFlags, FileChange, FileChangeStatus, RefKind, RepoCapabilities,
    RepoLocation, RevisionId, VcsChange, VcsCompareRequest, VcsCompareSpec, VcsKind, VcsRef,
    VcsSnapshot,
};
use crate::events::{RepositoryChangeKind, RepositorySyncReason};

#[derive(Debug, Clone, Copy, Default)]
pub struct GitBackend;

impl VcsBackend for GitBackend {
    fn kind(&self) -> VcsKind {
        VcsKind::Git
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
            kind: VcsKind::Git,
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

    fn resolve_compare_spec(
        &mut self,
        spec: &CompareSpec,
    ) -> Result<(String, String, VcsCompareRequest)> {
        let (resolved_left, resolved_right) =
            self.service
                .resolve_comparison(&spec.left_ref, &spec.right_ref, spec.mode)?;
        let backend_spec = VcsCompareRequest {
            spec: VcsCompareSpec::Range {
                from: resolved_left.clone(),
                to: resolved_right.clone(),
            },
            layout: spec.layout,
            renderer: spec.renderer,
        };
        Ok((resolved_left, resolved_right, backend_spec))
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

pub fn detect_git_location(path: &Path) -> Result<Option<RepoLocation>> {
    GitBackend.detect(path)
}

pub fn git_capabilities() -> RepoCapabilities {
    RepoCapabilities::git()
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
            kind: VcsKind::Git,
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
