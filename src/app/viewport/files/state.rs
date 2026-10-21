use std::sync::Arc;

use diffy_diff::DiffDocument;
use diffy_ui::diff_view::presentation::{DiffLayout, DiffNumbers, DiffPresentation, FileHeaders};
use diffy_ui::{DiffEvent, DiffKey, DiffViewState};

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

pub struct FileTab {
    pub id: TabId,
    pub file: FileAt,
    pub state: TabState,
}

#[derive(Default)]
pub struct FilesController {
    tabs: Vec<FileTab>,
    active: Option<TabId>,
    next_id: u64,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
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
            FilesAction::Input(input) => {
                let Some(FileTab {
                    state: TabState::Ready(view),
                    ..
                }) = self.active_tab_mut()
                else {
                    return Reduction::default();
                };
                view.handle(input.0);
                Reduction::changed()
            }
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
