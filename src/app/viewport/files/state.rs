use std::sync::Arc;

use diffy_diff::DiffDocument;
use diffy_diff::{FileId, Side};
use diffy_ui::diff_view::presentation::{DiffLayout, DiffNumbers, DiffPresentation, FileHeaders};
use diffy_ui::{
    DiffEvent, DiffKey, DiffOutcome, DiffTarget, DiffViewState, RevealAlign, SourcePoint,
};

use crate::app::text_targets::VIEWPORT;
use crate::contracts::shared::SharedContext;
use crate::contracts::shell::{Intent, LogicalFocus, Reduction, ShellEffect};
use crate::contracts::viewport::InteractionAction;
use crate::contracts::viewport::files::{
    FileAt, FileLoadRequest, FilesAction, FilesEffect, FilesEvent, TabId,
};
use crate::core::syntax::Grammars;

const FILE_VIEW_ID: &str = "diffy.file";
const MAX_REPEAT: u32 = 64;

pub enum TabState {
    Loading,
    Ready(Box<DiffViewState>),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reveal {
    Source { line: u32, column: u32, len: u32 },
    Scroll(f32),
}

pub struct FileTab {
    pub id: TabId,
    pub file: FileAt,
    pub state: TabState,
    pub reveal: Option<Reveal>,
}

impl FileTab {
    pub fn apply_reveal(&mut self) {
        let TabState::Ready(view) = &mut self.state else {
            return;
        };
        match self.reveal.take() {
            Some(Reveal::Source { line, column, len }) => {
                let point = SourcePoint {
                    file: FileId(0),
                    side: Side::New,
                    line,
                    byte: column,
                };
                view.reveal_target(DiffTarget::Source(point), RevealAlign::Center);
                view.select_source(point, len);
            }
            Some(Reveal::Scroll(offset)) => {
                view.handle(DiffEvent::ScrollTo(offset));
            }
            None => {}
        }
    }
}

#[derive(Default)]
pub struct FilesController {
    tabs: Vec<FileTab>,
    active: Option<TabId>,
    next_id: u64,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    definition: Option<(FileAt, SourcePoint, String)>,
}

impl std::fmt::Debug for FilesController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesController")
            .field("tabs", &self.tabs.len())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl FilesController {
    pub fn set_syntax_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake = Some(wake);
    }

    pub fn tabs(&self) -> &[FileTab] {
        &self.tabs
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut FileTab> {
        let active = self.active?;
        self.tabs.iter_mut().find(|tab| tab.id == active)
    }

    pub fn active_place(&self) -> Option<(FileAt, f32, Option<SourcePoint>)> {
        let active = self.active?;
        let tab = self.tabs.iter().find(|tab| tab.id == active)?;
        let (scroll, caret) = match &tab.state {
            TabState::Ready(view) => (view.scroll_offset(), view.caret().map(|(point, _)| point)),
            TabState::Loading | TabState::Failed(_) => (0.0, None),
        };
        Some((tab.file.clone(), scroll, caret))
    }

    pub fn take_definition(&mut self) -> Option<(FileAt, SourcePoint, String)> {
        self.definition.take()
    }

    pub fn request_definition(&mut self) -> Reduction {
        self.input(DiffEvent::Key(DiffKey::Definition))
    }

    pub fn open_at(&mut self, file: FileAt, reveal: Reveal, cx: &SharedContext) -> Reduction {
        let mut reduction = self.open(file.clone(), cx);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.file == file) {
            tab.reveal = Some(reveal);
            reduction.changed = true;
        }
        reduction
    }

    pub fn hover_at(&self, x: f32, y: f32) -> Option<(FileAt, diffy_ui::HoverHit)> {
        let active = self.active?;
        let tab = self.tabs.iter().find(|tab| tab.id == active)?;
        let TabState::Ready(view) = &tab.state else {
            return None;
        };
        Some((tab.file.clone(), view.hover_at(x, y)?))
    }

    pub fn set_link(&mut self, link: Option<(SourcePoint, u32)>) -> bool {
        let active = self.active;
        let mut changed = false;
        for tab in &mut self.tabs {
            if let TabState::Ready(view) = &mut tab.state {
                let link = if Some(tab.id) == active { link } else { None };
                changed |= view.set_link(link);
            }
        }
        changed
    }

    pub fn has_link(&self) -> bool {
        self.active_tab().is_some_and(|tab| match &tab.state {
            TabState::Ready(view) => view.has_link(),
            TabState::Loading | TabState::Failed(_) => false,
        })
    }

    fn active_tab(&self) -> Option<&FileTab> {
        let active = self.active?;
        self.tabs.iter().find(|tab| tab.id == active)
    }

    pub fn show_diff(&mut self) -> Reduction {
        self.activate(None)
    }

    fn input(&mut self, event: DiffEvent) -> Reduction {
        let Some(tab) = self.active_tab_mut() else {
            return Reduction::default();
        };
        let TabState::Ready(view) = &mut tab.state else {
            return Reduction::default();
        };
        match view.handle(event) {
            DiffOutcome::Definition { point, line } => {
                self.definition = Some((tab.file.clone(), point, line));
                Reduction::default()
            }
            DiffOutcome::Copy(text) if !text.is_empty() => {
                Reduction::default().effect(ShellEffect::SetClipboard(text))
            }
            _ => Reduction::changed(),
        }
    }

    fn position(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn open(&mut self, file: FileAt, cx: &SharedContext) -> Reduction {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.file == file) {
            return self.activate(Some(tab.id));
        }
        let Some(repository) = cx.repository.as_ref() else {
            return Reduction::default();
        };
        self.next_id += 1;
        let id = TabId(self.next_id);
        self.tabs.push(FileTab {
            id,
            file: file.clone(),
            state: TabState::Loading,
            reveal: None,
        });
        self.active = Some(id);
        Reduction::changed()
            .effect(FilesEffect::Load(Box::new(FileLoadRequest {
                tab: id,
                repo_path: repository.snapshot.path.clone(),
                file,
            })))
            .effect(focus_viewport())
    }

    fn activate(&mut self, id: Option<TabId>) -> Reduction {
        if id.is_some_and(|id| self.position(id).is_none()) || self.active == id {
            return Reduction::default();
        }
        self.active = id;
        Reduction::changed().effect(focus_viewport())
    }

    fn close(&mut self, id: TabId) -> Reduction {
        let Some(index) = self.position(id) else {
            return Reduction::default();
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = index
                .checked_sub(1)
                .and_then(|i| self.tabs.get(i))
                .or_else(|| self.tabs.get(index))
                .map(|tab| tab.id);
        }
        Reduction::changed().effect(focus_viewport())
    }

    fn step(&mut self, delta: isize) -> Reduction {
        let order: Vec<Option<TabId>> = std::iter::once(None)
            .chain(self.tabs.iter().map(|tab| Some(tab.id)))
            .collect();
        let current = order.iter().position(|id| *id == self.active).unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(order.len() as isize) as usize;
        self.activate(order[next])
    }

    fn loaded(&mut self, id: TabId, document: Arc<DiffDocument>) -> Reduction {
        let wake = self.wake.clone();
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return Reduction::default();
        };
        if !matches!(tab.state, TabState::Loading) {
            return Reduction::default();
        }
        let document = Arc::try_unwrap(document).unwrap_or_else(|shared| (*shared).clone());
        let mut view = DiffViewState::new(FILE_VIEW_ID, VIEWPORT, document);
        view.set_presentation(DiffPresentation {
            layout: DiffLayout::Unified,
            numbers: DiffNumbers::RelevantSide,
            headers: FileHeaders::Hidden,
            ..DiffPresentation::default()
        });
        view.expand_all();
        if let Some(wake) = wake {
            view.set_syntax_wake(move || wake());
        }
        view.enable_syntax(Grammars::shared().store());
        tab.state = TabState::Ready(Box::new(view));
        Reduction::changed()
    }

    pub fn reduce_action(&mut self, action: FilesAction, cx: &SharedContext) -> Reduction {
        match action {
            FilesAction::Open(file) => self.open(file, cx),
            FilesAction::Activate(id) => self.activate(Some(id)),
            FilesAction::ShowDiff => self.activate(None),
            FilesAction::Close(id) => self.close(id),
            FilesAction::NextTab => self.step(1),
            FilesAction::PreviousTab => self.step(-1),
            FilesAction::Input(input) => self.input(input.0),
        }
    }

    pub fn redirect(&mut self, action: &InteractionAction) -> Option<Reduction> {
        let (key, times) = match *action {
            InteractionAction::ScrollPages(pages) | InteractionAction::ScrollHalfPage(pages) => (
                if pages < 0 {
                    DiffKey::PageUp
                } else {
                    DiffKey::PageDown
                },
                pages.unsigned_abs().clamp(1, MAX_REPEAT),
            ),
            InteractionAction::MoveRowCursor(rows) => (
                if rows < 0 {
                    DiffKey::LineUp
                } else {
                    DiffKey::LineDown
                },
                rows.unsigned_abs().clamp(1, MAX_REPEAT),
            ),
            InteractionAction::ScrollToTop => (DiffKey::Home, 1),
            InteractionAction::ScrollToBottom => (DiffKey::End, 1),
            InteractionAction::GoToNextHunk | InteractionAction::GoToPreviousHunk => {
                return Some(Reduction::default());
            }
            _ => return None,
        };
        let Some(FileTab {
            state: TabState::Ready(view),
            ..
        }) = self.active_tab_mut()
        else {
            return self.active.map(|_| Reduction::default());
        };
        for _ in 0..times {
            view.handle(DiffEvent::Key(key));
        }
        Some(Reduction::changed())
    }

    pub fn reduce_event(&mut self, event: FilesEvent) -> Reduction {
        match event {
            FilesEvent::Loaded { tab, document } => self.loaded(tab, document),
            FilesEvent::Failed { tab, message } => {
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab) else {
                    return Reduction::default();
                };
                tab.state = TabState::Failed(message);
                Reduction::changed()
            }
        }
    }

    pub fn apply_intent(&mut self, intent: &Intent) -> Reduction {
        match intent {
            Intent::ActivateWorkspace { .. } if !self.tabs.is_empty() => {
                self.tabs.clear();
                self.active = None;
                Reduction::changed()
            }
            _ => Reduction::default(),
        }
    }
}

fn focus_viewport() -> ShellEffect {
    ShellEffect::Focus(Some(LogicalFocus::Viewport))
}
