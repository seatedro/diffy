use quark_app::testing::UiTestHarness;
use quark_app::{AppEvent as WindowEvent, InputEvent, UiAdapter, UiApp, UiContext, ViewContext};
use quark_ui::FocusId;
use quark_ui::element::AnyElement;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use crate::app::testing::{RecordingDispatcher, SIZE, TITLE, TestApp};
use crate::app::theme::ThemePair;
use crate::contracts::shared::WorkspaceEpoch;
use crate::contracts::shell::{Action, AppEvent, AppMessage, AppStartup, Effect, EventEnvelope};
use crate::platform::persistence::Settings;
use crate::platform::startup::{Args, StartupOptions};

const MAX_DRAIN_ROUNDS: usize = 16;

pub struct Hooked(pub TestApp);

impl Hooked {
    fn drain(&mut self, cx: &mut UiContext) {
        for _ in 0..MAX_DRAIN_ROUNDS {
            let requests = super::take_requests(self.0.controllers());
            if requests.is_empty() {
                return;
            }
            for action in super::apply_platform(requests, cx) {
                self.0.update(action, cx);
            }
        }
    }
}

impl UiApp for Hooked {
    type Action = Action;
    type Message = AppMessage;

    fn init(&mut self, cx: &mut UiContext) {
        self.0.init(cx);
        self.drain(cx);
    }

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        self.0.view(cx)
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        self.0.update(action, cx);
        self.drain(cx);
    }

    fn message(&mut self, message: AppMessage, cx: &mut UiContext) {
        self.0.message(message, cx);
        self.drain(cx);
    }

    fn app_event(&mut self, event: WindowEvent, cx: &mut UiContext) {
        if let WindowEvent::Menu(id) = &event
            && let Some(action) = super::menu_action(self.0.controllers(), id)
        {
            self.update(action, cx);
            return;
        }
        self.0.app_event(event, cx);
        self.drain(cx);
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        self.0.edit_text(target, command)
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.0.set_preedit(target, text, cursor);
    }

    fn set_text_value(&mut self, target: FocusId, value: String, cx: &mut UiContext) {
        self.0.set_text_value(target, value, cx);
        self.drain(cx);
    }

    fn event(&mut self, event: &InputEvent, cx: &mut UiContext) -> bool {
        if let Some(action) = super::key_action(self.0.controllers(), event, cx.focus()) {
            self.update(action, cx);
            cx.window.request_redraw();
            return true;
        }
        self.0.event(event, cx)
    }
}

pub type Harness = UiTestHarness<Hooked>;

pub fn startup(args: Args, settings: Settings) -> AppStartup {
    AppStartup {
        options: StartupOptions::from_parts_with_keyring(args, None, String::new(), false, false),
        settings,
    }
}

pub fn no_args() -> Args {
    Args {
        repo: None,
        left: None,
        right: None,
        compare_mode: None,
        layout: None,
        renderer: None,
        file_index: None,
        file_path: None,
        open_pr: None,
        link: None,
    }
}

pub fn harness_with(startup: AppStartup) -> Harness {
    let app = TestApp::new(RecordingDispatcher::default(), TITLE).with_startup(startup);
    let themes = ThemePair::default();
    UiTestHarness::with_adapter(
        UiAdapter::new(Hooked(app), TITLE).with_themes(themes.light, themes.dark),
        SIZE,
        2.0,
    )
}

pub fn harness() -> Harness {
    harness_with(startup(no_args(), Settings::default()))
}

pub fn deliver(ui: &mut Harness, event: impl Into<AppEvent>) {
    let workspace = ui.app().0.shared().workspace;
    deliver_at(ui, workspace, event);
}

pub fn deliver_at(ui: &mut Harness, workspace: WorkspaceEpoch, event: impl Into<AppEvent>) {
    ui.send_message(AppMessage::Runtime(EventEnvelope {
        workspace,
        event: event.into(),
    }));
}

pub fn effects(ui: &Harness) -> Vec<Effect> {
    ui.app()
        .0
        .dispatcher()
        .effects
        .iter()
        .map(|(effect, _)| effect.clone())
        .collect()
}

pub fn clear_effects(ui: &mut Harness) {
    ui.app_mut().0.dispatcher_mut().effects.clear();
}
