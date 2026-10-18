use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use diffy_diff::{DiffSession, DiffUpdate, GapId, Reveal, Revision, UpdateError};
use diffy_ui::diff_view::decorator::{DiffDecorator, DiffSources};
use diffy_ui::diff_view::presentation::{DiffLayout, DiffPresentation};
use diffy_ui::{
    DiffAnnotation, DiffEvent, DiffKey, DiffOutcome, DiffSessionViewState, DiffStyle, DiffTarget,
    FindOptions, RevealAlign, SearchDirection, SearchSummary, SourcePoint,
};
use quark_ui::FocusId;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use super::syntax::{SyntaxWake, ViewportSyntax};
use super::targets::{hunk_target, source_anchor, source_target};
use super::text_compare::TextCompare;
use crate::app::review::ReviewPresentation;
use crate::app::text_targets::{
    SEARCH, TEXT_COMPARE_LEFT, TEXT_COMPARE_RIGHT, TextCx, TextHost, VIEWPORT,
};
use crate::contracts::commands::{CommandDescriptor, CommandSource};
use crate::contracts::navigation::{SelectedFile, SelectionOrigin};
use crate::contracts::shared::{
    CompareGeneration, FileKey, HunkTarget, SharedContext, SourceTarget,
};
use crate::contracts::shell::{Controller, Intent, LogicalFocus, Reduction, ShellEffect};
use crate::contracts::viewport::InteractionAction;
use crate::contracts::viewport::InteractionEvent;
use crate::contracts::viewport::interaction::{
    ContextDirection, DiffInput, TextCompareSide, TextCompareView, ViewportInstall,
};
use crate::core::compare::LayoutMode;
use crate::core::keymap::ShortcutCommand;
use crate::platform::persistence::PersistedViewport;

pub const DIFF_ID: &str = "diffy.diff";
const MAX_REPEAT: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Default)]
pub struct SearchBar {
    pub open: bool,
    pub field: TextField,
    pub case_sensitive: bool,
}

pub struct InteractionController {
    pub(super) diff: Option<DiffSessionViewState>,
    pub(super) sources: Rc<DiffSources>,
    pub(super) generation: CompareGeneration,
    pub(super) phase: Phase,
    pub(super) failures: HashMap<FileKey, String>,
    pub(super) order_revision: u64,
    pub(super) layout: LayoutMode,
    pub(super) wrap: bool,
    pub(super) wrap_column: u16,
    pub(super) search: SearchBar,
    pub(super) published: Option<SourceTarget>,
    pub(super) shown_file: Option<FileKey>,
    pub(super) syntax: ViewportSyntax,
    pub(super) text_compare: TextCompare,
    pub(super) viewport_height: f32,
    pub(super) persisted: Option<PersistedViewport>,
    pub(super) review_revision: Option<u64>,
}

impl Default for InteractionController {
    fn default() -> Self {
        Self {
            diff: None,
            sources: Rc::default(),
            generation: CompareGeneration(0),
            phase: Phase::Idle,
            failures: HashMap::new(),
            order_revision: 0,
            layout: LayoutMode::Unified,
            wrap: false,
            wrap_column: 0,
            search: SearchBar::default(),
            published: None,
            shown_file: None,
            syntax: ViewportSyntax::default(),
            text_compare: TextCompare::default(),
            viewport_height: 0.0,
            persisted: None,
            review_revision: None,
        }
    }
}

impl fmt::Debug for InteractionController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InteractionController")
            .field("files", &self.diff.as_ref().map(|d| d.session().len()))
            .field("generation", &self.generation)
            .field("phase", &self.phase)
            .field("layout", &self.layout)
            .field("wrap", &self.wrap)
            .field("search", &self.search)
            .field("text_compare", &self.text_compare)
            .finish_non_exhaustive()
    }
}

impl InteractionController {
    pub fn set_syntax_wake(&mut self, wake: SyntaxWake) {
        self.syntax.set_wake(wake.clone());
        if let Some(diff) = &mut self.diff {
            diff.set_syntax_wake(move || wake());
        }
    }

    pub fn set_grammars(&mut self, grammars: crate::core::syntax::grammar::Grammars) {
        self.syntax.set_grammars(grammars);
    }

    pub fn diff(&self) -> Option<&DiffSessionViewState> {
        self.diff.as_ref()
    }

    pub fn diff_mut(&mut self) -> Option<&mut DiffSessionViewState> {
        self.diff.as_mut()
    }

    pub fn sources(&self) -> &Rc<DiffSources> {
        &self.sources
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn search(&self) -> &SearchBar {
        &self.search
    }

    pub fn search_summary(&self) -> SearchSummary {
        self.diff
            .as_ref()
            .map(DiffSessionViewState::search_summary)
            .unwrap_or_default()
    }

    pub fn text_compare(&self) -> &TextCompare {
        &self.text_compare
    }

    pub fn text_compare_mut(&mut self) -> &mut TextCompare {
        &mut self.text_compare
    }

    pub fn failure(&self, key: FileKey) -> Option<&str> {
        self.failures.get(&key).map(String::as_str)
    }

    pub fn wrap(&self) -> bool {
        self.wrap
    }

    pub fn layout(&self) -> LayoutMode {
        self.layout
    }

    pub fn selection_target(&self) -> Option<SourceTarget> {
        self.diff
            .as_ref()?
            .selected_lines()
            .as_ref()
            .map(source_target)
    }

    pub fn focused_hunk(&self) -> Option<HunkTarget> {
        let (file, hunk) = self.diff.as_ref()?.focused_hunk()?;
        let unit = self.sources.unit_of(file)?;
        Some(hunk_target(&self.sources.hunk_at(unit, hunk)?))
    }

    pub fn sync_review(&mut self, review: &ReviewPresentation) {
        let Some(diff) = &mut self.diff else {
            return;
        };
        let revision = review.revision();
        if self.review_revision == Some(revision) {
            return;
        }
        self.review_revision = Some(revision);
        let annotations: Vec<DiffAnnotation> = review
            .annotations()
            .iter()
            .map(|annotation| DiffAnnotation {
                id: annotation.id,
                anchor: source_anchor(&annotation.target),
                revision: annotation.revision,
            })
            .collect();
        if diff.annotations().map(|(a, _)| a).eq(annotations.iter()) {
            return;
        }
        diff.set_annotations(annotations);
    }

    pub(super) fn set_viewport_height(&mut self, height: f32) {
        self.viewport_height = height;
    }

    pub(super) fn style(&self, font_size: f32) -> DiffStyle {
        DiffStyle {
            font_size,
            wrap: self.wrap,
            ..DiffStyle::default()
        }
    }

    fn presentation(&self) -> DiffPresentation {
        DiffPresentation {
            layout: match self.layout {
                LayoutMode::Unified => DiffLayout::Unified,
                LayoutMode::Split => DiffLayout::Split,
            },
            wrap_column: (self.wrap_column > 0).then_some(self.wrap_column),
            sticky_headers: true,
            ..DiffPresentation::default()
        }
    }

    fn find_options(&self) -> FindOptions {
        FindOptions {
            case_sensitive: self.search.case_sensitive,
            ..FindOptions::default()
        }
    }

    fn ensure_diff(&mut self) -> &mut DiffSessionViewState {
        let presentation = self.presentation();
        let options = self.find_options();
        let query = self
            .search
            .open
            .then(|| self.search.field.text().to_owned());
        let syntax = &mut self.syntax;
        self.diff.get_or_insert_with(|| {
            let mut diff = DiffSessionViewState::new(DIFF_ID, VIEWPORT, DiffSession::new())
                .with_label("Diff")
                .with_scrollbar_auto_hide();
            diff.set_presentation(presentation);
            syntax.attach(&mut diff);
            if let Some(query) = query.filter(|q| !q.is_empty()) {
                diff.set_find_query(&query, options);
            }
            diff
        })
    }

    fn reset(&mut self, generation: CompareGeneration) {
        self.generation = generation;
        self.diff = None;
        self.sources = Rc::default();
        self.failures.clear();
        self.published = None;
        self.shown_file = None;
        self.review_revision = None;
        self.phase = Phase::Loading;
    }

    fn refresh_sources(&mut self) {
        if let Some(diff) = &self.diff {
            self.sources = Rc::new(diff.sources());
        }
    }

    fn apply(&mut self, update: DiffUpdate) -> bool {
        match self.ensure_diff().apply_update(update) {
            Ok(_) => {
                self.refresh_sources();
                true
            }
            Err(UpdateError::Stale { .. } | UpdateError::Removed { .. }) => false,
            Err(error) => {
                tracing::warn!(?error, "viewport refused a diff update");
                false
            }
        }
    }

    pub fn install(&mut self, update: ViewportInstall, cx: &SharedContext) -> Reduction {
        match update {
            ViewportInstall::Begin { generation } => {
                if generation < self.generation {
                    return Reduction::default();
                }
                self.text_compare.active = false;
                self.reset(generation);
                Reduction::changed()
            }
            ViewportInstall::File { prepared, remap } => {
                if prepared.generation() < self.generation {
                    return Reduction::default();
                }
                if prepared.generation() > self.generation || self.text_compare.active {
                    self.text_compare.active = false;
                    self.reset(prepared.generation());
                }
                let key = prepared.key();
                let installed = self.apply(DiffUpdate::Upsert {
                    file: prepared.snapshot().clone(),
                    remap,
                });
                if !installed {
                    return Reduction::default();
                }
                self.failures.remove(&key);
                self.phase = Phase::Ready;
                if let Some(selected) = cx.selected_file
                    && selected.key == key
                    && selected.origin != SelectionOrigin::Viewport
                {
                    self.reveal_file(key);
                }
                Reduction::changed()
            }
            ViewportInstall::Failed {
                generation,
                key,
                message,
                ..
            } => {
                if generation != self.generation {
                    return Reduction::default();
                }
                if self.diff.as_ref().is_none_or(|d| d.session().is_empty()) {
                    self.phase = Phase::Failed(message.clone());
                }
                self.failures.insert(key, message);
                Reduction::changed()
            }
            ViewportInstall::Remove { key, revision } => {
                if self.diff.is_none() {
                    return Reduction::default();
                }
                let changed = self.apply(DiffUpdate::Remove {
                    file: key,
                    revision: Revision(revision.0),
                });
                Reduction {
                    changed,
                    ..Reduction::default()
                }
            }
            ViewportInstall::Order { generation, files } => {
                if generation != self.generation || self.diff.is_none() {
                    return Reduction::default();
                }
                self.order_revision += 1;
                let changed = self.apply(DiffUpdate::Order {
                    revision: Revision(self.order_revision),
                    files,
                });
                Reduction {
                    changed,
                    ..Reduction::default()
                }
            }
            ViewportInstall::TextCompare { prepared } => {
                if !self.text_compare.pending
                    || prepared.generation() != self.text_compare.generation
                {
                    return Reduction::default();
                }
                self.text_compare.pending = false;
                self.text_compare.active = true;
                self.text_compare.view = TextCompareView::Diff;
                self.reset(self.generation);
                self.apply(DiffUpdate::Upsert {
                    file: prepared.snapshot().clone(),
                    remap: None,
                });
                self.phase = Phase::Ready;
                Reduction::changed().effect(focus(LogicalFocus::Viewport))
            }
        }
    }

    fn reveal_file(&mut self, key: FileKey) -> bool {
        let Some(diff) = &mut self.diff else {
            return false;
        };
        let revealed = diff.reveal_target(DiffTarget::File(key), RevealAlign::Top);
        if revealed {
            self.shown_file = Some(key);
        }
        revealed
    }

    fn key(&mut self, key: DiffKey) -> DiffOutcome {
        match &mut self.diff {
            Some(diff) => diff.handle(DiffEvent::Key(key)),
            None => DiffOutcome::Unchanged,
        }
    }

    fn outcome(&mut self, outcome: DiffOutcome) -> Reduction {
        match outcome {
            DiffOutcome::Unchanged => Reduction::default(),
            DiffOutcome::Changed | DiffOutcome::OpenFull { .. } | DiffOutcome::Annotate { .. } => {
                Reduction::changed()
            }
            DiffOutcome::Copy(text) if text.is_empty() => Reduction::default(),
            DiffOutcome::Copy(text) => Reduction::default().effect(ShellEffect::SetClipboard(text)),
        }
    }

    fn follow(&mut self, mut reduction: Reduction, select: bool) -> Reduction {
        if select {
            let target = self.selection_target();
            if target != self.published {
                self.published = target.clone();
                reduction.intents.push(Intent::SelectSource(target));
            }
        }
        let top = self.diff.as_ref().and_then(DiffSessionViewState::top_file);
        if top.is_some() && top != self.shown_file {
            self.shown_file = top;
            if let Some(key) = top {
                reduction
                    .intents
                    .push(Intent::SelectFile(Some(SelectedFile {
                        key,
                        origin: SelectionOrigin::Viewport,
                    })));
            }
        }
        reduction
    }

    fn navigate(&mut self, key: DiffKey) -> Reduction {
        let outcome = self.key(key);
        let mut reduction = self.outcome(outcome);
        if let Some(hunk) = self
            .focused_hunk()
            .filter(|_| matches!(key, DiffKey::NextHunk | DiffKey::PrevHunk))
        {
            let target = Some(hunk.source);
            if target != self.published {
                self.published = target.clone();
                reduction.intents.push(Intent::SelectSource(target));
            }
        }
        self.follow(reduction, false)
    }

    fn scroll_by(&mut self, delta: f32) -> Reduction {
        let Some(diff) = &mut self.diff else {
            return Reduction::default();
        };
        let to = (diff.scroll_offset() + delta).max(0.0);
        let outcome = diff.handle(DiffEvent::ScrollTo(to));
        let reduction = self.outcome(outcome);
        self.follow(reduction, false)
    }

    fn rerun_search(&mut self) {
        let options = self.find_options();
        let query = if self.search.open {
            self.search.field.text().to_owned()
        } else {
            String::new()
        };
        if let Some(diff) = &mut self.diff {
            diff.set_find_query(&query, options);
            if !query.is_empty() {
                diff.next_match(SearchDirection::Forward);
            }
        }
    }

    fn expand(
        &mut self,
        file: FileKey,
        hunk: u32,
        direction: ContextDirection,
        amount: u32,
    ) -> bool {
        let Some(unit) = self.sources.unit_of(file) else {
            return false;
        };
        let Some(global) = self.sources.global_hunk(unit, hunk) else {
            return false;
        };
        let (gap, reveal) = match direction {
            ContextDirection::Above => (Some(global), Reveal::Up),
            ContextDirection::Below => (self.sources.global_hunk(unit, hunk + 1), Reveal::Down),
            ContextDirection::All => (Some(global), Reveal::All),
        };
        let Some(diff) = &mut self.diff else {
            return false;
        };
        diff.reveal(
            GapId {
                file: unit,
                hunk: gap,
            },
            reveal,
            amount,
        )
    }

    fn apply_viewport_settings(&mut self, viewport: &PersistedViewport, startup: bool) -> bool {
        let previous = self.persisted.replace(viewport.clone());
        let old = previous.as_ref().filter(|_| !startup);
        let before = (self.wrap, self.wrap_column, self.layout);
        if old.is_none_or(|old| old.wrap_enabled != viewport.wrap_enabled) {
            self.wrap = viewport.wrap_enabled;
        }
        if old.is_none_or(|old| old.wrap_column != viewport.wrap_column) {
            self.wrap_column = u16::try_from(viewport.wrap_column).unwrap_or(u16::MAX);
        }
        if old.is_none_or(|old| old.layout != viewport.layout) {
            self.layout = viewport.layout;
        }
        let changed = before != (self.wrap, self.wrap_column, self.layout);
        if changed {
            self.sync_presentation();
        }
        changed
    }

    fn sync_presentation(&mut self) {
        let presentation = self.presentation();
        if let Some(diff) = &mut self.diff {
            diff.set_presentation(presentation);
        }
    }

    fn text_side(target: FocusId) -> Option<TextCompareSide> {
        match target {
            TEXT_COMPARE_LEFT => Some(TextCompareSide::Left),
            TEXT_COMPARE_RIGHT => Some(TextCompareSide::Right),
            _ => None,
        }
    }

    fn repeat(&mut self, key: DiffKey, times: u32) -> Reduction {
        let mut reduction = Reduction::default();
        for _ in 0..times {
            let outcome = self.key(key);
            reduction.merge(self.outcome(outcome));
        }
        self.follow(reduction, false)
    }
}

impl Controller for InteractionController {
    type Action = InteractionAction;
    type Event = InteractionEvent;

    fn reduce_action(&mut self, action: Self::Action, _cx: &SharedContext) -> Reduction {
        use InteractionAction as A;
        match action {
            A::Diff(DiffInput(DiffEvent::Key(
                key @ (DiffKey::NextHunk
                | DiffKey::PrevHunk
                | DiffKey::NextFile
                | DiffKey::PrevFile
                | DiffKey::Home
                | DiffKey::End),
            ))) => self.navigate(key),
            A::Diff(DiffInput(event)) => {
                let select = matches!(
                    event,
                    DiffEvent::Release | DiffEvent::Key(DiffKey::SelectAll)
                );
                let outcome = match &mut self.diff {
                    Some(diff) => diff.handle(event),
                    None => DiffOutcome::Unchanged,
                };
                let reduction = self.outcome(outcome);
                self.follow(reduction, select)
            }
            A::ScrollLines(lines) | A::MoveRowCursor(lines) => {
                let key = if lines < 0 {
                    DiffKey::LineUp
                } else {
                    DiffKey::LineDown
                };
                self.repeat(key, lines.unsigned_abs().min(MAX_REPEAT))
            }
            A::ScrollHalfPage(pages) => {
                let half = (self.viewport_height * 0.5).max(1.0);
                self.scroll_by(half * pages as f32)
            }
            A::ScrollPages(pages) => {
                let key = if pages < 0 {
                    DiffKey::PageUp
                } else {
                    DiffKey::PageDown
                };
                self.repeat(key, pages.unsigned_abs().min(MAX_REPEAT))
            }
            A::ScrollToTop => self.navigate(DiffKey::Home),
            A::ScrollToBottom => self.navigate(DiffKey::End),
            A::GoToNextHunk => self.navigate(DiffKey::NextHunk),
            A::GoToPreviousHunk => self.navigate(DiffKey::PrevHunk),
            A::GoToNextFile => self.navigate(DiffKey::NextFile),
            A::GoToPreviousFile => self.navigate(DiffKey::PrevFile),
            A::OpenSearch => {
                self.search.open = true;
                self.rerun_search();
                Reduction::changed().effect(focus(LogicalFocus::Search))
            }
            A::CloseSearch => {
                self.search.open = false;
                self.rerun_search();
                Reduction::changed().effect(focus(LogicalFocus::Viewport))
            }
            A::SearchNext | A::SearchPrevious => {
                let direction = if action == A::SearchNext {
                    SearchDirection::Forward
                } else {
                    SearchDirection::Backward
                };
                let moved = self
                    .diff
                    .as_mut()
                    .and_then(|diff| diff.next_match(direction))
                    .is_some();
                let reduction = Reduction {
                    changed: moved,
                    ..Reduction::default()
                };
                self.follow(reduction, false)
            }
            A::ToggleSearchCaseSensitive => {
                self.search.case_sensitive = !self.search.case_sensitive;
                self.rerun_search();
                Reduction::changed()
            }
            A::CopySelection => {
                let text = self
                    .diff
                    .as_ref()
                    .map(DiffSessionViewState::selected_text)
                    .unwrap_or_default();
                self.outcome(DiffOutcome::Copy(text))
            }
            A::SetSourceSelection(target) => {
                let (Some(target), Some(diff)) = (target, &mut self.diff) else {
                    return Reduction::default();
                };
                let point = SourcePoint {
                    file: target.file,
                    side: target.side,
                    line: target.lines.start,
                    byte: 0,
                };
                Reduction {
                    changed: diff.reveal_target(DiffTarget::Source(point), RevealAlign::Center),
                    ..Reduction::default()
                }
            }
            A::ExpandContext {
                file,
                hunk,
                direction,
                amount,
            } => Reduction {
                changed: self.expand(file, hunk, direction, amount),
                ..Reduction::default()
            },
            A::ExpandAllContext => Reduction {
                changed: self.diff.as_mut().is_some_and(|diff| diff.expand_all()),
                ..Reduction::default()
            },
            A::SetLayout(layout) => {
                if self.layout == layout {
                    return Reduction::default();
                }
                self.layout = layout;
                self.sync_presentation();
                Reduction::changed()
            }
            A::SetTextCompareView(view) => {
                self.text_compare.active = true;
                self.text_compare.view = view;
                let target = match view {
                    TextCompareView::Edit => LogicalFocus::TextCompareLeft,
                    TextCompareView::Diff => LogicalFocus::Viewport,
                };
                Reduction::changed().effect(focus(target))
            }
            A::SwapTextCompareSides => {
                self.text_compare.swap();
                Reduction::changed()
            }
            A::ClearTextCompareSide(side) => {
                self.text_compare.clear(side);
                Reduction::changed()
            }
            A::SetTextCompareLanguage(language) => {
                let changed = self.text_compare.set_language(language);
                let reduction = Reduction::changed();
                if changed && self.text_compare.view == TextCompareView::Diff {
                    let effect = self.text_compare.request(self.generation, self.layout);
                    return reduction.effect(effect);
                }
                reduction
            }
            A::ScrollTextCompare(side, lines) => {
                let editor = self.text_compare.editor_mut(side);
                editor.scroll(lines as f32 * editor.scroll_line_height_px());
                Reduction::changed()
            }
            A::ToggleTextCompareLanguageMenu => {
                self.text_compare.language_menu = !self.text_compare.language_menu;
                Reduction::changed()
            }
            A::CompareText => {
                self.text_compare.active = true;
                let effect = self.text_compare.request(self.generation, self.layout);
                Reduction::changed().effect(effect)
            }
        }
    }

    fn reduce_event(&mut self, _event: Self::Event, _cx: &SharedContext) -> Reduction {
        Reduction::default()
    }

    fn apply_intent(&mut self, intent: &Intent, _cx: &SharedContext) -> Reduction {
        let changed = match intent {
            Intent::Startup(startup) => {
                self.apply_viewport_settings(&startup.settings.viewport, true)
            }
            Intent::SettingsChanged(settings) => {
                self.apply_viewport_settings(&settings.settings.viewport, false)
            }
            Intent::SelectFile(Some(selected)) if selected.origin != SelectionOrigin::Viewport => {
                self.reveal_file(selected.key)
            }
            Intent::GrammarAvailable(_) => match &mut self.diff {
                Some(diff) => {
                    diff.refresh_syntax();
                    true
                }
                None => false,
            },
            Intent::ActivateWorkspace { .. } => {
                self.text_compare.active = false;
                self.reset(self.generation);
                self.phase = Phase::Idle;
                true
            }
            _ => false,
        };
        Reduction {
            changed,
            ..Reduction::default()
        }
    }
}

impl TextHost for InteractionController {
    fn owns_text(&self, target: FocusId) -> bool {
        target == SEARCH || Self::text_side(target).is_some()
    }

    fn edit_text(
        &mut self,
        target: FocusId,
        command: TextEditCommand,
        cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        if let Some(side) = Self::text_side(target) {
            return Some(self.text_compare.edit(side, command, cx.now_ms));
        }
        let outcome = self.search.field.apply_at(command, cx.now_ms);
        if outcome.text_changed {
            self.rerun_search();
        }
        Some(outcome)
    }

    fn set_preedit(
        &mut self,
        target: FocusId,
        text: String,
        cursor: Option<(usize, usize)>,
    ) -> bool {
        match Self::text_side(target) {
            Some(side) => self.text_compare.editor_mut(side).set_preedit(text, cursor),
            None => self.search.field.set_preedit(text, cursor),
        }
        true
    }

    fn set_text_value(
        &mut self,
        target: FocusId,
        value: String,
        _cx: &TextCx,
    ) -> Option<TextEditOutcome> {
        let text_changed = match Self::text_side(target) {
            Some(side) => {
                let editor = self.text_compare.editor_mut(side);
                let changed = editor.text() != value;
                editor.set_text(&value);
                if changed {
                    self.text_compare.invalidate();
                }
                changed
            }
            None => {
                let changed = self.search.field.text() != value;
                self.search.field.set_text(value);
                if changed {
                    self.rerun_search();
                }
                changed
            }
        };
        Some(TextEditOutcome {
            text_changed,
            selection_changed: true,
            ..TextEditOutcome::default()
        })
    }
}

fn focus(target: LogicalFocus) -> ShellEffect {
    ShellEffect::Focus(Some(target))
}

impl CommandSource for InteractionController {
    fn commands(&self, _cx: &SharedContext, out: &mut Vec<CommandDescriptor>) {
        use InteractionAction as A;
        use ShortcutCommand as S;
        let has_files = self.diff.as_ref().is_some_and(|d| !d.session().is_empty());
        let matches = self.search.open && self.search_summary().matches > 0;
        let selected = self.diff.as_ref().is_some_and(|d| d.selection().is_some());
        let unified = self.layout == LayoutMode::Unified;
        out.extend([
            CommandDescriptor::new("diff.find", "Find in Diff", A::OpenSearch)
                .shortcut(S::OpenSearch)
                .enabled(has_files),
            CommandDescriptor::new("diff.find_next", "Next Match", A::SearchNext).enabled(matches),
            CommandDescriptor::new("diff.find_previous", "Previous Match", A::SearchPrevious)
                .enabled(matches),
            CommandDescriptor::new("diff.next_hunk", "Next Hunk", A::GoToNextHunk)
                .shortcut(S::NextHunk)
                .enabled(has_files),
            CommandDescriptor::new("diff.previous_hunk", "Previous Hunk", A::GoToPreviousHunk)
                .shortcut(S::PreviousHunk)
                .enabled(has_files),
            CommandDescriptor::new("diff.next_file", "Next File in Diff", A::GoToNextFile)
                .enabled(has_files),
            CommandDescriptor::new(
                "diff.previous_file",
                "Previous File in Diff",
                A::GoToPreviousFile,
            )
            .enabled(has_files),
            CommandDescriptor::new("diff.top", "Go to Top", A::ScrollToTop).enabled(has_files),
            CommandDescriptor::new("diff.bottom", "Go to Bottom", A::ScrollToBottom)
                .shortcut(S::GoToBottom)
                .enabled(has_files),
            CommandDescriptor::new(
                "diff.unified",
                "Unified View",
                A::SetLayout(LayoutMode::Unified),
            )
            .shortcut(S::UnifiedView)
            .enabled(!unified),
            CommandDescriptor::new("diff.split", "Split View", A::SetLayout(LayoutMode::Split))
                .shortcut(S::SplitView)
                .enabled(unified),
            CommandDescriptor::new("diff.expand_all", "Expand All Context", A::ExpandAllContext)
                .enabled(has_files),
            CommandDescriptor::new("diff.copy", "Copy Selection", A::CopySelection)
                .enabled(selected),
            CommandDescriptor::new(
                "diff.text_compare",
                "New Text Compare",
                A::SetTextCompareView(TextCompareView::Edit),
            ),
        ]);
    }
}
