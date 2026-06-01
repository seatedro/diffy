use crate::actions::WorkspaceAction;
use crate::effects::{Effect, RepositoryEffect};
use crate::events::RepositorySyncReason;

use super::*;

pub(super) fn reduce_action(state: &mut AppState, action: WorkspaceAction) -> Vec<Effect> {
    state.apply_workspace_action(action)
}

impl AppState {
    pub(super) fn apply_workspace_action(&mut self, action: WorkspaceAction) -> Vec<Effect> {
        match action {
            WorkspaceAction::OpenRepository(path) => self.open_repository(path),
            WorkspaceAction::NewTextCompare => self.new_text_compare(),
            WorkspaceAction::ShowWorkingTree => self.show_working_tree(),
            WorkspaceAction::RefreshRepository => self.refresh_repository(),
        }
    }

    fn refresh_repository(&mut self) -> Vec<Effect> {
        let Some(path) = self.compare.repo_path.get(&self.store) else {
            self.push_error("Open a repository before refreshing.");
            return Vec::new();
        };
        if self.workspace.source.get(&self.store) == WorkspaceSource::Compare {
            self.kickoff_compare()
        } else {
            vec![
                RepositoryEffect::SyncRepository {
                    path,
                    reason: RepositorySyncReason::Rescan,
                    reporter_generation: None,
                }
                .into(),
            ]
        }
    }
}
