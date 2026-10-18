use std::collections::VecDeque;

use std::rc::Rc;

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
use super::state::{CONFIRM_ACCEPT, CONFIRM_CANCEL};
use super::{CommandsController, LayerKind, PlatformRequest};
use crate::app::shell::Slot;
use crate::app::text_targets::{self, PICKER, SETTINGS, SIDEBAR, TextCx, TextHost};
use crate::contracts::commands::{
    CommandDescriptor, CommandsAction, CommandsEffect, ConfirmationRequest, SettingsSnapshot,
    ToastUpdate,
};
use crate::contracts::navigation::{CompareField, NavigationAction, RepositorySyncReason};
use crate::contracts::shared::{JobId, SharedContext};
use crate::contracts::shell::{
    Action, Controller, Effect, Intent, LogicalFocus, Reduction, ShellAction,
};
use crate::core::frecency::FrecencyStore;
use crate::core::keymap::{KeymapOverride, ShortcutCommand};
use crate::core::theme_types::ThemeMode;
use crate::platform::persistence::{Settings, SettingsStore};

struct CmdApp {
    commands: CommandsController,
    shared: SharedContext,
    dispatched: Vec<Action>,
    effects: Vec<Effect>,
    clock: u64,
    pending: Vec<Pending>,
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
        };
        app.push_intent(Intent::Startup(Rc::new(startup(no_args(), settings))));
        app
    }

    fn reduce(&mut self, reduction: Reduction) {
        self.effects.extend(reduction.effects);
        let mut queue = VecDeque::from(reduction.intents);
        while let Some(intent) = queue.pop_front() {
            match &intent {
                Intent::SettingsChanged(s) => self.shared.settings = s.clone(),
                Intent::RepositoryChanged(r) => self.shared.repository = Some(r.clone()),
                _ => {}
            }
            let next = self.commands.apply_intent(&intent, &self.shared);
            self.effects.extend(next.effects);
            queue.extend(next.intents);
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
        let overlays = super::overlays(&mut self.commands, &slot, cx);
        let toasts = super::toasts(&mut self.commands, &slot, cx);
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
        self.commands
            .edit_text(target, command, &cx)
            .unwrap_or_default()
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
    ui.click_node(By::role_name(Role::SearchInput, "Filter files"));
    let filter = ui.focus();

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

    ui.key("mod+p");
    ui.find(By::role_name(Role::Dialog, "Command palette"));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Dialog, "Command palette"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
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
    ui.click_node(By::role_name(Role::SearchInput, "Filter files"));
    let filter = ui.focus();
    ui.key("mod+,");
    ui.find(By::role_name(Role::Region, "Settings"));
    assert_eq!(ui.focus(), Some(SETTINGS));

    ui.click_node(By::role_name(Role::Tab, "Editor"));
    ui.find(By::role_name(Role::Switch, "Wrap lines"));
    ui.key("escape");
    assert!(
        ui.try_find(By::role_name(Role::Region, "Settings"))
            .is_none()
    );
    assert_eq!(ui.focus(), filter);
}

#[test]
fn settings_controls_update_and_persist_the_snapshot() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenSettings);
    ui.click_node(By::role_name(Role::Button, "Increase interface scale"));
    assert_eq!(settings(&ui).ui_scale_pct, 110);
    ui.find(By::name("110%"));
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
    let json = std::fs::read_to_string(&path).unwrap();
    for key in [
        "\"theme_mode\": \"light\"",
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
fn keymap_rebinding_rejects_conflicts_and_saves_overrides() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.run(CommandsAction::OpenKeymaps);
    ui.find(By::role_name(Role::Tab, "Keymaps"));
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
    ui.find(By::role_name(Role::Region, "Settings"));

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
        ui.app().dispatched,
        vec![Action::from(NavigationAction::OpenRepositoryDialog)]
    );
    ui.app_event(WindowEvent::Menu("app.about".to_owned()));
    ui.find(By::role_name(Role::Tab, "About"));
    assert_eq!(
        ui.app().commands.settings_page().map(|p| p.section),
        Some(crate::contracts::commands::SettingsSection::About)
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

#[test]
fn ref_picker_lists_repository_refs_and_commits_the_choice() {
    let mut ui = cmd_harness(Settings::default());
    focus_anchor(&mut ui);
    ui.inject(Intent::RepositoryChanged(Rc::new(
        crate::contracts::navigation::RepositoryContext {
            revision: crate::contracts::navigation::RepositoryRevision(1),
            snapshot: std::sync::Arc::new(super::fixtures::snapshot(
                RepositorySyncReason::Open,
                false,
            )),
        },
    )));
    ui.run(CommandsAction::OpenRefPicker(CompareField::Right));
    ui.find(By::role_name(Role::Dialog, "Head ref"));
    assert_eq!(ui.focus(), Some(PICKER));
    ui.type_text("feat");
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
