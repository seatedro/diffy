use quark_ui::FocusId;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use crate::app::route::Controllers;
use crate::app::viewport::ViewportController;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{LogicalFocus, Reduction};

pub const VIEWPORT: FocusId = FocusId::from_key("diffy.viewport");
pub const SIDEBAR: FocusId = FocusId::from_key("diffy.sidebar");
pub const SIDEBAR_FILTER: FocusId = FocusId::from_key("diffy.sidebar.filter");
pub const SEARCH: FocusId = FocusId::from_key("diffy.viewport.search");
pub const PALETTE: FocusId = quark_components::PALETTE_INPUT;
pub const PICKER: FocusId = FocusId::from_key("diffy.picker");
pub const SETTINGS: FocusId = FocusId::from_key("diffy.settings");
pub const COMMIT_MESSAGE: FocusId = FocusId::from_key("diffy.commit.message");
pub const REVIEW_COMPOSER: FocusId = FocusId::from_key("diffy.review.composer");
pub const TEXT_COMPARE_LEFT: FocusId = FocusId::from_key("diffy.text_compare.left");
pub const TEXT_COMPARE_RIGHT: FocusId = FocusId::from_key("diffy.text_compare.right");
pub const SETTINGS_OPENAI_KEY: FocusId = FocusId::from_key("diffy.settings.openai_key");
pub const SETTINGS_ANTHROPIC_KEY: FocusId = FocusId::from_key("diffy.settings.anthropic_key");
pub const SETTINGS_STEERING_PROMPT: FocusId = FocusId::from_key("diffy.settings.steering_prompt");
pub const REVIEW_ACCOUNT: FocusId = FocusId::from_key("diffy.review.account");

pub fn focus_id(focus: LogicalFocus) -> FocusId {
    match focus {
        LogicalFocus::Viewport => VIEWPORT,
        LogicalFocus::Sidebar => SIDEBAR,
        LogicalFocus::SidebarFilter => SIDEBAR_FILTER,
        LogicalFocus::Search => SEARCH,
        LogicalFocus::Palette => PALETTE,
        LogicalFocus::Picker => PICKER,
        LogicalFocus::Settings => SETTINGS,
        LogicalFocus::CommitMessage => COMMIT_MESSAGE,
        LogicalFocus::ReviewComposer => REVIEW_COMPOSER,
        LogicalFocus::TextCompareLeft => TEXT_COMPARE_LEFT,
        LogicalFocus::TextCompareRight => TEXT_COMPARE_RIGHT,
        LogicalFocus::SettingsOpenAiKey => SETTINGS_OPENAI_KEY,
        LogicalFocus::SettingsAnthropicKey => SETTINGS_ANTHROPIC_KEY,
        LogicalFocus::SettingsSteeringPrompt => SETTINGS_STEERING_PROMPT,
        LogicalFocus::ReviewAccount => REVIEW_ACCOUNT,
    }
}

pub struct TextCx<'a> {
    pub shared: &'a SharedContext,
    pub now_ms: u64,
}

pub trait TextHost {
    fn owns_text(&self, _target: FocusId) -> bool {
        false
    }

    fn edit_text(
        &mut self,
        _target: FocusId,
        _command: TextEditCommand,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        None
    }

    fn set_preedit(
        &mut self,
        _target: FocusId,
        _text: String,
        _cursor: Option<(usize, usize)>,
    ) -> bool {
        false
    }

    fn set_text_value(
        &mut self,
        _target: FocusId,
        _value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        None
    }

    fn after_edit(&mut self, _target: FocusId, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }
}

impl TextHost for ViewportController {
    fn owns_text(&self, target: FocusId) -> bool {
        self.interaction.owns_text(target)
    }

    fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        self.interaction.edit_text(target, command, cx)
    }

    fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        self.interaction.set_preedit(target, text, cursor)
    }

    fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        self.interaction.set_text_value(target, value, cx)
    }

    fn after_edit(&mut self, target: FocusId, cx: &SharedContext) -> Reduction {
        self.interaction.after_edit(target, cx)
    }
}

impl Controllers {
    fn text_owner(&mut self, target: FocusId) -> Option<&mut dyn TextHost> {
        let hosts: [&mut dyn TextHost; 6] = [
            &mut self.commands,
            &mut self.navigation,
            &mut self.viewport,
            &mut self.review,
            &mut self.mutations,
            &mut self.delivery,
        ];
        hosts.into_iter().find(|host| host.owns_text(target))
    }

    pub fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        self.text_owner(target)?.edit_text(target, command, cx)
    }

    pub fn after_edit(&mut self, target: FocusId, cx: &SharedContext) -> Reduction {
        self.text_owner(target)
            .map(|host| host.after_edit(target, cx))
            .unwrap_or_default()
    }

    pub fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        self.text_owner(target)
            .is_some_and(|host| host.set_preedit(target, text, cursor))
    }

    pub fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        self.text_owner(target)?.set_text_value(target, value, cx)
    }
}
