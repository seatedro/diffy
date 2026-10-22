use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use quark_app::testing::{By, UiTestHarness};
use quark_app::{AppEvent as WindowEvent, InputEvent, UiAdapter, UiApp, UiContext, ViewContext};
use quark_components::PALETTE_INPUT;
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole as Role;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};
use tempfile::TempDir;

use super::harness::{self, effects, no_args, startup};
use super::picker::PickerKind;
use super::state::{CONFIRM_ACCEPT, CONFIRM_CANCEL};
use super::{CommandsController, LayerKind, PlatformRequest};
use crate::app::shell::Slot;
use crate::app::text_targets::{
    self, LAUNCHER, PICKER, SETTINGS_SEARCH, SIDEBAR, TextCx, TextHost,
};
use crate::contracts::commands::{
    CommandDescriptor, CommandsAction, CommandsEffect, CommandsEvent, ConfirmationRequest,
    LauncherFolder, RefPickerRequest, SettingsSnapshot, ToastUpdate,
};
use crate::contracts::navigation::{CompareField, NavigationAction, RepositorySyncReason};
use crate::contracts::review::{GitHubRequest, ReviewAction, ReviewEffect, ReviewEvent};
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{
    Action, Controller, Effect, Intent, LogicalFocus, Reduction, ShellAction, ShellEffect,
    ShellEvent,
};
use crate::core::frecency::FrecencyStore;
use crate::core::keymap::{KeymapOverride, ShortcutCommand};
use crate::core::repo_discovery::{DiscoveredRepo, RepoScan};
use crate::core::theme_types::ThemeMode;
use crate::platform::persistence::{Settings, SettingsStore};

struct CmdApp {
    commands: CommandsController,
    shared: SharedContext,
    dispatched: Vec<Action>,
    effects: Vec<Effect>,
    clock: u64,
    pending: Vec<Pending>,
    intents: Vec<Intent>,
}

impl CmdApp {
    fn new(settings: Settings) -> Self {
        let mut app = Self {
            commands: CommandsController::default(),
            shared: SharedContext::default(),
            dispatched: Vec::new(),
            effects: Vec::new(),
            clock: 0,
            pending: Vec::new(),
            intents: Vec::new(),
        };
        app.push_intent(Intent::Startup(Rc::new(startup(no_args(), settings))));
        app
    }

    fn reduce(&mut self, reduction: Reduction) {
        let events: Vec<CommandsEvent> = reduction.effects.iter().filter_map(run_worker).collect();
        self.effects.extend(reduction.effects);
        let mut queue = VecDeque::from(reduction.intents);
        while let Some(intent) = queue.pop_front() {
            match &intent {
                Intent::Startup(startup) => {
                    let mut settings = startup.settings.clone();
                    super::normalize_settings(&mut settings);
                    self.shared.settings = Rc::new(SettingsSnapshot {
                        revision: self.shared.settings.revision + 1,
                        settings,
                    });
                }
                Intent::SettingsChanged(s) => self.shared.settings = s.clone(),
                Intent::RepositoryChanged(r) => self.shared.repository = Some(r.clone()),
                _ => {}
            }
            let next = self.commands.apply_intent(&intent, &self.shared);
            self.effects.extend(next.effects);
            queue.extend(next.intents);
            self.intents.push(intent);
        }
        for event in events {
            let next = self.commands.reduce_event(event, &self.shared);
            self.reduce(next);
        }
    }

    fn push_intent(&mut self, intent: Intent) {
        self.reduce(Reduction::default().intent(intent));
    }

    fn drain(&mut self, cx: &mut UiContext) {
        for _ in 0..8 {
            let requests = self.commands.take_platform();
            if requests.is_empty() {
                return;
            }
            for action in super::apply_platform(requests, cx) {
                self.route(action, cx);
            }
        }
    }

    fn route(&mut self, action: Action, cx: &mut UiContext) {
        match action {
            Action::Commands(action) => {
                let reduction = self.commands.reduce_action(action, &self.shared);
                self.reduce(reduction);
            }
            Action::Shell(ShellAction::FocusLogical(focus)) => {
                cx.set_focus(focus.map(text_targets::focus_id))
            }
            action => self.dispatched.push(action),
        }
    }
}

impl UiApp for CmdApp {
    type Action = Action;
    type Message = ();

    fn init(&mut self, cx: &mut UiContext) {
        self.drain(cx);
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let overlays = super::overlays(&mut self.commands, &slot, None, cx);
        let toasts = super::toasts(&mut self.commands, &slot, 0.0, None, cx);
        let theme = cx.theme;
        quark::view! {
            <div w={width} h={height} class="relative flex-col" bg={theme.colors.app_bg}>
                <div
                    w={width}
                    h={40.0}
                    focus_ring={SIDEBAR}
                    role="button"
                    aria-label="Anchor"
                    on:click={ShellAction::FocusLogical(Some(LogicalFocus::Sidebar))}
                />
                {?overlays}
                {?toasts}
            </div>
        }
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        self.clock = cx.window.elapsed().as_millis() as u64;
        self.route(action, cx);
        self.drain(cx);
    }

    fn message(&mut self, _message: (), cx: &mut UiContext) {
        for pending in std::mem::take(&mut self.pending) {
            match pending {
                Pending::Action(action) => self.route(action, cx),
                Pending::Intent(intent) => self.push_intent(intent),
            }
        }
        self.drain(cx);
        cx.window.request_redraw();
    }

    fn event(&mut self, event: &InputEvent, cx: &mut UiContext) -> bool {
        if let InputEvent::Focused(focused) = event {
            self.commands.observe_window_focus(*focused);
            return false;
        }
        let InputEvent::KeyPress(chord) = event else {
            return false;
        };
        let Some(binding) = chord.binding() else {
            return false;
        };
        match self.commands_key(&binding.to_string()) {
            Some(action) => {
                self.update(action, cx);
                cx.window.request_redraw();
                true
            }
            None => false,
        }
    }

    fn app_event(&mut self, event: WindowEvent, cx: &mut UiContext) {
        if let WindowEvent::Menu(id) = event
            && let Some(action) = self.commands.menu_action(&id)
        {
            self.update(action, cx);
        }
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.clock,
        };
        let outcome = self
            .commands
            .edit_text(target, command, &cx)
            .unwrap_or_default();
        let next = self.commands.after_edit(target, &self.shared);
        self.reduce(next);
        outcome
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.commands.set_preedit(target, text, cursor);
    }
}

impl CmdApp {
    fn commands_key(&self, binding: &str) -> Option<Action> {
        self.commands.key_action(binding)
    }
}

type Cmd = UiTestHarness<CmdApp>;

fn cmd_harness(settings: Settings) -> Cmd {
    UiTestHarness::with_adapter(
        UiAdapter::new(CmdApp::new(settings), "Diffy"),
        (1024.0, 720.0),
        2.0,
    )
}

fn settings(ui: &Cmd) -> &Settings {
    &ui.app().commands.settings().settings
}

fn saved(ui: &Cmd) -> Vec<Settings> {
    ui.app()
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Commands(CommandsEffect::SaveSettings(s)) => Some((**s).clone()),
            _ => None,
        })
        .collect()
}

fn focus_anchor(ui: &mut Cmd) {
    ui.click_node(By::role_name(Role::Button, "Anchor"));
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn palette_opens_from_the_shortcut_runs_a_command_and_returns_focus() {
    let mut ui = harness::harness();
    ui.click_node(By::role_name(Role::SearchInput, "Search repositories"));
    let filter = ui.focus();
    assert_eq!(filter, Some(LAUNCHER));

    ui.key("mod+k");
    ui.find(By::role_name(Role::Dialog, "Command palette"));
    assert_eq!(ui.focus(), Some(PALETTE_INPUT));
    ui.type_text("line wrapping");
    ui.key("enter");

    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Command palette"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
    let shared = ui.app().0.shared();
    assert!(shared.settings.settings.viewport.wrap_enabled);
    assert!(effects(&ui).iter().any(|e| matches!(
        e,
        Effect::Commands(CommandsEffect::SaveSettings(s)) if s.viewport.wrap_enabled
    )));

    ui.key("mod+k");
    ui.find(By::role_name(Role::Dialog, "Command palette"));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Command palette"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
}

#[test]
fn palette_omits_its_own_command() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenCommandPalette);
    ui.type_text("palette");
    let tree = ui.accessibility_tree();
    assert!(!tree.contains("\"Command Palette"));
    ui.find(By::name("No results"));
}

#[test]
fn external_commands_replace_builtins_and_claim_shortcuts() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::SetExternalCommands(Rc::from(vec![
        CommandDescriptor::new(
            "settings.open",
            "Open Settings Elsewhere",
            NavigationAction::RefreshRepository,
        ),
        CommandDescriptor::new(
            "review.next_thread",
            "Next Thread",
            NavigationAction::SelectNextFile,
        )
        .shortcut(ShortcutCommand::OpenSettings),
    ])));
    let commands = ui.app().commands.commands();
    assert_eq!(
        commands.iter().filter(|c| c.id == "settings.open").count(),
        1
    );
    assert!(
        commands
            .iter()
            .filter(|c| c.shortcut == Some(ShortcutCommand::OpenSettings))
            .all(|c| c.id == "review.next_thread")
    );
    ui.key("mod+,");
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::SelectNextFile)]
    );
    assert!(!ui.app().commands.settings_open());

    ui.run(CommandsAction::SetExternalCommands(Rc::from(Vec::new())));
    ui.key("mod+,");
    assert!(ui.app().commands.settings_open());
}

#[test]
fn a_new_overlay_replaces_the_open_one_and_escape_returns_focus() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenThemePicker);
    assert_eq!(ui.focus(), Some(PICKER));
    ui.run(CommandsAction::OpenCommandPalette);
    assert_eq!(ui.app().commands.top_layer(), Some(LayerKind::Palette));
    assert!(!ui.app().commands.has_layer(LayerKind::Picker));
    ui.key("escape");
    assert_eq!(ui.app().commands.top_layer(), None);
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn a_confirmation_blocks_other_overlays_until_answered() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenCommandPalette);
    ui.inject(Intent::RequestConfirmation(discard_request()));
    ui.frame();
    assert_eq!(ui.app().commands.top_layer(), Some(LayerKind::Confirmation));
    assert!(!ui.app().commands.has_layer(LayerKind::Palette));
    ui.run(CommandsAction::OpenCommandPalette);
    ui.run(CommandsAction::OpenSettings);
    ui.run(CommandsAction::DismissOverlays);
    assert_eq!(ui.app().commands.top_layer(), Some(LayerKind::Confirmation));
    assert!(!ui.app().commands.settings_open());
    ui.key("escape");
    assert_eq!(ui.app().commands.top_layer(), None);
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn palette_routes_other_streams_actions_through_dispatch() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::SetExternalCommands(Rc::from(vec![
        CommandDescriptor::new(
            "review.submit",
            "Submit Review",
            NavigationAction::RefreshRepository,
        ),
    ])));
    ui.run(CommandsAction::OpenCommandPalette);
    ui.type_text("submit review");
    ui.key("enter");
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::RefreshRepository)]
    );
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn palette_hands_a_pull_request_url_to_review() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenCommandPalette);
    ui.type_text("open pull request");
    ui.key("enter");
    ui.find(By::role_name(Role::Dialog, "Open pull request"));
    assert_eq!(ui.focus(), Some(PICKER));
    ui.type_text("https://github.com/o/r/pull/7");
    ui.key("enter");
    assert!(ui.app().intents.iter().any(|i| matches!(
        i,
        Intent::OpenPullRequest { url } if url == "https://github.com/o/r/pull/7"
    )));
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn pasting_a_pull_request_link_into_the_palette_opens_it() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenCommandPalette);
    ui.type_text("https://github.com/owner/repo/pull/7/files");
    ui.find(By::name("Open owner/repo#7"));
    ui.key("enter");
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(ReviewAction::LoadPullRequest {
            url: "https://github.com/owner/repo/pull/7".to_owned(),
        })]
    );

    ui.run(CommandsAction::OpenCommandPalette);
    ui.type_text("owner/repo#");
    assert!(ui.try_find(By::name("Open owner/repo#")).is_none());
}

#[test]
fn palette_recents_follow_frecency() {
    let dir = TempDir::new().unwrap();
    let store = FrecencyStore::open(dir.path()).unwrap();
    let mut commands = CommandsController::default();
    commands.set_frecency(store.clone());
    let shared = SharedContext::default();
    commands.apply_intent(
        &Intent::Startup(Rc::new(startup(no_args(), Settings::default()))),
        &shared,
    );

    commands.reduce_action(CommandsAction::RunCommand("theme.pick"), &shared);
    commands.reduce_action(CommandsAction::CloseOverlay, &shared);
    assert!(store.score("cmd:theme.pick") > 0.0);

    commands.reduce_action(CommandsAction::OpenCommandPalette, &shared);
    assert_eq!(
        commands.palette().recent().first(),
        Some(&quark::stable_hash("theme.pick"))
    );
}

#[test]
fn settings_open_from_the_shortcut_and_escape_returns_focus() {
    let mut ui = harness::harness();
    harness::deliver(
        &mut ui,
        ShellEvent::RepositoryDialogClosed {
            path: Some(PathBuf::from("/fixtures/alpha")),
        },
    );
    ui.click_node(By::role_name(Role::SearchInput, "Filter files"));
    let filter = ui.focus();
    ui.key("mod+,");
    ui.find(By::role_name(Role::Dialog, "Settings"));
    assert_eq!(ui.focus(), Some(SETTINGS_SEARCH));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Settings"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
    ui.key("mod+,");
    ui.find(By::role_name(Role::Navigation, "Files"));

    ui.click_node(By::role_name(Role::Tab, "Diff"));
    ui.find(By::role_name(Role::Switch, "Wrap lines"));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Settings"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
}

#[test]
fn settings_controls_update_and_persist_the_snapshot() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenSettings);
    ui.find(By::test_id("settings.preview"));
    ui.click_node(By::role_name(Role::RadioButton, "System"));
    assert_eq!(settings(&ui).theme_mode, ThemeMode::System);
    ui.run(CommandsAction::IncreaseUiScale);
    assert_eq!(settings(&ui).ui_scale_pct, 110);
    ui.run(CommandsAction::SetThemeMode(ThemeMode::Light));
    assert_eq!(settings(&ui).theme_mode, ThemeMode::Light);
    for _ in 0..20 {
        ui.run(CommandsAction::IncreaseUiScale);
    }
    assert_eq!(settings(&ui).ui_scale_pct, super::state::MAX_UI_SCALE_PCT);
    let last = saved(&ui).pop().expect("saved settings");
    assert_eq!(last, *settings(&ui));
    assert_eq!(ui.app().shared.settings.settings, last);
}

#[test]
fn existing_settings_files_load_and_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(
        &path,
        r#"{
  "theme_mode": "light",
  "theme_name": "storm",
  "ui_scale_pct": 300,
  "fonts": { "ui_family": "__diffy_system_font__", "mono_family": "JetBrains Mono" },
  "sidebar_width_px": 320,
  "last_compare": {
    "repo_path": "/fixtures/alpha",
    "left_ref": "main",
    "right_ref": "feature",
    "mode": "two-dot",
    "layout": "split",
    "renderer": "builtin"
  },
  "viewport": { "wrap_enabled": true, "wrap_column": 100, "layout": "split" },
  "wheel_scroll_lines": 5,
  "auto_update": false,
  "continuous_scroll": true,
  "ai_steering_prompt": "",
  "keymap_overrides": [{ "command": "toggle-wrap", "binding": "shift+w" }],
  "unknown_field": 1
}"#,
    )
    .unwrap();
    let store = SettingsStore::new_in(dir.path());
    let loaded = store.load().unwrap();
    assert_eq!(loaded.theme_name, "storm");
    assert_eq!(
        loaded.keymap_overrides[0].command,
        ShortcutCommand::ToggleWrap
    );

    let mut ui = cmd_harness(loaded.clone());
    let snapshot = settings(&ui).clone();
    assert_eq!(snapshot.ui_scale_pct, super::state::MAX_UI_SCALE_PCT);
    assert_eq!(snapshot.fonts.ui_family, crate::fonts::UI_FAMILY);
    assert_eq!(snapshot.last_compare, loaded.last_compare);
    assert_eq!(snapshot.keymap_overrides, loaded.keymap_overrides);

    ui.run(CommandsAction::ToggleContinuousScroll);
    let written = saved(&ui).pop().expect("saved");
    store.save(&written).unwrap();
    let reloaded = store.load().unwrap();
    assert_eq!(reloaded, written);
    assert!(!reloaded.continuous_scroll);
    assert!(!path.exists());
    let json: String = crate::platform::database::Database::in_dir(dir.path())
        .unwrap()
        .connection()
        .unwrap()
        .query_row("SELECT json FROM settings", [], |row| row.get(0))
        .unwrap();
    for key in [
        "\"theme_mode\":\"light\"",
        "\"toggle-wrap\"",
        "\"two-dot\"",
        "\"keymap_overrides\"",
    ] {
        assert!(json.contains(key), "{key} missing from {json}");
    }
}

#[test]
fn foreign_settings_changes_are_adopted_and_persisted() {
    let mut ui = cmd_harness(Settings::default());
    let mut next = settings(&ui).clone();
    next.sidebar_width_px = Some(333);
    let snapshot = Rc::new(SettingsSnapshot {
        revision: ui.app().commands.settings().revision + 1,
        settings: next.clone(),
    });
    ui.inject(Intent::SettingsChanged(snapshot));
    assert_eq!(settings(&ui).sidebar_width_px, Some(333));
    assert_eq!(saved(&ui).pop(), Some(next));
}

#[test]
fn keymap_list_scrolls_to_its_last_binding() {
    let mut ui = cmd_harness(Settings::default());
    ui.resize(900.0, 420.0);
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenKeymaps);
    let last = |ui: &Cmd| {
        ui.find_all(Role::Button)
            .into_iter()
            .filter(|n| n.name.as_deref().is_some_and(|n| n.starts_with("Rebind ")))
            .last()
            .unwrap()
    };
    let before = last(&ui).bounds;
    assert!(before.y > 420.0);
    let content = ui.find(By::test_id("settings.content")).bounds;
    ui.pointer_move((
        content.x + content.width / 2.0,
        content.y + content.height / 2.0,
    ));
    ui.wheel(0.0, 100_000.0);
    let after = last(&ui).bounds;
    assert!(after.y + after.height <= 420.0);
    ui.run(CommandsAction::SetSettingsSection(
        crate::contracts::commands::SettingsSection::Appearance,
    ));
    ui.run(CommandsAction::SetSettingsSection(
        crate::contracts::commands::SettingsSection::Keymaps,
    ));
    assert_eq!(last(&ui).bounds.y, before.y);
}

#[test]
fn keymap_rebinding_rejects_conflicts_and_saves_overrides() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenKeymaps);
    ui.find(By::role_name(Role::Tab, "Keyboard"));
    ui.click_node(By::role_name(Role::Button, "Rebind Next hunk"));
    ui.find(By::role_name(Role::Button, "Press keys for Next hunk"));

    ui.key("n");
    ui.find(By::role_name(Role::Alert, "N is already used by Next file"));
    assert!(settings(&ui).keymap_overrides.is_empty());

    ui.key("shift+h");
    assert_eq!(
        settings(&ui).keymap_overrides,
        vec![KeymapOverride {
            command: ShortcutCommand::NextHunk,
            binding: "shift+h".to_owned()
        }]
    );
    assert!(
        ui.try_find(By::role_name(Role::Button, "Press keys for Next hunk"))
            .is_none()
    );

    ui.click_node(By::role_name(Role::Button, "Rebind Next file"));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Button, "Press keys for Next file"))
            .is_none()
    );
    ui.find(By::role_name(Role::Dialog, "Settings"));

    ui.click_node(By::role_name(Role::Button, "Reset Next hunk"));
    assert!(settings(&ui).keymap_overrides.is_empty());
}

#[test]
fn rebinding_installs_new_key_bindings_and_menus() {
    let mut commands = CommandsController::default();
    let shared = SharedContext::default();
    commands.apply_intent(
        &Intent::Startup(Rc::new(startup(no_args(), Settings::default()))),
        &shared,
    );
    let initial = commands.take_platform();
    assert!(
        initial
            .iter()
            .any(|r| matches!(r, PlatformRequest::Theme(_)))
    );
    assert!(
        initial
            .iter()
            .any(|r| matches!(r, PlatformRequest::KeyBindings(_)))
    );
    let menus = initial
        .iter()
        .find_map(|r| match r {
            PlatformRequest::Menus(m) => Some(m.clone()),
            _ => None,
        })
        .expect("menus");
    let labels: Vec<&str> = menus.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, ["Diffy", "File", "Edit", "View", "Window", "Help"]);
    let app_menu = format!("{:?}", menus[0]);
    assert!(
        app_menu.contains("Quit Diffy")
            && app_menu.contains("About Diffy")
            && app_menu.contains("Settings")
    );
    assert!(format!("{:?}", menus[1]).contains("Open Repository"));

    commands.reduce_action(CommandsAction::OpenKeymaps, &shared);
    commands.reduce_action(
        CommandsAction::ApplyKeymapBinding {
            command: ShortcutCommand::ToggleSidebar,
            binding: "cmd+shift+b".to_owned(),
        },
        &shared,
    );
    let requests = commands.take_platform();
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, PlatformRequest::KeyBindings(_)))
    );
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, PlatformRequest::Menus(_)))
    );
    let primary = if cfg!(target_os = "macos") {
        "mod+shift+b"
    } else {
        "cmd+shift+b"
    };
    assert_eq!(
        commands.settings().settings.keymap_overrides[0].binding,
        primary
    );
}

#[test]
fn menu_items_route_through_command_descriptors() {
    let mut ui = cmd_harness(Settings::default());
    ui.app_event(WindowEvent::Menu("repo.open".to_owned()));
    assert_eq!(
        ui.app().commands.picker().map(|p| p.kind),
        Some(PickerKind::Repository)
    );
    assert!(ui.app().dispatched.is_empty());
    ui.key("escape");
    ui.app_event(WindowEvent::Menu("app.about".to_owned()));
    ui.find(By::role_name(Role::Tab, "Updates"));
    assert_eq!(
        ui.app().commands.settings_page().map(|p| p.section),
        Some(crate::contracts::commands::SettingsSection::Updates)
    );
}

fn discard_request() -> ConfirmationRequest {
    ConfirmationRequest {
        title: "Confirm discard".to_owned(),
        message: "Discard changes to one file?".to_owned(),
        confirm_label: "Discard".to_owned(),
        destructive: true,
        confirm: Box::new(NavigationAction::RefreshRepository.into()),
        return_focus: None,
    }
}

#[test]
fn confirmation_traps_focus_and_escape_cancels_back_to_the_caller() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.inject(Intent::RequestConfirmation(discard_request()));
    ui.frame();
    ui.frame();
    ui.find(By::role_name(Role::Dialog, "Confirm discard"));
    ui.find(By::role_name(Role::Button, "Cancel"));

    ui.key("tab");
    assert_eq!(ui.focus(), Some(CONFIRM_ACCEPT));
    ui.key("tab");
    assert_eq!(ui.focus(), Some(CONFIRM_CANCEL));

    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Confirm discard"))
            .is_none()
    );
    assert_eq!(ui.focus(), Some(SIDEBAR));
    assert!(ui.app().dispatched.is_empty());
}

#[test]
fn confirming_dispatches_the_requested_action() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.inject(Intent::RequestConfirmation(discard_request()));
    ui.frame();
    ui.click_node(By::role_name(Role::Button, "Discard"));
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::RefreshRepository)]
    );
    assert!(!ui.app().commands.has_layer(LayerKind::Confirmation));
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

fn ref_request(side: CompareField, working_copy: bool) -> CommandsAction {
    CommandsAction::ShowRefPicker(Box::new(RefPickerRequest {
        side,
        left: "main".to_owned(),
        right: if working_copy {
            "Working tree"
        } else {
            "feature"
        }
        .to_owned(),
        working_copy,
        mode: crate::core::compare::CompareMode::ThreeDot,
        renderer: crate::core::compare::RendererKind::Builtin,
    }))
}

fn with_repository(ui: &mut Cmd) {
    ui.inject(Intent::RepositoryChanged(Rc::new(
        crate::contracts::navigation::RepositoryContext {
            revision: crate::contracts::navigation::RepositoryRevision(1),
            snapshot: std::sync::Arc::new(super::fixtures::snapshot(
                RepositorySyncReason::Open,
                false,
            )),
        },
    )));
}

#[test]
fn palette_ref_command_asks_navigation_for_the_compare() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenRefPicker(CompareField::Left));
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::ChooseRef(
            CompareField::Left
        ))]
    );
}

#[test]
fn ref_picker_groups_refs_and_commits_the_choice() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    with_repository(&mut ui);
    ui.run(ref_request(CompareField::Right, false));
    ui.find(By::role_name(Role::Dialog, "Head ref"));
    assert_eq!(ui.focus(), Some(PICKER));
    ui.find(By::role_name(Role::Heading, "Branches \u{00B7} 2"));
    let current = ui.find(By::role_name(Role::ListBoxOption, "feature"));
    assert!(current.bounds.height > 0.0);
    ui.type_text("feat");
    ui.find(By::role_name(Role::Heading, "Branches \u{00B7} 1 match"));
    ui.key("enter");
    assert_eq!(
        ui.app().dispatched,
        vec![
            Action::from(NavigationAction::SetRef {
                field: CompareField::Right,
                value: "feature".to_owned()
            }),
            Action::from(NavigationAction::StartCompare),
        ]
    );
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn ref_picker_tab_swaps_sides_and_footer_sets_mode_and_engine() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    with_repository(&mut ui);
    ui.run(ref_request(CompareField::Left, false));
    ui.find(By::role_name(Role::Dialog, "Base ref"));
    ui.key("tab");
    ui.find(By::role_name(Role::Dialog, "Head ref"));
    assert_eq!(ui.focus(), Some(PICKER));
    ui.click_node(By::role_name(Role::RadioButton, "Directly"));
    ui.click_node(By::role_name(Role::RadioButton, "Semantic"));
    assert_eq!(
        ui.app().dispatched,
        vec![
            Action::from(NavigationAction::SetCompareMode(
                crate::core::compare::CompareMode::TwoDot
            )),
            Action::from(
                crate::contracts::viewport::pipeline::PipelineAction::SetRenderer(
                    crate::core::compare::RendererKind::Semantic
                )
            ),
        ]
    );
    ui.find(By::role_name(Role::Dialog, "Head ref"));
}

#[test]
fn ref_picker_against_the_working_tree_keeps_the_worktree_side() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    with_repository(&mut ui);
    ui.run(ref_request(CompareField::Left, true));
    ui.type_text("feature");
    ui.key("enter");
    assert_eq!(
        ui.app().dispatched[..2],
        [
            Action::from(NavigationAction::SetRef {
                field: CompareField::Left,
                value: "feature".to_owned()
            }),
            Action::from(NavigationAction::SetRef {
                field: CompareField::Right,
                value: crate::core::vcs::git::WORKDIR_REF.to_owned()
            }),
        ]
    );
}

#[test]
fn ref_picker_lays_out_only_visible_rows_for_thousands_of_refs() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    let mut snapshot = super::fixtures::snapshot(RepositorySyncReason::Open, false);
    snapshot
        .refs
        .extend((0..10_000).map(|i| crate::core::vcs::model::VcsRef {
            name: format!("tag-{i}"),
            kind: crate::core::vcs::model::RefKind::Tag,
            target: crate::core::vcs::model::RevisionId::git(format!("{i:07}")),
            active: false,
            upstream: None,
            ahead_behind: None,
            worktree: None,
        }));
    ui.inject(Intent::RepositoryChanged(Rc::new(
        crate::contracts::navigation::RepositoryContext {
            revision: crate::contracts::navigation::RepositoryRevision(1),
            snapshot: std::sync::Arc::new(snapshot),
        },
    )));
    ui.run(ref_request(CompareField::Left, false));
    let options = ui.find_all(Role::ListBoxOption).len();
    assert!(options > 0 && options < 40, "{options} options laid out");
    ui.key("pagedown");
    ui.key("pagedown");
    let selected = ui.app().commands.ref_picker().and_then(|p| p.selected);
    assert!(selected.is_some_and(|row| row > 10));
}

#[test]
fn picker_escape_and_arrow_keys() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenThemePicker);
    ui.find(By::role_name(Role::Dialog, "Theme"));
    let start = ui.app().commands.picker().map(|p| p.selected);
    ui.key("arrowdown");
    assert_eq!(
        ui.app().commands.picker().map(|p| p.selected),
        start.map(|s| s + 1)
    );
    ui.key("escape");
    assert!(ui.try_find(By::role_name(Role::Dialog, "Theme")).is_none());
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn toasts_show_progress_and_dismiss() {
    let mut ui = cmd_harness(Settings::default());
    ui.inject(Intent::Toast(ToastUpdate::Progress {
        id: JobId(4),
        message: "Fetching origin".to_owned(),
        completed: 1,
        total: 3,
    }));
    ui.inject(Intent::Toast(ToastUpdate::error("Push failed")));
    ui.frame();
    ui.find(By::role_name(Role::Status, "Fetching origin"));
    ui.find(By::role_name(Role::Alert, "Push failed"));
    let dismiss = ui.find_all(By::role_name(Role::Button, "Dismiss"));
    ui.click(dismiss[1].center());
    assert!(
        ui.try_find(By::role_name(Role::Alert, "Push failed"))
            .is_none()
    );
    ui.inject(Intent::Toast(ToastUpdate::Dismiss(JobId(4))));
    assert!(
        ui.try_find(By::role_name(Role::Status, "Fetching origin"))
            .is_none()
    );
}

fn run_worker(effect: &Effect) -> Option<CommandsEvent> {
    match effect {
        Effect::Commands(CommandsEffect::LoadRefCatalog(request)) => {
            Some(CommandsEvent::RefCatalogReady {
                generation: request.generation,
                catalog: std::sync::Arc::new(crate::core::ref_catalog::build_catalog(
                    &request.snapshot.refs,
                    &request.snapshot.changes,
                    &[],
                    &request.recent,
                    true,
                )),
            })
        }
        Effect::Commands(CommandsEffect::FilterRefs {
            generation,
            catalog,
            query,
        }) => Some(CommandsEvent::RefsFiltered {
            generation: *generation,
            results: std::sync::Arc::new(crate::core::ref_catalog::filter(catalog, query)),
        }),
        _ => None,
    }
}

enum Pending {
    Action(Action),
    Intent(Intent),
}

trait Drive {
    fn run(&mut self, action: impl Into<Action>);
    fn inject(&mut self, intent: Intent);
}

impl Drive for Cmd {
    fn run(&mut self, action: impl Into<Action>) {
        self.app_mut().pending.push(Pending::Action(action.into()));
        self.send_message(());
    }

    fn inject(&mut self, intent: Intent) {
        self.app_mut().pending.push(Pending::Intent(intent));
        self.send_message(());
    }
}

#[test]
fn return_inserts_a_newline_in_multiline_editors() {
    let mut ui = harness::harness();
    ui.key("mod+k");
    ui.type_text("new text compare");
    ui.key("enter");
    ui.click_node(By::test_id("text-compare.left"));
    ui.type_text("one");
    ui.key("enter");
    ui.type_text("two");
    let controllers = ui.app().0.controllers();
    assert_eq!(
        controllers.viewport.interaction.text_compare().left.text(),
        "one\ntwo"
    );
}

#[test]
fn theme_picker_offers_and_selects_the_current_default_theme() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenThemePicker);
    let current = ui.find(By::role_name(Role::ListBoxOption, "TokyoNight"));
    assert_eq!(
        ui.find_all(Role::ListBoxOption)
            .iter()
            .filter(|n| n.name.as_deref() == Some("TokyoNight"))
            .count(),
        1
    );
    assert!(current.bounds.height > 0.0);
    ui.type_text("tokyo");
    ui.find(By::role_name(Role::ListBoxOption, "TokyoNight"));
}

#[test]
fn undo_toasts_dispatch_undo_or_expiry_once() {
    let mut ui = cmd_harness(Settings::default());
    let toast = |id| {
        Intent::Toast(ToastUpdate::Undo {
            id: JobId(id),
            kind: crate::contracts::commands::ToastKind::Success,
            message: "Discarded 3 lines".to_owned(),
            undo: Box::new(NavigationAction::RefreshRepository.into()),
            expires_ms: 8000,
            expired: Some(Box::new(NavigationAction::StartCompare.into())),
        })
    };
    ui.inject(toast(1));
    assert!(ui.app().effects.iter().any(|e| matches!(
        e,
        Effect::Commands(CommandsEffect::ExpireToast {
            id: JobId(1),
            after_ms: 8000
        })
    )));
    ui.click_node(By::role_name(Role::Button, "Undo"));
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::RefreshRepository)]
    );
    ui.inject(toast(2));
    ui.click_node(By::role_name(Role::Button, "Dismiss"));
    assert_eq!(
        ui.app().dispatched[1..],
        [Action::from(NavigationAction::StartCompare)]
    );
}

#[test]
fn settings_search_finds_rows_across_sections_and_keys() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenSettings);
    ui.click_node(By::role_name(Role::SearchInput, "Search settings"));
    ui.type_text("wrap");
    ui.find(By::role_name(Role::Switch, "Wrap lines"));
    ui.find(By::role_name(Role::Group, "Wrap column"));
    assert!(ui.try_find(By::role_name(Role::Group, "Theme")).is_none());
    ui.find(By::role_name(Role::Button, "Rebind Toggle line wrapping"));
}

#[test]
fn activation_click_on_the_backdrop_keeps_the_palette_open() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenCommandPalette);
    ui.send_event(InputEvent::Focused(true));
    ui.run(CommandsAction::Palette(
        quark_components::PaletteEvent::Dismiss,
    ));
    assert_eq!(ui.app().commands.top_layer(), Some(LayerKind::Palette));
    ui.run(CommandsAction::Palette(
        quark_components::PaletteEvent::Dismiss,
    ));
    assert_eq!(ui.app().commands.top_layer(), None);
}

#[test]
fn a_tooltip_shows_after_the_pointer_rests_and_hides_when_it_leaves() {
    let mut ui = harness::harness();
    let button = ui.find(By::role_name(Role::Button, "Syntax languages"));
    ui.pointer_move(button.center());
    ui.frame();
    assert!(!ui.painted_text().contains("Syntax languages"));
    ui.advance(600);
    assert!(ui.painted_text().contains("Syntax languages"));
    ui.pointer_move((button.center().0, 10.0));
    ui.frame();
    assert!(!ui.painted_text().contains("Syntax languages"));
}

fn scanned(ui: &mut harness::Harness, repos: &[(&str, Option<&str>)]) {
    let scan = RepoScan {
        repos: repos
            .iter()
            .map(|(path, remote)| DiscoveredRepo {
                path: PathBuf::from(path),
                branch: Some("main".to_owned()),
                active: None,
                remote: remote.map(str::to_owned),
            })
            .collect(),
        ..RepoScan::default()
    };
    harness::deliver(
        ui,
        CommandsEvent::RepositoriesScanned {
            scan: Arc::new(scan),
            cached: false,
        },
    );
}

fn opened_repo(ui: &harness::Harness) -> Option<PathBuf> {
    ui.app()
        .0
        .controllers()
        .navigation
        .repo_path()
        .map(PathBuf::from)
}

#[test]
fn launcher_replaces_the_workspace_until_a_repository_opens_and_focuses_its_box() {
    let ui = harness::harness();
    ui.find(By::role_name(Role::Main, "Launcher"));
    ui.find(By::name("What do you want to review?"));
    let input = ui.find(By::role_name(Role::SearchInput, "Search repositories"));
    assert!(input.focused);
    assert_eq!(ui.focus(), Some(LAUNCHER));
    for section in ["Pull requests", "More"] {
        ui.find(By::role_name(Role::Group, section));
    }
    ui.find(By::role_name(Role::ListBoxOption, "Open folder\u{2026}"));
    assert!(
        ui.try_find(By::role_name(Role::Navigation, "Files"))
            .is_none()
    );
}

#[test]
fn typing_in_the_launcher_filters_repositories_and_counts_visible_changes() {
    let mut ui = harness::harness();
    scanned(&mut ui, &[("/w/alpha", None), ("/w/beta", None)]);
    ui.find(By::role_name(Role::Group, "Repositories"));
    ui.find(By::role_name(Role::ListBoxOption, "alpha"));
    ui.find(By::role_name(Role::ListBoxOption, "beta"));
    assert!(effects(&ui).iter().any(|effect| matches!(
        effect,
        Effect::Commands(CommandsEffect::CountChanges(paths)) if paths.len() == 2
    )));

    ui.type_text("bet");
    ui.find(By::role_name(Role::ListBoxOption, "beta"));
    assert!(
        ui.try_find(By::role_name(Role::ListBoxOption, "alpha"))
            .is_none()
    );

    harness::deliver(
        &mut ui,
        CommandsEvent::ChangesCounted {
            path: PathBuf::from("/w/beta"),
            count: Some(3),
        },
    );
    assert!(ui.painted_text().contains("3 changed"));

    ui.key("escape");
    ui.find(By::role_name(Role::ListBoxOption, "alpha"));
}

#[test]
fn enter_and_arrows_in_the_launcher_open_a_repository() {
    let mut ui = harness::harness();
    scanned(&mut ui, &[("/w/alpha", None), ("/w/beta", None)]);
    ui.key("arrowdown");
    ui.key("ctrl+p");
    ui.key("arrowdown");
    ui.key("enter");
    assert_eq!(opened_repo(&ui), Some(PathBuf::from("/w/beta")));
    assert!(ui.try_find(By::role_name(Role::Main, "Launcher")).is_none());
    assert_eq!(ui.focus(), Some(SIDEBAR));
}

#[test]
fn a_pull_request_link_opens_the_matching_local_clone_then_the_pull_request() {
    let mut ui = harness::harness();
    harness::deliver(&mut ui, ReviewEvent::TokenLoaded { token: None });
    scanned(
        &mut ui,
        &[
            ("/w/alpha", Some("git@github.com:Owner/Alpha.git")),
            ("/w/beta", None),
        ],
    );
    harness::clear_effects(&mut ui);
    ui.type_text("https://github.com/owner/alpha/pull/7/files");
    ui.find(By::role_name(Role::ListBoxOption, "Open owner/alpha#7"));
    ui.key("enter");
    assert_eq!(opened_repo(&ui), Some(PathBuf::from("/w/alpha")));
    assert!(effects(&ui).iter().any(|effect| matches!(
        effect,
        Effect::Review(ReviewEffect::GitHub { request, .. })
            if **request == GitHubRequest::LoadPullRequest {
                url: "https://github.com/owner/alpha/pull/7".to_owned(),
                repo_path: PathBuf::from("/w/alpha"),
                token: None,
            }
    )));
}

fn launcher_text(ui: &mut Cmd, text: &str) {
    let app = ui.app_mut();
    let cx = TextCx {
        shared: &app.shared,
        now_ms: 0,
    };
    app.commands.set_text_value(LAUNCHER, text.to_owned(), &cx);
}

fn launcher_event(ui: &mut Cmd, event: CommandsEvent) {
    let app = ui.app_mut();
    let reduction = app.commands.reduce_event(event, &app.shared);
    app.reduce(reduction);
    ui.send_message(());
}

#[test]
fn a_pull_request_without_a_local_clone_asks_for_its_folder() {
    let mut ui = cmd_harness(Settings::default());
    launcher_text(&mut ui, "other/repo#3");
    ui.run(CommandsAction::LauncherConfirm);
    let url = "https://github.com/other/repo/pull/3".to_owned();
    let folder = LauncherFolder::LocateClone { url: url.clone() };
    assert!(
        ui.app()
            .effects
            .contains(&Effect::Shell(ShellEffect::OpenLauncherDialog(
                folder.clone()
            )))
    );
    ui.run(CommandsAction::LauncherFolderChosen {
        folder,
        path: PathBuf::from("/w/elsewhere"),
    });
    assert_eq!(
        ui.app().dispatched,
        vec![
            Action::from(NavigationAction::OpenRepository(PathBuf::from(
                "/w/elsewhere"
            ))),
            Action::from(ReviewAction::LoadPullRequest { url }),
        ]
    );
}

#[test]
fn the_signed_out_launcher_row_opens_github_sign_in() {
    let mut ui = harness::harness();
    ui.click_node(By::role_name(
        Role::ListBoxOption,
        "Sign in to GitHub to see your PRs",
    ));
    ui.find(By::role_name(Role::Dialog, "Sign in to GitHub"));
}

#[test]
fn cloning_from_the_launcher_reports_progress_and_opens_the_clone() {
    let mut ui = cmd_harness(Settings::default());
    launcher_text(&mut ui, "owner/repo");
    ui.run(CommandsAction::LauncherConfirm);
    let folder = LauncherFolder::CloneInto {
        url: "https://github.com/owner/repo.git".to_owned(),
        name: "repo".to_owned(),
    };
    assert!(
        ui.app()
            .effects
            .contains(&Effect::Shell(ShellEffect::OpenLauncherDialog(
                folder.clone()
            )))
    );
    ui.run(CommandsAction::LauncherFolderChosen {
        folder,
        path: PathBuf::from("/w"),
    });
    let job = ui
        .app()
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Commands(CommandsEffect::CloneRepository { job, url, dest }) => {
                assert_eq!(url, "https://github.com/owner/repo.git");
                assert_eq!(dest, &PathBuf::from("/w/repo"));
                Some(*job)
            }
            _ => None,
        })
        .expect("clone effect");
    launcher_event(
        &mut ui,
        CommandsEvent::CloneProgress {
            job,
            received: 5,
            total: 10,
        },
    );
    ui.find(By::role_name(
        Role::Status,
        "Cloning repo: 5/10 objects\u{2026}",
    ));
    launcher_event(
        &mut ui,
        CommandsEvent::CloneFinished {
            job,
            path: PathBuf::from("/w/repo"),
            result: Ok(()),
        },
    );
    ui.find(By::role_name(Role::Status, "Cloned repo"));
    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(NavigationAction::OpenRepository(
            PathBuf::from("/w/repo")
        ))]
    );
}

#[test]
fn g_then_p_opens_your_pull_requests() {
    use crate::contracts::review::{PullsAction, ReviewAction};

    let mut ui = cmd_harness(Settings::default());
    ui.run(CommandsAction::Shortcut(ShortcutCommand::GoPrefix));
    ui.run(CommandsAction::Leader("p".to_owned()));

    assert_eq!(
        ui.app().dispatched,
        vec![Action::from(ReviewAction::Pulls(PullsAction::Open))]
    );
}
