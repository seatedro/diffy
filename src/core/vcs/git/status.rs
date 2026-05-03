use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusBits(u32);

impl StatusBits {
    pub const INDEX_NEW: Self = Self(1 << 0);
    pub const INDEX_MODIFIED: Self = Self(1 << 1);
    pub const INDEX_DELETED: Self = Self(1 << 2);
    pub const INDEX_RENAMED: Self = Self(1 << 3);
    pub const INDEX_TYPECHANGE: Self = Self(1 << 4);
    pub const WT_NEW: Self = Self(1 << 5);
    pub const WT_MODIFIED: Self = Self(1 << 6);
    pub const WT_DELETED: Self = Self(1 << 7);
    pub const WT_RENAMED: Self = Self(1 << 8);
    pub const WT_TYPECHANGE: Self = Self(1 << 9);
    pub const CONFLICTED: Self = Self(1 << 10);

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl std::ops::BitOr for StatusBits {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for StatusBits {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for StatusBits {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StatusScope {
    Staged,
    Unstaged,
    Untracked,
}

impl StatusScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusOperation {
    Stage,
    Unstage,
    Discard,
}

impl StatusOperation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Unstage => "unstage",
            Self::Discard => "discard",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusItem {
    pub path: String,
    pub old_path: Option<String>,
    pub scope: StatusScope,
    pub status: String,
}

pub fn status_items_from_entry(path: String, status: StatusBits) -> Vec<StatusItem> {
    status_items_from_entry_with_old_path(path, None, status)
}

pub fn status_items_from_entry_with_old_path(
    path: String,
    old_path: Option<String>,
    status: StatusBits,
) -> Vec<StatusItem> {
    let mut items = Vec::new();

    if status.contains(StatusBits::INDEX_NEW) {
        items.push(item(path.clone(), None, StatusScope::Staged, "A"));
    } else if status.intersects(
        StatusBits::INDEX_MODIFIED | StatusBits::INDEX_TYPECHANGE | StatusBits::CONFLICTED,
    ) {
        items.push(item(path.clone(), None, StatusScope::Staged, "M"));
    } else if status.contains(StatusBits::INDEX_DELETED) {
        items.push(item(path.clone(), None, StatusScope::Staged, "D"));
    } else if status.contains(StatusBits::INDEX_RENAMED) {
        items.push(item(
            path.clone(),
            old_path.clone(),
            StatusScope::Staged,
            "R",
        ));
    }

    if status.contains(StatusBits::WT_NEW) {
        items.push(item(path.clone(), None, StatusScope::Untracked, "U"));
    } else if status
        .intersects(StatusBits::WT_MODIFIED | StatusBits::WT_TYPECHANGE | StatusBits::CONFLICTED)
    {
        items.push(item(path.clone(), None, StatusScope::Unstaged, "M"));
    } else if status.contains(StatusBits::WT_DELETED) {
        items.push(item(path.clone(), None, StatusScope::Unstaged, "D"));
    } else if status.contains(StatusBits::WT_RENAMED) {
        items.push(item(path, old_path, StatusScope::Unstaged, "R"));
    }

    items.sort_by(|left, right| {
        scope_sort_key(left.scope)
            .cmp(&scope_sort_key(right.scope))
            .then(left.path.cmp(&right.path))
    });
    items
}

fn item(path: String, old_path: Option<String>, scope: StatusScope, status: &str) -> StatusItem {
    StatusItem {
        path,
        old_path,
        scope,
        status: status.to_owned(),
    }
}

fn scope_sort_key(scope: StatusScope) -> u8 {
    match scope {
        StatusScope::Staged => 0,
        StatusScope::Unstaged => 1,
        StatusScope::Untracked => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StatusBits, StatusScope, status_items_from_entry, status_items_from_entry_with_old_path,
    };

    #[test]
    fn flattens_staged_and_unstaged_entries() {
        let items = status_items_from_entry(
            "src/lib.rs".to_owned(),
            StatusBits::INDEX_MODIFIED | StatusBits::WT_MODIFIED,
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].scope, StatusScope::Staged);
        assert_eq!(items[1].scope, StatusScope::Unstaged);
    }

    #[test]
    fn maps_untracked_entries() {
        let items = status_items_from_entry("src/new.rs".to_owned(), StatusBits::WT_NEW);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].scope, StatusScope::Untracked);
        assert_eq!(items[0].status, "U");
    }

    #[test]
    fn keeps_old_path_on_rename_entries() {
        let items = status_items_from_entry_with_old_path(
            "src/new.rs".to_owned(),
            Some("src/old.rs".to_owned()),
            StatusBits::INDEX_RENAMED,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].scope, StatusScope::Staged);
        assert_eq!(items[0].status, "R");
        assert_eq!(items[0].old_path.as_deref(), Some("src/old.rs"));
    }
}
