use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{FileDiffSnapshot, Revision, diff_texts};
use diffy_ui::diff_view::decorator::{
    DiffDecorator, HeaderContext, HeaderSlot, LineContext, LineMark, SeparatorContext,
};
use quark::view;
use quark_app::testing::UiTestHarness;
use quark_app::{UiAdapter, UiApp, UiContext, ViewContext};
use quark_ui::FocusId;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome};

use super::{InteractionController, view_shared};
use crate::app::commands::keymap;
use crate::app::shell::Slot;
use crate::app::text_targets::{TextCx, TextHost, focus_id};
use crate::contracts::commands::CommandsAction;
use crate::contracts::shared::{CompareGeneration, FileKey, FileRevision, SharedContext};
use crate::contracts::shell::{
    Action, Controller, Effect, Intent, Reduction, ShellAction, ShellEffect,
};
use crate::contracts::viewport::interaction::ViewportInstall;
use crate::contracts::viewport::{PreparedDiff, ViewportAction};
use crate::platform::persistence::Settings;

pub const GENERATION: CompareGeneration = CompareGeneration(1);

pub fn numbered(lines: std::ops::Range<u32>) -> String {
    lines.map(|i| format!("line {i}\n")).collect()
}

pub fn prepared(id: u64, revision: u64, path: &str, old: &str, new: &str) -> PreparedDiff {
    prepared_at(GENERATION, id, revision, path, old, new)
}

pub fn prepared_at(
    generation: CompareGeneration,
    id: u64,
    revision: u64,
    path: &str,
    old: &str,
    new: &str,
) -> PreparedDiff {
    let doc = diff_texts(Some(path), Some(path), Some(old), Some(new), 3);
    let key = FileKey(id);
    let snapshot = FileDiffSnapshot::new(key, Revision(revision), Arc::new(doc))
        .unwrap_or_else(|error| panic!("fixture snapshot: {error:?}"));
    PreparedDiff::new(key, generation, FileRevision(revision), snapshot)
}

pub fn two_files() -> [PreparedDiff; 2] {
    let base = numbered(0..60);
    let first = base
        .replace("line 10\n", "changed ten\n")
        .replace("line 40\n", "changed forty\n");
    let second = base.replace("line 5\n", "needle five\n");
    [
        prepared(1, 1, "a.rs", &base, &first),
        prepared(2, 1, "b.rs", &base, &second),
    ]
}

#[derive(Default)]
pub struct Recorder {
    pub revision: u64,
    pub separators: RefCell<Vec<SeparatorContextRecord>>,
    pub headers: RefCell<Vec<Option<(FileKey, u64)>>>,
    pub marked: Option<(FileKey, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeparatorContextRecord {
    pub file: Option<(FileKey, u64)>,
    pub hunk: Option<(u32, std::ops::Range<u32>, std::ops::Range<u32>)>,
}

impl DiffDecorator for Recorder {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        if slot == HeaderSlot::Actions {
            self.headers
                .borrow_mut()
                .push(cx.source.map(|s| (s.id, s.revision.0)));
        }
        None
    }

    fn separator(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        self.separators.borrow_mut().push(SeparatorContextRecord {
            file: cx.source.map(|s| (s.id, s.revision.0)),
            hunk: cx
                .hunk
                .as_ref()
                .map(|h| (h.hunk, h.old.clone(), h.new.clone())),
        });
        None
    }

    fn line_mark(&self, cx: &LineContext) -> Option<LineMark> {
        let (file, line) = self.marked?;
        (cx.source.map(|s| s.id) == Some(file) && cx.line == line).then_some(LineMark::Selected)
    }
}

pub struct Host {
    pub interaction: InteractionController,
    pub shared: SharedContext,
    pub decorator: Rc<dyn DiffDecorator>,
    pub effects: Vec<Effect>,
    pub intents: Vec<Intent>,
    pub clock: u64,
}

impl Default for Host {
    fn default() -> Self {
        Self::with_decorator(Rc::new(Recorder::default()))
    }
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_decorator(decorator: Rc<dyn DiffDecorator>) -> Self {
        let mut interaction = InteractionController::default();
        interaction.set_grammars(crate::core::syntax::grammar::Grammars::none());
        Self {
            interaction,
            shared: SharedContext::default(),
            decorator,
            effects: Vec::new(),
            intents: Vec::new(),
            clock: 0,
        }
    }

    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut snapshot = (*self.shared.settings).clone();
        change(&mut snapshot.settings);
        snapshot.revision += 1;
        let snapshot = Rc::new(snapshot);
        self.shared.settings = snapshot.clone();
        self.intent(Intent::SettingsChanged(snapshot));
    }

    pub fn install(&mut self, update: ViewportInstall) {
        let reduction = self.interaction.install(update, &self.shared);
        self.absorb_detached(reduction);
    }

    pub fn intent(&mut self, intent: Intent) {
        let reduction = self.interaction.apply_intent(&intent, &self.shared);
        self.absorb_detached(reduction);
    }

    fn absorb_detached(&mut self, reduction: Reduction) {
        self.effects.extend(reduction.effects);
        self.publish(reduction.intents);
    }

    fn publish(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            match &intent {
                Intent::SelectSource(target) => self.shared.selection = target.clone(),
                Intent::SelectFile(file) => self.shared.selected_file = *file,
                _ => {}
            }
            let reduction = self.interaction.apply_intent(&intent, &self.shared);
            self.effects.extend(reduction.effects);
            self.intents.push(intent);
            self.intents.extend(reduction.intents);
        }
    }

    fn absorb(&mut self, reduction: Reduction, cx: &mut UiContext) {
        for effect in reduction.effects {
            match effect {
                Effect::Shell(ShellEffect::SetClipboard(text)) => {
                    cx.window.set_clipboard_text(&text)
                }
                Effect::Shell(ShellEffect::Focus(focus)) => cx.set_focus(focus.map(focus_id)),
                effect => self.effects.push(effect),
            }
        }
        self.publish(reduction.intents);
    }
}

impl UiApp for Host {
    type Action = Action;
    type Message = ();

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        self.clock = cx.frame.elapsed().as_millis() as u64;
        let (width, height) = cx.frame.size();
        let slot = Slot {
            shared: &self.shared,
            width,
            height,
        };
        let diff = view_shared(&mut self.interaction, &slot, self.decorator.clone(), cx);
        view! { <div w={width} h={height}>{diff}</div> }
    }

    fn update(&mut self, action: Action, cx: &mut UiContext) {
        match action {
            Action::Viewport(ViewportAction::Interaction(action)) => {
                let reduction = self.interaction.reduce_action(action, &self.shared);
                self.absorb(reduction, cx);
            }
            Action::Shell(ShellAction::FocusLogical(focus)) => cx.set_focus(focus.map(focus_id)),
            Action::Commands(CommandsAction::ToggleWrap) => {
                self.update_settings(|s| s.viewport.wrap_enabled = !s.viewport.wrap_enabled)
            }
            _ => {}
        }
    }

    fn edit_text(&mut self, target: FocusId, command: TextEditCommand) -> TextEditOutcome {
        let cx = TextCx {
            shared: &self.shared,
            now_ms: self.clock,
        };
        self.interaction
            .edit_text(target, command, &cx)
            .unwrap_or_default()
    }

    fn set_preedit(&mut self, target: FocusId, text: String, cursor: Option<(usize, usize)>) {
        self.interaction.set_preedit(target, text, cursor);
    }
}

pub fn harness(host: Host, size: (f32, f32)) -> UiTestHarness<Host> {
    let adapter = UiAdapter::new(host, "Test").with_key_bindings(keymap::key_bindings(&[], &[]));
    let mut ui = UiTestHarness::with_adapter(adapter, size, 1.0);
    ui.set_accessibility_active(true);
    ui.frame();
    ui
}
