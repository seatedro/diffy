use std::hash::{DefaultHasher, Hasher};
use std::path::{Component, Path, PathBuf};

use crate::core::error::{DiffyError, Result};

const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SavedFile {
    path: PathBuf,
    before: Option<Vec<u8>>,
    mode: Option<u32>,
    after: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeSnapshot {
    root: PathBuf,
    files: Vec<SavedFile>,
}

pub struct PendingSnapshot {
    root: PathBuf,
    files: Vec<SavedFile>,
}

fn digest(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

fn relative(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then(|| path.to_path_buf())
}

#[cfg(unix)]
fn mode_of(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn mode_of(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match mode {
        Some(mode) => std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)),
        None => Ok(()),
    }
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}

fn read_current(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(DiffyError::io(path, "read", error)),
    }
}

impl PendingSnapshot {
    pub fn capture<'a>(root: &Path, paths: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let mut files = Vec::new();
        let mut total = 0usize;
        for path in paths {
            let path = relative(path)?;
            if files.iter().any(|file: &SavedFile| file.path == path) {
                continue;
            }
            let absolute = root.join(&path);
            let metadata = std::fs::symlink_metadata(&absolute).ok();
            if metadata.as_ref().is_some_and(|m| !m.is_file()) {
                return None;
            }
            let before = read_current(&absolute).ok()?;
            total += before.as_ref().map_or(0, Vec::len);
            if total > MAX_SNAPSHOT_BYTES {
                return None;
            }
            files.push(SavedFile {
                path,
                before,
                mode: metadata.as_ref().and_then(mode_of),
                after: None,
            });
        }
        Some(Self {
            root: root.to_path_buf(),
            files,
        })
    }

    pub fn finish(mut self) -> Option<WorktreeSnapshot> {
        for file in &mut self.files {
            file.after = read_current(&self.root.join(&file.path))
                .ok()?
                .map(|bytes| digest(&bytes));
        }
        let changed = self
            .files
            .iter()
            .any(|file| file.before.as_deref().map(digest) != file.after);
        changed.then(|| WorktreeSnapshot {
            root: self.root,
            files: self.files,
        })
    }
}

impl WorktreeSnapshot {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|file| file.path.as_path())
    }

    pub fn restore(&self) -> Result<()> {
        for file in &self.files {
            let absolute = self.root.join(&file.path);
            let current = read_current(&absolute)?.map(|bytes| digest(&bytes));
            if current != file.after {
                return Err(DiffyError::General(format!(
                    "{} changed after the discard, so it was not restored.",
                    file.path.display()
                )));
            }
        }
        for file in &self.files {
            let absolute = self.root.join(&file.path);
            match &file.before {
                Some(bytes) => {
                    if let Some(parent) = absolute.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|error| DiffyError::io(parent, "create", error))?;
                    }
                    std::fs::write(&absolute, bytes)
                        .map_err(|error| DiffyError::io(&absolute, "write", error))?;
                    set_mode(&absolute, file.mode)
                        .map_err(|error| DiffyError::io(&absolute, "set mode of", error))?;
                }
                None => match std::fs::remove_file(&absolute) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(DiffyError::io(&absolute, "remove", error)),
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_exact_bytes_only_when_untouched() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a"), b"x\r\n\xff").unwrap();
        let pending = PendingSnapshot::capture(root, ["a", "gone"]).unwrap();
        std::fs::write(root.join("a"), b"y\n").unwrap();
        std::fs::write(root.join("gone"), b"new").unwrap();
        let snapshot = pending.finish().unwrap();
        snapshot.restore().unwrap();
        assert_eq!(std::fs::read(root.join("a")).unwrap(), b"x\r\n\xff");
        assert!(!root.join("gone").exists());

        let pending = PendingSnapshot::capture(root, ["a"]).unwrap();
        std::fs::write(root.join("a"), b"z\n").unwrap();
        let snapshot = pending.finish().unwrap();
        std::fs::write(root.join("a"), b"edited\n").unwrap();
        assert!(snapshot.restore().is_err());
        assert_eq!(std::fs::read(root.join("a")).unwrap(), b"edited\n");
    }

    #[test]
    fn rejects_paths_outside_the_root() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(PendingSnapshot::capture(dir.path(), ["../escape"]).is_none());
    }
}
