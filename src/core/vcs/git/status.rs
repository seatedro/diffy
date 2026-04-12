use git2::Status;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StatusScope {
    Staged,
    Unstaged,
    Untracked,
}

impl StatusScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Unstaged => "unstaged",
            Self::Untracked => "untracked",
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
    pub scope: StatusScope,
    pub status: String,
}

pub fn status_items_from_entry(path: String, status: Status) -> Vec<StatusItem> {
    let mut items = Vec::new();

    if status.contains(Status::INDEX_NEW) {
        items.push(item(path.clone(), StatusScope::Staged, "A"));
    } else if status
        .intersects(Status::INDEX_MODIFIED | Status::INDEX_TYPECHANGE | Status::CONFLICTED)
    {
        items.push(item(path.clone(), StatusScope::Staged, "M"));
    } else if status.contains(Status::INDEX_DELETED) {
        items.push(item(path.clone(), StatusScope::Staged, "D"));
    } else if status.contains(Status::INDEX_RENAMED) {
        items.push(item(path.clone(), StatusScope::Staged, "R"));
    }

    if status.contains(Status::WT_NEW) {
        items.push(item(path.clone(), StatusScope::Untracked, "U"));
    } else if status.intersects(Status::WT_MODIFIED | Status::WT_TYPECHANGE | Status::CONFLICTED) {
        items.push(item(path.clone(), StatusScope::Unstaged, "M"));
    } else if status.contains(Status::WT_DELETED) {
        items.push(item(path.clone(), StatusScope::Unstaged, "D"));
    } else if status.contains(Status::WT_RENAMED) {
        items.push(item(path, StatusScope::Unstaged, "R"));
    }

    items.sort_by(|left, right| {
        scope_sort_key(left.scope)
            .cmp(&scope_sort_key(right.scope))
            .then(left.path.cmp(&right.path))
    });
    items
}

fn item(path: String, scope: StatusScope, status: &str) -> StatusItem {
    StatusItem {
        path,
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
    use git2::Status;

    use super::{StatusScope, status_items_from_entry};

    #[test]
    fn flattens_staged_and_unstaged_entries() {
        let items = status_items_from_entry(
            "src/lib.rs".to_owned(),
            Status::INDEX_MODIFIED | Status::WT_MODIFIED,
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].scope, StatusScope::Staged);
        assert_eq!(items[1].scope, StatusScope::Unstaged);
    }

    #[test]
    fn maps_untracked_entries() {
        let items = status_items_from_entry("src/new.rs".to_owned(), Status::WT_NEW);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].scope, StatusScope::Untracked);
        assert_eq!(items[0].status, "U");
    }
}
