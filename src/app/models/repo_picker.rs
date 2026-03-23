use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use git2::{Repository, RepositoryOpenFlags};
use qmetaobject::*;

#[derive(Clone, Default, SimpleListItem)]
pub struct RepositoryPickerEntry {
    pub name: QString,
    pub path: QString,
    pub is_directory: bool,
    pub is_git_repo: bool,
}

#[derive(Default, QObject)]
pub struct RepositoryPickerModel {
    base: qt_base_class!(trait QAbstractListModel),

    current_path: qt_property!(QString; READ get_current_path NOTIFY current_path_changed),
    current_path_changed: qt_signal!(),

    current_path_is_repository: qt_property!(bool; READ get_current_path_is_repository NOTIFY current_path_is_repository_changed),
    current_path_is_repository_changed: qt_signal!(),

    entries: Vec<RepositoryPickerEntry>,
    current_path_value: QString,
    current_path_is_repository_value: bool,
}

impl RepositoryPickerModel {
    pub fn get_current_path(&self) -> QString {
        self.current_path_value.clone()
    }

    pub fn get_current_path_is_repository(&self) -> bool {
        self.current_path_is_repository_value
    }

    pub fn set_current_path(&mut self, path: QString) {
        let normalized = normalize_path(&path.to_string());
        let next = QString::from(normalized.to_string_lossy().to_string());
        if self.current_path_value == next && !self.entries.is_empty() {
            return;
        }
        self.current_path_value = next;
        self.reload();
        self.current_path_changed();
    }

    pub fn go_up(&mut self) -> bool {
        let current = PathBuf::from(self.current_path_value.to_string());
        let Some(parent) = current.parent() else {
            return false;
        };
        self.set_current_path(QString::from(parent.to_string_lossy().to_string()));
        true
    }

    pub fn navigate_to_entry(&mut self, index: i32) -> bool {
        if index < 0 || index as usize >= self.entries.len() {
            return false;
        }
        let entry = self.entries[index as usize].path.clone();
        self.set_current_path(entry);
        true
    }

    pub fn entry_path(&self, index: i32) -> QString {
        if index < 0 || index as usize >= self.entries.len() {
            return QString::default();
        }
        self.entries[index as usize].path.clone()
    }

    pub fn entry_is_repository(&self, index: i32) -> bool {
        if index < 0 || index as usize >= self.entries.len() {
            return false;
        }
        self.entries[index as usize].is_git_repo
    }

    fn reload(&mut self) {
        let path = PathBuf::from(self.current_path_value.to_string());
        let mut next_entries = Vec::new();

        if let Ok(read_dir) = fs::read_dir(&path) {
            let mut dirs = read_dir
                .flatten()
                .filter_map(|entry| {
                    let file_type = entry.file_type().ok()?;
                    if !file_type.is_dir() {
                        return None;
                    }
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    Some(RepositoryPickerEntry {
                        name: QString::from(name),
                        path: QString::from(path.to_string_lossy().to_string()),
                        is_directory: true,
                        is_git_repo: is_repository_root(&path),
                    })
                })
                .collect::<Vec<_>>();
            dirs.sort_by(|left, right| {
                left.name
                    .to_string()
                    .to_lowercase()
                    .cmp(&right.name.to_string().to_lowercase())
            });
            next_entries = dirs;
        }

        (self as &mut dyn QAbstractListModel).begin_reset_model();
        self.entries = next_entries;
        (self as &mut dyn QAbstractListModel).end_reset_model();

        let next_is_repo = is_repository_root(&path);
        if self.current_path_is_repository_value != next_is_repo {
            self.current_path_is_repository_value = next_is_repo;
            self.current_path_is_repository_changed();
        }
    }
}

impl QAbstractListModel for RepositoryPickerModel {
    fn row_count(&self) -> i32 {
        self.entries.len() as i32
    }

    fn data(&self, index: QModelIndex, role: i32) -> QVariant {
        let row = index.row();
        if row < 0 || row as usize >= self.entries.len() {
            return QVariant::default();
        }
        let entry = &self.entries[row as usize];
        match role {
            USER_ROLE => entry.name.clone().into(),
            value if value == USER_ROLE + 1 => entry.path.clone().into(),
            value if value == USER_ROLE + 2 => entry.is_git_repo.into(),
            value if value == USER_ROLE + 3 => entry.is_directory.into(),
            _ => QVariant::default(),
        }
    }

    fn role_names(&self) -> HashMap<i32, QByteArray> {
        HashMap::from([
            (USER_ROLE, QByteArray::from("name")),
            (USER_ROLE + 1, QByteArray::from("path")),
            (USER_ROLE + 2, QByteArray::from("isRepository")),
            (USER_ROLE + 3, QByteArray::from("isDirectory")),
        ])
    }
}

fn normalize_path(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty() {
        std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    } else if path.exists() {
        path.canonicalize().unwrap_or(path)
    } else {
        path
    }
}

fn is_repository_root(path: &Path) -> bool {
    Repository::open_ext(
        path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&std::ffi::OsStr>(),
    )
    .is_ok()
}
