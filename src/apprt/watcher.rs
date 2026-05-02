use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use notify::event::{EventKind, MetadataKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::apprt::vcs_worker::VcsWorkerCommand;
use crate::core::vcs::discovery;
use crate::events::RepositoryChangeKind;

const REPO_WATCH_DEBOUNCE: Duration = Duration::from_millis(150);

pub struct RepoWatchWorker {
    sender: Sender<WatchCommand>,
}

impl RepoWatchWorker {
    pub fn new(dirty_sender: Sender<VcsWorkerCommand>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let callback_sender = sender.clone();
        thread::spawn(move || repo_watch_worker_loop(callback_sender, dirty_sender, receiver));
        Self { sender }
    }

    pub fn dispatch(&self, path: Option<PathBuf>) {
        let _ = self.sender.send(WatchCommand::SetRepo(path));
    }
}

enum WatchCommand {
    SetRepo(Option<PathBuf>),
    Notify(Result<Event, notify::Error>),
}

struct ActiveRepoWatch {
    request_path: PathBuf,
    metadata_dir: PathBuf,
    workdir: Option<PathBuf>,
    worktree_metadata_paths: Vec<PathBuf>,
    watched_paths: Vec<PathBuf>,
}

fn repo_watch_worker_loop(
    callback_sender: Sender<WatchCommand>,
    dirty_sender: Sender<VcsWorkerCommand>,
    receiver: Receiver<WatchCommand>,
) {
    let mut watcher = match notify::recommended_watcher(move |event| {
        let _ = callback_sender.send(WatchCommand::Notify(event));
    }) {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!("failed to create repository watcher: {error}");
            return;
        }
    };

    let mut active = None;
    let mut pending_dirty: Option<RepositoryChangeKind> = None;

    loop {
        let command = if pending_dirty.is_some() {
            match receiver.recv_timeout(REPO_WATCH_DEBOUNCE) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };

        match command {
            Some(WatchCommand::SetRepo(path)) => {
                pending_dirty = None;
                replace_active_watch(&mut watcher, &mut active, path);
            }
            Some(WatchCommand::Notify(Ok(event))) => {
                if let Some(active_watch) = active.as_ref()
                    && should_consider_event(&event)
                    && !is_ignored_metadata_event(active_watch, &event)
                {
                    let change_kind = classify_event(active_watch, &event);
                    pending_dirty = Some(match pending_dirty {
                        Some(existing) => existing.merge(change_kind),
                        None => change_kind,
                    });
                }
            }
            Some(WatchCommand::Notify(Err(error))) => {
                tracing::warn!("repository watcher error: {error}");
                pending_dirty = Some(RepositoryChangeKind::Both);
            }
            None => {
                let Some(active_watch) = active.as_ref() else {
                    pending_dirty = None;
                    continue;
                };
                let Some(change_hint) = pending_dirty.take() else {
                    continue;
                };
                let _ = dirty_sender.send(VcsWorkerCommand::Dirty {
                    path: active_watch.request_path.clone(),
                    change_hint,
                });
            }
        }
    }
}

fn replace_active_watch(
    watcher: &mut RecommendedWatcher,
    active: &mut Option<ActiveRepoWatch>,
    path: Option<PathBuf>,
) {
    if let Some(existing) = active.take() {
        unwatch_paths(watcher, &existing.watched_paths);
    }

    let Some(path) = path else {
        return;
    };

    let watch_paths = match discovery::watch_paths_for_repository(&path) {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!(
                repo = %path.display(),
                "failed to resolve repository watch paths: {error}"
            );
            return;
        }
    };

    for watch_path in &watch_paths.watched_paths {
        if let Err(error) = watcher.watch(watch_path, RecursiveMode::Recursive) {
            tracing::warn!(
                path = %watch_path.display(),
                "failed to watch repository path: {error}"
            );
        }
    }

    let metadata_dir = std::fs::canonicalize(&watch_paths.metadata_dir)
        .unwrap_or_else(|_| watch_paths.metadata_dir.clone());
    let workdir = watch_paths
        .workdir
        .as_ref()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()));
    let worktree_metadata_paths = watch_paths
        .worktree_metadata_paths
        .iter()
        .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
        .collect();

    *active = Some(ActiveRepoWatch {
        request_path: path,
        metadata_dir,
        workdir,
        worktree_metadata_paths,
        watched_paths: watch_paths.watched_paths,
    });
}

fn unwatch_paths(watcher: &mut RecommendedWatcher, watched_paths: &[PathBuf]) {
    for path in watched_paths {
        if let Err(error) = watcher.unwatch(path) {
            tracing::debug!(path = %path.display(), "failed to unwatch repository path: {error}");
        }
    }
}

fn should_consider_event(event: &Event) -> bool {
    match event.kind {
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => false,
        _ => true,
    }
}

fn is_ignored_metadata_event(active: &ActiveRepoWatch, event: &Event) -> bool {
    if active
        .metadata_dir
        .file_name()
        .and_then(|name| name.to_str())
        != Some(".jj")
    {
        return false;
    }

    event.paths.iter().all(|path| {
        path.starts_with(&active.metadata_dir)
            || active
                .workdir
                .as_ref()
                .is_some_and(|workdir| path == workdir)
    })
}

fn classify_event(active: &ActiveRepoWatch, event: &Event) -> RepositoryChangeKind {
    event
        .paths
        .iter()
        .map(|path| classify_path(active, path))
        .reduce(RepositoryChangeKind::merge)
        .unwrap_or(RepositoryChangeKind::Both)
}

fn classify_path(active: &ActiveRepoWatch, path: &Path) -> RepositoryChangeKind {
    if path.starts_with(&active.metadata_dir) {
        if is_worktree_metadata_path(active, path) {
            RepositoryChangeKind::Worktree
        } else {
            RepositoryChangeKind::Metadata
        }
    } else if active
        .workdir
        .as_ref()
        .is_some_and(|workdir| path.starts_with(workdir))
    {
        RepositoryChangeKind::Worktree
    } else {
        RepositoryChangeKind::Both
    }
}

fn is_worktree_metadata_path(active: &ActiveRepoWatch, path: &Path) -> bool {
    active
        .worktree_metadata_paths
        .iter()
        .any(|metadata_path| path == metadata_path || path.starts_with(metadata_path))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::event::{AccessKind, DataChange, MetadataKind, ModifyKind};
    use notify::{Event, EventKind};

    use super::should_consider_event;

    #[test]
    fn access_events_are_ignored() {
        let event = Event {
            kind: EventKind::Access(AccessKind::Read),
            paths: vec![PathBuf::from("/tmp/demo/.git/HEAD")],
            attrs: Default::default(),
        };

        assert!(!should_consider_event(&event));
    }

    #[test]
    fn access_time_metadata_events_are_ignored() {
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
            paths: vec![PathBuf::from("/tmp/demo/.git/HEAD")],
            attrs: Default::default(),
        };

        assert!(!should_consider_event(&event));
    }

    #[test]
    fn content_changes_are_considered() {
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/tmp/demo/src/lib.rs")],
            attrs: Default::default(),
        };

        assert!(should_consider_event(&event));
    }
}
