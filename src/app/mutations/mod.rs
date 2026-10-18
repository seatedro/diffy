mod controls;
mod operations;
mod presentation;
mod settings;
mod state;
#[cfg(test)]
mod tests;
mod view;

pub use controls::{Chrome, Control, Tone};
pub use operations::{
    RowAction, change_actions, context_menu, file_row_actions, group_action, operation_log_actions,
    selection_actions,
};
pub use presentation::{MutationPresentation, PresentationStyle};
pub use settings::clankers_settings;
pub use state::{LineKey, MutationsController, working_tree};
pub use view::{commit_box, panel, selection_bar, view};
