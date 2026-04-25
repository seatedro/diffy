use crate::actions::SyntaxAction;
use crate::effects::Effect;
use crate::events::SyntaxEvent;

use super::AppState;

pub(super) fn reduce_action(_state: &mut AppState, action: SyntaxAction) -> Vec<Effect> {
    match action {}
}

pub(super) fn reduce_event(state: &mut AppState, event: SyntaxEvent) -> Vec<Effect> {
    match event {
        SyntaxEvent::FileSyntaxReady(payload) => state.handle_file_syntax_ready(payload),
        SyntaxEvent::SyntaxPackInstallStarted { language } => {
            state.handle_syntax_pack_install_started(&language);
            Vec::new()
        }
        SyntaxEvent::SyntaxPackInstalled { language } => {
            state.handle_syntax_pack_installed(&language)
        }
        SyntaxEvent::SyntaxPackInstallFinished { language }
        | SyntaxEvent::SyntaxPackInstallFailed { language } => {
            state.handle_syntax_pack_install_finished(&language);
            Vec::new()
        }
    }
}
