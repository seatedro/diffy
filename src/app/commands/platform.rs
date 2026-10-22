use quark_app::platform::menu::{Menu, MenuAction, MenuItem, MenuRole};
use quark_app::{InputEvent, UiContext, WindowOptions};
use quark_ui::FocusId;
use quark_ui::accessibility::Politeness;
use quark_ui::key_context::KeyBindings;

use super::state::{CommandsController, LayerKind};
use crate::app::route::Controllers;
use crate::app::text_targets::{
    COMMIT_MESSAGE, LAUNCHER, REVIEW_COMPOSER, SEARCH, SETTINGS_STEERING_PROMPT, SIDEBAR_FILTER,
    TEXT_COMPARE_LEFT, TEXT_COMPARE_RIGHT,
};
use crate::contracts::commands::{CommandDescriptor, CommandsAction, RefPickerAction};
use crate::contracts::navigation::NavigationAction;
use crate::contracts::review::ReviewAction;
use crate::contracts::shell::Action;
use crate::fonts::FontSettings;

pub enum PlatformRequest {
    Focus(Option<FocusId>),
    Theme(Box<crate::app::theme::ThemeSelection>),
    Fonts(FontSettings),
    KeyBindings(KeyBindings),
    Menus(Vec<Menu>),
    Dispatch(Action),
    Announce(String),
    Quit,
}

impl std::fmt::Debug for PlatformRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Focus(focus) => f.debug_tuple("Focus").field(focus).finish(),
            Self::Theme(_) => f.write_str("Theme"),
            Self::Fonts(fonts) => f.debug_tuple("Fonts").field(fonts).finish(),
            Self::KeyBindings(_) => f.write_str("KeyBindings"),
            Self::Menus(menus) => f.debug_tuple("Menus").field(&menus.len()).finish(),
            Self::Dispatch(action) => f.debug_tuple("Dispatch").field(action).finish(),
            Self::Announce(text) => f.debug_tuple("Announce").field(text).finish(),
            Self::Quit => f.write_str("Quit"),
        }
    }
}

pub fn take_requests(controllers: &Controllers) -> Vec<PlatformRequest> {
    let mut requests = controllers.commands.take_platform();
    requests.extend(
        controllers
            .navigation
            .take_dispatch()
            .into_iter()
            .map(PlatformRequest::Dispatch),
    );
    requests
}

pub fn apply(requests: Vec<PlatformRequest>, cx: &mut UiContext) -> Vec<Action> {
    let mut followups = Vec::new();
    for request in requests {
        match request {
            PlatformRequest::Focus(focus) => cx.set_focus(focus),
            PlatformRequest::Theme(selection) => match *selection {
                crate::app::theme::ThemeSelection::Fixed(theme) => cx.set_theme(theme),
                crate::app::theme::ThemeSelection::System(pair) => {
                    cx.set_themes(pair.light, pair.dark)
                }
            },
            PlatformRequest::Fonts(fonts) => {
                let mut quark_fonts = WindowOptions::default().fonts;
                quark_fonts.ui_family = fonts.ui_family;
                quark_fonts.mono_family = fonts.mono_family;
                cx.set_fonts(&quark_fonts);
            }
            PlatformRequest::KeyBindings(bindings) => *cx.key_bindings_mut() = bindings,
            PlatformRequest::Menus(menus) => cx.window.set_menus(menus),
            PlatformRequest::Dispatch(action) => followups.push(action),
            PlatformRequest::Announce(text) => cx.announce(text, Politeness::Polite),
            PlatformRequest::Quit => cx.window.exit(),
        }
    }
    followups
}

pub fn key_action(
    controllers: &Controllers,
    event: &InputEvent,
    focus: Option<FocusId>,
) -> Option<Action> {
    if let InputEvent::ModifiersChanged(modifiers) = event {
        controllers
            .navigation
            .observe_modifiers(crate::app::navigation::ClickModifiers {
                extend: modifiers.shift_key(),
                toggle: modifiers.super_key() || modifiers.control_key(),
            });
        return None;
    }
    if let InputEvent::Focused(focused) = event {
        controllers.commands.observe_window_focus(*focused);
        return None;
    }
    let InputEvent::KeyPress(chord) = event else {
        return None;
    };
    controllers.commands.observe_key();
    let binding = chord.binding()?.to_string();
    if let Some(action) = controllers.commands.key_action(&binding) {
        return Some(action);
    }
    if focus == Some(LAUNCHER)
        && controllers.commands.launcher_active()
        && controllers.commands.top_layer().is_none()
        && !controllers.commands.settings_open()
        && !controllers.review.is_modal_open()
        && let Some(action) = launcher_key(&binding)
    {
        return Some(action.into());
    }
    let quiet = controllers.commands.top_layer().is_none()
        && !controllers.commands.settings_open()
        && !controllers.review.is_modal_open()
        && !focus.is_some_and(|f| TEXT_FIELDS.contains(&f));
    if binding == "escape"
        && quiet
        && controllers.review.overview_open()
        && controllers.review.composer().is_none()
    {
        return Some(ReviewAction::CloseOverview.into());
    }
    let leaves_commit = binding == "escape"
        && quiet
        && controllers.navigation.viewing_commit()
        && !controllers.viewport.interaction.search().open
        && !controllers.viewport.interaction.has_text_selection()
        && controllers.mutations.selected_lines().is_empty();
    leaves_commit.then(|| NavigationAction::ClearHistoryCommit.into())
}

fn launcher_key(binding: &str) -> Option<CommandsAction> {
    Some(match binding {
        "arrowdown" | "ctrl+n" => CommandsAction::LauncherMove(1),
        "arrowup" | "ctrl+p" => CommandsAction::LauncherMove(-1),
        "enter" => CommandsAction::LauncherConfirm,
        "escape" => CommandsAction::LauncherClear,
        _ => return None,
    })
}

const TEXT_FIELDS: [FocusId; 7] = [
    SEARCH,
    SIDEBAR_FILTER,
    COMMIT_MESSAGE,
    REVIEW_COMPOSER,
    TEXT_COMPARE_LEFT,
    TEXT_COMPARE_RIGHT,
    SETTINGS_STEERING_PROMPT,
];

pub fn menu_action(controllers: &Controllers, id: &str) -> Option<Action> {
    controllers.commands.menu_action(id)
}

pub fn menus(
    commands: &[CommandDescriptor],
    bindings: impl Fn(&CommandDescriptor) -> Option<String>,
) -> Vec<Menu> {
    let item = |id: &str| -> MenuItem {
        let Some(command) = commands.iter().find(|c| c.id == id) else {
            return MenuItem::Separator;
        };
        let mut action =
            MenuAction::new(command.id, command.title.clone()).enabled(command.enabled);
        if let Some(binding) = bindings(command) {
            action = action.shortcut(&binding);
        }
        action.into()
    };
    let role = MenuItem::from;
    let name = crate::platform::startup::app_display_name();
    vec![
        Menu::new(
            name,
            vec![
                item(ids::ABOUT),
                MenuItem::Separator,
                item(ids::SETTINGS),
                MenuItem::Separator,
                role(MenuRole::Services),
                MenuItem::Separator,
                role(MenuRole::Hide),
                role(MenuRole::HideOthers),
                role(MenuRole::ShowAll),
                MenuItem::Separator,
                item(ids::QUIT),
            ],
        ),
        Menu::new(
            "File",
            vec![
                item(ids::OPEN_REPOSITORY),
                item(ids::GO_TO_FILE),
                item(ids::GO_TO_SYMBOL),
                item(ids::GO_TO_WORKSPACE_SYMBOL),
                MenuItem::Separator,
                item(ids::WORKING_TREE),
                item(ids::REFRESH),
                MenuItem::Separator,
                item(ids::CLOSE),
            ],
        ),
        Menu::edit(),
        Menu::new(
            "View",
            vec![
                item(ids::PALETTE),
                MenuItem::Separator,
                item(ids::TOGGLE_SIDEBAR),
                item(ids::TOGGLE_FILE_TREE),
                item(ids::TOGGLE_WRAP),
                item(ids::TOGGLE_CONTINUOUS),
                MenuItem::Separator,
                item(ids::TOGGLE_THEME_MODE),
                item(ids::THEME),
                MenuItem::Separator,
                item(ids::SCALE_UP),
                item(ids::SCALE_DOWN),
                item(ids::SCALE_RESET),
            ],
        ),
        Menu::new(
            "Window",
            vec![
                role(MenuRole::Minimize),
                role(MenuRole::Zoom),
                MenuItem::Separator,
                role(MenuRole::BringAllToFront),
            ],
        ),
        Menu::new("Help", vec![item(ids::KEYMAPS), item(ids::WEBSITE)]),
    ]
}

pub mod ids {
    pub const ABOUT: &str = "app.about";
    pub const SETTINGS: &str = "settings.open";
    pub const KEYMAPS: &str = "settings.keymaps";
    pub const QUIT: &str = "app.quit";
    pub const WEBSITE: &str = "app.website";
    pub const PALETTE: &str = "palette.open";
    pub const OPEN_REPOSITORY: &str = "repo.open";
    pub const GO_TO_FILE: &str = "file.go_to";
    pub const CLOSE: &str = "file.close";
    pub const GO_TO_SYMBOL: &str = "symbol.go_to";
    pub const GO_TO_WORKSPACE_SYMBOL: &str = "symbol.workspace";
    pub const WORKING_TREE: &str = "repo.working_tree";
    pub const REFRESH: &str = "repo.refresh";
    pub const TOGGLE_SIDEBAR: &str = "view.sidebar";
    pub const TOGGLE_FILE_TREE: &str = "view.file_tree";
    pub const TOGGLE_WRAP: &str = "view.wrap";
    pub const TOGGLE_CONTINUOUS: &str = "view.continuous";
    pub const TOGGLE_THEME_MODE: &str = "theme.mode";
    pub const THEME: &str = "theme.pick";
    pub const SCALE_UP: &str = "scale.up";
    pub const SCALE_DOWN: &str = "scale.down";
    pub const SCALE_RESET: &str = "scale.reset";
}

impl CommandsController {
    pub(super) fn key_action(&self, binding: &str) -> Option<Action> {
        if self.go_pending() {
            return Some(CommandsAction::Leader(binding.to_owned()).into());
        }
        if let Some(command) = self.capturing() {
            return Some(
                match binding {
                    "escape" => CommandsAction::CancelKeymapRebind,
                    binding => CommandsAction::ApplyKeymapBinding {
                        command,
                        binding: binding.to_owned(),
                    },
                }
                .into(),
            );
        }
        let action = match self.top_layer() {
            Some(LayerKind::Palette) => {
                palette_key(binding).then(|| CommandsAction::OverlayKey(binding.to_owned()))
            }
            Some(LayerKind::ContextMenu) => Some(CommandsAction::OverlayKey(binding.to_owned())),
            Some(LayerKind::Picker) => match binding {
                "escape" => Some(CommandsAction::CloseOverlay),
                "enter" => Some(CommandsAction::ConfirmOverlaySelection),
                "arrowdown" | "ctrl+n" => Some(CommandsAction::MoveOverlaySelection(1)),
                "arrowup" | "ctrl+p" => Some(CommandsAction::MoveOverlaySelection(-1)),
                "pagedown" => Some(CommandsAction::MoveOverlaySelection(8)),
                "pageup" => Some(CommandsAction::MoveOverlaySelection(-8)),
                "tab" if self.picker_completes_paths() => {
                    Some(CommandsAction::TabCompletePickerDir)
                }
                "arrowright" if self.picker_browses_repos() => {
                    Some(CommandsAction::RepoGroupKey(true))
                }
                "arrowleft" if self.picker_browses_repos() => {
                    Some(CommandsAction::RepoGroupKey(false))
                }
                _ => None,
            },
            Some(LayerKind::RefPicker) => match binding {
                "escape" => Some(CommandsAction::CloseOverlay),
                "enter" => Some(CommandsAction::ConfirmOverlaySelection),
                "arrowdown" | "ctrl+n" => Some(CommandsAction::MoveOverlaySelection(1)),
                "arrowup" | "ctrl+p" => Some(CommandsAction::MoveOverlaySelection(-1)),
                "pagedown" => Some(CommandsAction::MoveOverlaySelection(8)),
                "pageup" => Some(CommandsAction::MoveOverlaySelection(-8)),
                "tab" | "shift+tab" => Some(CommandsAction::RefPicker(RefPickerAction::ToggleSide)),
                "mod+c" if self.ref_picker_query_empty() => {
                    Some(CommandsAction::RefPicker(RefPickerAction::CopySha))
                }
                _ => None,
            },
            Some(LayerKind::Confirmation) => {
                (binding == "escape").then_some(CommandsAction::CancelConfirmation)
            }
            None if self.settings_open() => {
                (binding == "escape").then_some(CommandsAction::CloseSettings)
            }
            None => None,
        };
        action.map(Action::from)
    }

    fn ref_picker_query_empty(&self) -> bool {
        self.ref_picker().is_some_and(|p| p.query.text().is_empty())
    }

    pub(super) fn menu_action(&self, id: &str) -> Option<Action> {
        self.command_by_id(id).map(|command| command.action)
    }
}

fn palette_key(binding: &str) -> bool {
    matches!(
        binding,
        "escape" | "enter" | "arrowdown" | "arrowup" | "pagedown" | "pageup" | "ctrl+n" | "ctrl+p"
    ) || binding.rsplit_once('+').is_some_and(|(mods, key)| {
        matches!(mods, "cmd" | "ctrl") && key.len() == 1 && key.as_bytes()[0].is_ascii_digit()
    })
}
