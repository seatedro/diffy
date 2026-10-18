use quark_app::platform::menu::{Menu, MenuAction, MenuItem, MenuRole};
use quark_app::{InputEvent, UiContext, WindowOptions};
use quark_ui::FocusId;
use quark_ui::accessibility::Politeness;
use quark_ui::key_context::KeyBindings;
use quark_ui::theme::Theme;

use super::state::{CommandsController, LayerKind};
use crate::app::route::Controllers;
use crate::contracts::commands::{CommandDescriptor, CommandsAction};
use crate::contracts::shell::Action;
use crate::fonts::FontSettings;

pub enum PlatformRequest {
    Focus(Option<FocusId>),
    Theme(Box<Theme>),
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
            Self::Theme(theme) => f.debug_tuple("Theme").field(&theme.mode).finish(),
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
    let mut requests = controllers.navigation.take_platform();
    requests.extend(controllers.commands.take_platform());
    requests
}

pub fn apply(requests: Vec<PlatformRequest>, cx: &mut UiContext) -> Vec<Action> {
    let mut followups = Vec::new();
    for request in requests {
        match request {
            PlatformRequest::Focus(focus) => cx.set_focus(focus),
            PlatformRequest::Theme(theme) => cx.set_theme(*theme),
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

pub fn key_action(controllers: &Controllers, event: &InputEvent) -> Option<Action> {
    let InputEvent::KeyPress(chord) = event else {
        return None;
    };
    let binding = chord.binding()?;
    controllers.commands.key_action(&binding.to_string())
}

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
                item(ids::SWITCH_REPOSITORY),
                MenuItem::Separator,
                item(ids::WORKING_TREE),
                item(ids::REFRESH),
                MenuItem::Separator,
                role(MenuRole::CloseWindow),
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
                MenuItem::Separator,
                role(MenuRole::ToggleFullscreen),
            ],
        ),
        Menu::window(),
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
    pub const SWITCH_REPOSITORY: &str = "repo.switch";
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
