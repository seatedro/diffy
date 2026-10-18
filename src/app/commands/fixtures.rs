use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::FileStatus;

use crate::contracts::navigation::{
    NavigationEvent, RepositoryRevision, RepositorySnapshot, RepositorySyncReason,
};
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision};
use crate::contracts::viewport::pipeline::{CompareManifestSnapshot, FileStats, ManifestFile};
use crate::core::compare::stats::CompareFilePaths;
use crate::core::vcs::model::{
    ChangeFlags, RefKind, RepoCapabilities, RepoLocation, RevisionId, VcsChange, VcsKind,
    VcsOperationLogEntry, VcsRef,
};

pub const REPO: &str = "/fixtures/alpha";

pub fn snapshot(reason: RepositorySyncReason, operation_log: bool) -> RepositorySnapshot {
    let root = PathBuf::from(REPO);
    RepositorySnapshot {
        location: RepoLocation {
            kind: VcsKind::GIT,
            profile: "git",
            workspace_root: root.clone(),
            store_root: None,
        },
        path: root,
        reason,
        change_kind: None,
        capabilities: RepoCapabilities {
            operation_log,
            ..RepoCapabilities::git()
        },
        refs: ["main", "feature"]
            .into_iter()
            .map(|name| VcsRef {
                name: name.to_owned(),
                kind: RefKind::Branch,
                target: RevisionId::git(format!("{name}0000")),
                active: name == "main",
                upstream: None,
                ahead_behind: None,
            })
            .collect(),
        changes: vec![change("c1", "First change"), change("c2", "Second change")],
        operation_log: vec![VcsOperationLogEntry {
            operation_id: "op1".to_owned(),
            short_operation_id: "op1".to_owned(),
            user: "user".to_owned(),
            time: "now".to_owned(),
            description: "snapshot working copy".to_owned(),
        }],
        file_changes: Vec::new(),
        publish_plan: None,
    }
}

pub fn change(revision: &str, summary: &str) -> VcsChange {
    VcsChange {
        revision: RevisionId::git(format!("{revision}aaaa")),
        change_id: None,
        short_change_id: None,
        short_change_id_prefix_len: None,
        short_revision: revision.to_owned(),
        summary: summary.to_owned(),
        author_name: "author".to_owned(),
        timestamp: 0,
        flags: ChangeFlags::default(),
    }
}

pub fn snapshot_ready(revision: u64, snapshot: RepositorySnapshot) -> NavigationEvent {
    NavigationEvent::SnapshotReady {
        revision: RepositoryRevision(revision),
        snapshot: Arc::new(snapshot),
    }
}

pub fn file(key: u64, path: &str) -> ManifestFile {
    ManifestFile {
        key: FileKey(key),
        revision: FileRevision(1),
        paths: CompareFilePaths::from_paths(Some(path), Some(path)),
        status: FileStatus::Modified,
        binary: false,
        stats: FileStats::Pending,
    }
}

pub fn renamed(key: u64, old: &str, new: &str) -> ManifestFile {
    ManifestFile {
        paths: CompareFilePaths::from_paths(Some(old), Some(new)),
        status: FileStatus::Renamed,
        ..file(key, new)
    }
}

pub fn manifest(generation: u64, files: Vec<ManifestFile>) -> Rc<CompareManifestSnapshot> {
    Rc::new(CompareManifestSnapshot {
        generation: CompareGeneration(generation),
        resolved_left: String::new(),
        resolved_right: String::new(),
        files: files.into(),
    })
}
