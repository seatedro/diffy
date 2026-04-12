use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use git2::Repository;
use notify::event::{EventKind, MetadataKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::apprt::git_worker::GitWorkerCommand;

const REPO_WATCH_DEBOUNCE: Duration = Duration::from_millis(150);

pub struct RepoWatchWorker {
    sender: Sender<WatchCommand>,
}

impl RepoWatchWorker {
    pub fn new(dirty_sender: Sender<GitWorkerCommand>) -> Self {
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
    watched_paths: Vec<PathBuf>,
}

fn repo_watch_worker_loop(
    callback_sender: Sender<WatchCommand>,
    dirty_sender: Sender<GitWorkerCommand>,
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
    let mut pending_dirty = false;

    loop {
        let command = if pending_dirty {
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
                pending_dirty = false;
                replace_active_watch(&mut watcher, &mut active, path);
            }
            Some(WatchCommand::Notify(Ok(event))) => {
                if active.is_some() && should_consider_event(&event) {
                    pending_dirty = true;
                }
            }
            Some(WatchCommand::Notify(Err(error))) => {
                tracing::warn!("repository watcher error: {error}");
                pending_dirty = true;
            }
            None => {
                let Some(active_watch) = active.as_ref() else {
                    pending_dirty = false;
                    continue;
                };
                let _ = dirty_sender.send(GitWorkerCommand::Dirty {
                    path: active_watch.request_path.clone(),
                });
                pending_dirty = false;
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

    let watched_paths = match watch_paths_for_repo(&path) {
        Ok(paths) => paths,
        Err(error) => {
            tracing::warn!(
                repo = %path.display(),
                "failed to resolve repository watch paths: {error}"
            );
            return;
        }
    };

    for watch_path in &watched_paths {
        if let Err(error) = watcher.watch(watch_path, RecursiveMode::Recursive) {
            tracing::warn!(
                path = %watch_path.display(),
                "failed to watch repository path: {error}"
            );
        }
    }

    *active = Some(ActiveRepoWatch {
        request_path: path,
        watched_paths,
    });
}

fn watch_paths_for_repo(path: &Path) -> Result<Vec<PathBuf>, git2::Error> {
    let repo = Repository::open(path)?;
    let git_dir = repo.path().to_path_buf();
    let workdir = repo.workdir().map(Path::to_path_buf);

    Ok(match workdir {
        Some(workdir) if git_dir.starts_with(&workdir) => vec![workdir],
        Some(workdir) => vec![workdir, git_dir],
        None => vec![git_dir],
    })
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
