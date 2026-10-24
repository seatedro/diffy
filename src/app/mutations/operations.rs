use crate::contracts::commands::ContextMenuEntry;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::shell::Action;
use quark_ui::icons::lucide;

use crate::contracts::shared::{FileKey, SharedContext};
use crate::contracts::viewport::pipeline::ManifestFile;
use crate::core::vcs::model::{
    ChangeBucket, FileOperation, JjOperation, RepoCapabilities, VcsChange, VcsOperation,
    VcsOperationLogEntry,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RowAction {
    pub label: String,
    pub action: Action,
    pub destructive: bool,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    pub test_id: String,
}

impl RowAction {
    fn new(label: impl Into<String>, operation: VcsOperation, destructive: bool) -> Self {
        let label = label.into();
        Self {
            test_id: format!("mutations.op.{}", label.to_lowercase().replace(' ', "-")),
            label,
            action: MutationsAction::RunOperation(operation).into(),
            destructive,
            icon: None,
            shortcut: None,
        }
    }

    fn file(
        test_id: String,
        label: impl Into<String>,
        action: MutationsAction,
        icon: &'static str,
        destructive: bool,
    ) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
            destructive,
            icon: Some(icon),
            shortcut: None,
            test_id,
        }
    }

    fn with_shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }
}

fn manifest_entry(cx: &SharedContext, file: FileKey) -> Option<&ManifestFile> {
    cx.manifest
        .as_ref()
        .filter(|manifest| manifest.working_tree)?
        .files
        .iter()
        .find(|entry| entry.key == file)
}

fn staging(cx: &SharedContext) -> bool {
    cx.repository
        .as_ref()
        .is_some_and(|repo| repo.snapshot.capabilities.staging_area)
}

pub fn file_row_actions(entry: &ManifestFile, cx: &SharedContext) -> Vec<RowAction> {
    if manifest_entry(cx, entry.key).is_none() {
        return Vec::new();
    }
    let key = entry.key;
    let id = |name: &str| format!("mutations.row.{}.{name}", key.0);
    let mut actions: Vec<RowAction> = Vec::new();
    for bucket in &entry.buckets {
        let next = match bucket {
            ChangeBucket::Unstaged | ChangeBucket::Untracked if staging(cx) => vec![
                RowAction::file(
                    id("stage"),
                    "Stage file",
                    MutationsAction::StageFile(key),
                    lucide::PLUS,
                    false,
                )
                .with_shortcut("s"),
            ],
            ChangeBucket::Staged => vec![RowAction::file(
                id("unstage"),
                "Unstage file",
                MutationsAction::UnstageFile(key),
                lucide::MINUS,
                false,
            )],
            _ => Vec::new(),
        };
        actions.extend(next);
        let discard = match bucket {
            ChangeBucket::Unstaged | ChangeBucket::WorkingCopy => Some(RowAction::file(
                id("discard"),
                "Discard changes",
                MutationsAction::DiscardFile(key),
                lucide::CORNER_UP_LEFT,
                true,
            )),
            ChangeBucket::Untracked => Some(RowAction::file(
                id("delete"),
                "Delete file",
                MutationsAction::DiscardFile(key),
                lucide::X,
                true,
            )),
            _ => None,
        };
        actions.extend(discard);
    }
    let mut seen = Vec::new();
    actions.retain(|action| {
        let fresh = !seen.contains(&action.test_id);
        seen.push(action.test_id.clone());
        fresh
    });
    actions
}

pub fn group_action(bucket: ChangeBucket, cx: &SharedContext) -> Option<RowAction> {
    let working = cx
        .manifest
        .as_ref()
        .is_some_and(|manifest| manifest.working_tree);
    if !working || !staging(cx) {
        return None;
    }
    let (label, operation, icon) = match bucket {
        ChangeBucket::Unstaged | ChangeBucket::Untracked => {
            ("Stage all", FileOperation::Stage, lucide::PLUS)
        }
        ChangeBucket::Staged => ("Unstage all", FileOperation::Unstage, lucide::MINUS),
        _ => return None,
    };
    Some(RowAction::file(
        format!("mutations.group.{bucket:?}").to_lowercase(),
        label,
        MutationsAction::ApplyBucket { bucket, operation },
        icon,
        false,
    ))
}

pub fn selection_actions(files: &[FileKey], cx: &SharedContext) -> Vec<RowAction> {
    let entries: Vec<&ManifestFile> = files
        .iter()
        .filter_map(|file| manifest_entry(cx, *file))
        .collect();
    if entries.is_empty() || entries.len() != files.len() {
        return Vec::new();
    }
    let all = |wanted: &[ChangeBucket]| {
        entries
            .iter()
            .all(|entry| entry.buckets.iter().any(|bucket| wanted.contains(bucket)))
    };
    let count = files.len();
    let noun = if count == 1 { "file" } else { "files" };
    let mut actions = Vec::new();
    let apply = |operation| MutationsAction::ApplyFiles {
        files: files.to_vec(),
        operation,
    };
    if staging(cx) && all(&[ChangeBucket::Unstaged, ChangeBucket::Untracked]) {
        actions.push(
            RowAction::file(
                "mutations.selection.stage".to_owned(),
                format!("Stage {count} {noun}"),
                apply(FileOperation::Stage),
                lucide::PLUS,
                false,
            )
            .with_shortcut("s"),
        );
    }
    if staging(cx) && all(&[ChangeBucket::Staged]) {
        actions.push(RowAction::file(
            "mutations.selection.unstage".to_owned(),
            format!("Unstage {count} {noun}"),
            apply(FileOperation::Unstage),
            lucide::MINUS,
            false,
        ));
    }
    if all(&[
        ChangeBucket::Unstaged,
        ChangeBucket::Untracked,
        ChangeBucket::WorkingCopy,
    ]) {
        actions.push(RowAction::file(
            "mutations.selection.discard".to_owned(),
            format!("Discard {count} {noun}"),
            apply(FileOperation::Discard),
            lucide::CORNER_UP_LEFT,
            true,
        ));
    }
    actions
}

pub fn operation_log_actions(
    entry: &VcsOperationLogEntry,
    latest: bool,
    capabilities: RepoCapabilities,
) -> Vec<RowAction> {
    if !capabilities.operation_log {
        return Vec::new();
    }
    let mut actions = Vec::with_capacity(2);
    if latest {
        actions.push(RowAction::new(
            "Undo last operation",
            VcsOperation::Jj(JjOperation::UndoLastOperation),
            true,
        ));
    } else {
        actions.push(RowAction::new(
            format!("Restore to operation {}", entry.short_operation_id),
            VcsOperation::JjRestoreOperation {
                operation_id: entry.operation_id.clone(),
                label: entry.short_operation_id.clone(),
            },
            true,
        ));
    }
    actions
}

pub fn change_actions(change: &VcsChange, capabilities: RepoCapabilities) -> Vec<RowAction> {
    if !capabilities.operation_log || change.flags.current || change.flags.immutable {
        return Vec::new();
    }
    let revision = change
        .change_id
        .clone()
        .unwrap_or_else(|| change.revision.id.clone());
    let label = change
        .short_change_id
        .clone()
        .unwrap_or_else(|| change.short_revision.clone());
    vec![
        RowAction::new(
            format!("Edit change {label}"),
            VcsOperation::JjEditRevision {
                revision: revision.clone(),
                label: label.clone(),
            },
            false,
        ),
        RowAction::new(
            format!("Rebase current change onto {label}"),
            VcsOperation::JjRebaseCurrentChangeOnto {
                destination: revision,
            },
            false,
        ),
    ]
}

pub fn context_menu(actions: Vec<RowAction>) -> Vec<ContextMenuEntry> {
    actions
        .into_iter()
        .map(|row| ContextMenuEntry::Item {
            label: row.label,
            action: Box::new(row.action),
            shortcut: None,
            destructive: row.destructive,
            disabled: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::vcs::jj::service::jj_capabilities;
    use crate::core::vcs::model::{ChangeFlags, RevisionId, VcsKind};

    fn entry(id: &str) -> VcsOperationLogEntry {
        VcsOperationLogEntry {
            operation_id: format!("{id}full"),
            short_operation_id: id.to_owned(),
            user: "u".to_owned(),
            time: "t".to_owned(),
            description: "d".to_owned(),
        }
    }

    #[test]
    fn operation_rows_undo_latest_and_restore_older() {
        let caps = jj_capabilities(false);
        let latest = operation_log_actions(&entry("aa"), true, caps);
        assert_eq!(latest[0].label, "Undo last operation");
        let older = operation_log_actions(&entry("bb"), false, caps);
        assert_eq!(
            older[0].action,
            Action::Mutations(MutationsAction::RunOperation(
                VcsOperation::JjRestoreOperation {
                    operation_id: "bbfull".to_owned(),
                    label: "bb".to_owned(),
                }
            ))
        );
        assert!(operation_log_actions(&entry("cc"), false, RepoCapabilities::git()).is_empty());
        assert_eq!(context_menu(older).len(), 1);
    }

    #[test]
    fn change_rows_edit_and_rebase_mutable_changes() {
        let change = VcsChange {
            revision: RevisionId {
                backend: VcsKind::JJ,
                id: "c0ffee".to_owned(),
            },
            change_id: Some("kxyz".to_owned()),
            short_change_id: Some("kx".to_owned()),
            short_change_id_prefix_len: Some(2),
            short_revision: "c0f".to_owned(),
            summary: "s".to_owned(),
            author_name: "a".to_owned(),
            author_email: None,
            parents: Vec::new(),
            timestamp: 0,
            flags: ChangeFlags::default(),
        };
        let actions = change_actions(&change, jj_capabilities(false));
        assert_eq!(actions[0].label, "Edit change kx");
        assert_eq!(actions[1].label, "Rebase current change onto kx");
    }
}
