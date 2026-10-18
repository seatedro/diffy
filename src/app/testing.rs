use quark_app::testing::UiTestHarness;
use quark_app::{UiAdapter, UiSender};
use quark_ui::theme::ThemeMode;

use crate::app::shell::{DiffyApp, EffectDispatcher};
use crate::app::theme::ThemePair;
use crate::contracts::shared::WorkspaceEpoch;
use crate::contracts::shell::{AppEvent, AppMessage, Effect, EventEnvelope};

pub const TITLE: &str = "Diffy";
pub const SIZE: (f32, f32) = (1280.0, 800.0);

#[derive(Debug, Default)]
pub struct RecordingDispatcher {
    pub effects: Vec<(Effect, WorkspaceEpoch)>,
    pub attached: bool,
}

impl EffectDispatcher for RecordingDispatcher {
    fn attach(&mut self, _sender: UiSender<AppMessage>) {
        self.attached = true;
    }

    fn dispatch(&mut self, effect: Effect, workspace: WorkspaceEpoch) {
        self.effects.push((effect, workspace));
    }
}

pub type TestApp = DiffyApp<RecordingDispatcher>;
pub type Harness = UiTestHarness<TestApp>;

pub fn app() -> TestApp {
    DiffyApp::new(RecordingDispatcher::default(), TITLE)
}

pub fn adapter(themes: ThemePair, mode: Option<ThemeMode>) -> UiAdapter<TestApp> {
    crate::app::adapter(app(), TITLE, themes, mode)
}

pub fn harness() -> Harness {
    harness_at(SIZE, 2.0)
}

pub fn harness_at(size: (f32, f32), scale: f32) -> Harness {
    UiTestHarness::with_adapter(adapter(ThemePair::default(), None), size, scale)
}

pub fn deliver(ui: &mut Harness, workspace: WorkspaceEpoch, event: impl Into<AppEvent>) {
    ui.send_message(AppMessage::Runtime(EventEnvelope {
        workspace,
        event: event.into(),
    }));
}

pub fn deliver_current(ui: &mut Harness, event: impl Into<AppEvent>) {
    let workspace = ui.app().shared().workspace;
    deliver(ui, workspace, event);
}
