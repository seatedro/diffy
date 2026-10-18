use crate::contracts::commands::ContextMenuEntry;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::shell::Action;
use crate::core::vcs::model::{
    JjOperation, RepoCapabilities, VcsChange, VcsOperation, VcsOperationLogEntry,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RowAction {
    pub label: String,
    pub action: Action,
    pub destructive: bool,
}

impl RowAction {
    fn new(label: impl Into<String>, operation: VcsOperation, destructive: bool) -> Self {
        Self {
            label: label.into(),
            action: MutationsAction::RunOperation(operation).into(),
            destructive,
        }
    }
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
            timestamp: 0,
            flags: ChangeFlags::default(),
        };
        let actions = change_actions(&change, jj_capabilities(false));
        assert_eq!(actions[0].label, "Edit change kx");
        assert_eq!(actions[1].label, "Rebase current change onto kx");
    }
}
