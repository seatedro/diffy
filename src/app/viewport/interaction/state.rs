use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{
    DiffSession, DiffUpdate, GapId, Reveal, Revision, RowKind, Side, SourcePurpose, SourceRemap,
    UpdateError,
};
use diffy_ui::diff_view::decorator::{DiffDecorator, DiffSources, FileSource, LinePointer};
use diffy_ui::diff_view::presentation::{DiffLayout, DiffMarkers, DiffNumbers, DiffPresentation};
use diffy_ui::{
    DiffAnnotation, DiffEvent, DiffKey, DiffOutcome, DiffSessionViewState, DiffStyle, DiffTarget,
    FindOptions, RevealAlign, SearchDirection, SearchSides, SearchSummary, SourcePoint,
};
use quark_ui::FocusId;
use quark_ui::text_input::{TextEditCommand, TextEditOutcome, TextField};

use super::decorations::{Decorations, HeaderControls, Plain, is_generated};
use super::scopes::SharedScopes;
use super::syntax::{SyntaxWake, ViewportSyntax};
use super::targets::{hunk_target, source_anchor, source_target};
use super::text_compare::TextCompare;
use crate::app::review::ReviewPresentation;
use crate::app::text_targets::{
    SEARCH, TEXT_COMPARE_LEFT, TEXT_COMPARE_RIGHT, TextCx, TextHost, VIEWPORT,
};
use crate::contracts::commands::{CommandDescriptor, CommandSource, ToastUpdate};
use crate::contracts::mutations::MutationsAction;
use crate::contracts::navigation::{SelectedFile, SelectionOrigin};
use crate::contracts::review::CardKey;
use crate::contracts::shared::{
    CompareGeneration, FileKey, FileRevision, HunkTarget, SharedContext, SourceTarget,
};
use crate::contracts::shell::{Controller, Intent, LogicalFocus, Reduction, ShellEffect};
use crate::contracts::viewport::InteractionAction;
use crate::contracts::viewport::InteractionEvent;
use crate::contracts::viewport::interaction::{
    ContextDirection, DiffInput, InstallItem, LineInput, TextCompareSide, TextCompareView,
    ViewportInstall,
};
use crate::contracts::viewport::pipeline::{PreparedDiff, PreparedSegment, SlotUpdate};
use crate::core::compare::{LayoutMode, RendererKind};
use crate::core::keymap::ShortcutCommand;
use crate::platform::persistence::{PersistedViewport, Settings};

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
    pub(super) semantic: bool,
    pub(super) wrap: bool,
    pub(super) wrap_column: u16,
    pub(super) search: SearchBar,
    pub(super) published: Option<SourceTarget>,
    pub(super) shown_file: Option<FileKey>,
    pub(super) syntax: ViewportSyntax,
    pub(super) text_compare: TextCompare,
    pub(super) viewport_height: f32,
    pub(super) persisted: Option<PersistedViewport>,
    pub(super) review_revision: Option<(u64, u32)>,
    review: Option<Rc<ReviewPresentation>>,
    thread_cursor: Option<CardKey>,
    pub(super) viewport_width: f32,
    pub(super) continuous: bool,
    pub(super) source_requests: Vec<(FileKey, SourcePurpose)>,
    pub(super) search_reduction: Reduction,
    pub(super) decorations: Option<Rc<Decorations>>,
    pub(super) plain: Rc<dyn DiffDecorator>,
    pub(super) seeded: HashSet<FileKey>,
    pub(super) scopes: SharedScopes,
    pub(super) scope_revision: u64,
    pub(super) modifiers: LineModifiers,
    pub(super) line_actions: Vec<MutationsAction>,
    pub(super) line_drag: Option<SourceTarget>,
    pub(super) stale_view: Option<f32>,
    pub(super) definition: Option<(SourcePoint, String)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineModifiers {
    pub extend: bool,
    pub toggle: bool,
}

struct Pending {
    key: Option<FileKey>,
    segment: Option<Arc<PreparedSegment>>,
    update: DiffUpdate,
}

impl Pending {
    fn slot(update: DiffUpdate) -> Self {
        Self {
            key: None,
            segment: None,
            update,
        }
    }
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
            semantic: false,
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
            review: None,
            thread_cursor: None,
            viewport_width: 0.0,
            continuous: false,
            source_requests: Vec::new(),
            search_reduction: Reduction::default(),
            decorations: None,
            plain: Rc::new(Plain),
            seeded: HashSet::new(),
            scopes: SharedScopes::default(),
            scope_revision: 0,
            modifiers: LineModifiers::default(),
            line_actions: Vec::new(),
            line_drag: None,
            definition: None,
            stale_view: None,
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

    /// The hunk the diff marks for keys acting on one hunk, among the
    /// files `accept` takes; see [`diffy_ui::diff_view::ViewFrame::target_hunk`].
    pub fn target_hunk(&self, accept: impl Fn(FileSource) -> bool) -> Option<HunkTarget> {
        let frame = self.diff.as_ref()?.frame()?;
        let (unit, hunk) =
            frame.target_hunk(|unit| self.sources.file(unit).is_some_and(&accept))?;
        Some(hunk_target(&self.sources.hunk_at(unit, hunk)?))
    }

    pub fn focused_hunk(&self) -> Option<HunkTarget> {
        let (file, hunk) = self.diff.as_ref()?.focused_hunk()?;
        let unit = self.sources.unit_of(file)?;
        Some(hunk_target(&self.sources.hunk_at(unit, hunk)?))
    }

    pub fn sync_review(&mut self, review: &Rc<ReviewPresentation>) {
        if !self
            .review
            .as_ref()
            .is_some_and(|held| Rc::ptr_eq(held, review))
        {
            self.review = Some(review.clone());
        }
        let width = self.viewport_width;
        let Some(diff) = &mut self.diff else {
            return;
        };
        let stamp = (review.revision(), width.to_bits());
        if width <= 0.0 || self.review_revision == Some(stamp) {
            return;
        }
        self.review_revision = Some(stamp);
        let annotations: Vec<DiffAnnotation> = review
            .annotations()
            .iter()
            .map(|annotation| DiffAnnotation {
                id: annotation.id,
                anchor: source_anchor(&annotation.target),
                revision: annotation.revision,
            })
            .collect();
        if !diff.annotations().map(|(a, _)| a).eq(annotations.iter()) {
            diff.set_annotations(annotations);
        }
        for annotation in review.annotations() {
            if let Some(height) = review.inline_height(&annotation.card, width) {
                diff.set_annotation_height(annotation.id, annotation.revision, height);
            }
        }
    }

    pub(super) fn decorations(
        &mut self,
        inner: Option<Rc<dyn DiffDecorator>>,
        theme: &quark_ui::theme::Theme,
    ) -> Rc<dyn DiffDecorator> {
        let inner = inner.unwrap_or_else(|| self.plain.clone());
        if self.scopes.borrow_mut().poll() {
            self.scope_revision += 1;
        }
        let controls = HeaderControls {
            layout: self.layout,
            wrap: self.wrap,
            search_open: self.search.open,
            scopes: self.scope_revision,
        };
        if let Some(current) = &self.decorations
            && current.matches(&inner, controls, theme)
        {
            return current.clone();
        }
        let decorations = Rc::new(Decorations::new(
            inner,
            controls,
            self.scopes.clone(),
            theme,
        ));
        self.decorations = Some(decorations.clone());
        decorations
    }

    fn request_scopes(&mut self, keys: &[FileKey]) {
        let Some(diff) = &self.diff else {
            return;
        };
        let mut scopes = self.scopes.borrow_mut();
        if !scopes.has_worker() {
            scopes.set_worker(self.syntax.scope_worker());
        }
        for key in keys {
            if let Some(snapshot) = diff.session().file(*key) {
                scopes.request(snapshot);
            }
        }
    }

    fn collapse_generated(&mut self, keys: &[FileKey]) {
        let Some(diff) = &mut self.diff else {
            return;
        };
        for &key in keys {
            if !self.seeded.insert(key) {
                continue;
            }
            let generated = diff
                .session()
                .file(key)
                .map(|snapshot| snapshot.diff.path(0))
                .is_some_and(is_generated);
            if generated {
                diff.set_file_collapsed(key, true);
            }
        }
    }

    pub(super) fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_width = width;
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
            markers: DiffMarkers::Bars,
            numbers: if self.semantic && self.layout == LayoutMode::Unified {
                DiffNumbers::RelevantSide
            } else {
                DiffNumbers::Both
            },
            wrap_column: (self.wrap_column > 0).then_some(self.wrap_column),
            sticky_headers: true,
            ..DiffPresentation::default()
        }
    }

    fn find_options(&self) -> FindOptions {
        FindOptions {
            sides: SearchSides::Shown,
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
        self.seeded.clear();
        self.scopes.borrow_mut().clear();
        self.stale_view = None;
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
        self.install_batch([InstallItem::View(update)], cx)
    }

    pub fn install_slot(&mut self, update: SlotUpdate, cx: &SharedContext) -> Reduction {
        self.install_batch([InstallItem::Slot(update)], cx)
    }

    pub fn install_batch(
        &mut self,
        batch: impl IntoIterator<Item = InstallItem>,
        cx: &SharedContext,
    ) -> Reduction {
        let mut reduction = Reduction::default();
        let mut pending = Vec::new();
        let mut restore = None;
        for install in batch {
            let settles = match &install {
                InstallItem::View(
                    ViewportInstall::Begin { .. } | ViewportInstall::TextCompare { .. },
                ) => false,
                InstallItem::View(ViewportInstall::File { prepared, .. }) => {
                    prepared.generation() >= self.generation
                }
                _ => true,
            };
            if settles && let Some(offset) = self.stale_view.take() {
                let collapsed: Vec<FileKey> = self.diff.as_ref().map_or(Vec::new(), |diff| {
                    let order = diff.session().order();
                    order
                        .iter()
                        .copied()
                        .filter(|&key| diff.is_file_collapsed(key))
                        .collect()
                });
                let seeded = std::mem::take(&mut self.seeded);
                self.reset(self.generation);
                self.seeded = seeded;
                restore = Some((offset, collapsed));
            }
            match install {
                InstallItem::View(ViewportInstall::File { prepared, remap }) => {
                    reduction.merge(self.queue_file(prepared, remap, &mut pending, cx));
                }
                InstallItem::Slot(SlotUpdate::Placeholder { key, placeholder }) => {
                    pending.push(Pending::slot(DiffUpdate::Placeholder {
                        file: key,
                        placeholder,
                    }));
                }
                InstallItem::Slot(SlotUpdate::Evict {
                    key,
                    revision,
                    estimated_lines,
                }) => pending.push(Pending::slot(DiffUpdate::Evict {
                    file: key,
                    revision: Revision(revision.0),
                    estimated_lines,
                })),
                InstallItem::View(update) => {
                    reduction.merge(self.flush(std::mem::take(&mut pending), cx));
                    reduction.merge(self.install_view(update, cx));
                }
            }
        }
        reduction.merge(self.flush(pending, cx));
        if let Some((offset, collapsed)) = restore
            && let Some(diff) = &mut self.diff
        {
            for key in collapsed {
                diff.set_file_collapsed(key, true);
            }
            diff.handle(DiffEvent::ScrollTo(offset));
            reduction.changed = true;
        }
        reduction
    }

    pub fn has_text_selection(&self) -> bool {
        self.diff
            .as_ref()
            .is_some_and(|diff| diff.selection().is_some())
    }

    fn queue_file(
        &mut self,
        prepared: PreparedDiff,
        remap: Option<SourceRemap>,
        pending: &mut Vec<Pending>,
        cx: &SharedContext,
    ) -> Reduction {
        if prepared.generation() < self.generation {
            return Reduction::default();
        }
        let mut reduction = Reduction::default();
        if prepared.generation() > self.generation || self.text_compare.active {
            reduction = self.flush(std::mem::take(pending), cx);
            self.text_compare.active = false;
            self.reset(prepared.generation());
        }
        pending.push(Pending {
            key: Some(prepared.key()),
            segment: prepared.segment().cloned(),
            update: DiffUpdate::Upsert {
                file: prepared.snapshot().clone(),
                remap,
            },
        });
        reduction
    }

    fn flush(&mut self, pending: Vec<Pending>, cx: &SharedContext) -> Reduction {
        if pending.is_empty() {
            return Reduction::default();
        }
        let keys: Vec<Option<FileKey>> = pending.iter().map(|p| p.key).collect();
        let results = self
            .ensure_diff()
            .apply_updates(pending.into_iter().map(|p| (p.update, p.segment)));
        let mut changed = false;
        let mut installed = Vec::new();
        for (key, result) in keys.into_iter().zip(results) {
            match result {
                Ok(_) => {
                    changed = true;
                    if let Some(key) = key {
                        self.failures.remove(&key);
                        self.phase = Phase::Ready;
                        installed.push(key);
                    }
                }
                Err(UpdateError::Stale { .. } | UpdateError::Removed { .. }) => {}
                Err(error) => tracing::warn!(?error, "viewport refused a diff update"),
            }
        }
        if !changed {
            return Reduction::default();
        }
        self.refresh_sources();
        self.collapse_generated(&installed);
        self.request_scopes(&installed);
        if let Some(selected) = cx.selected_file
            && selected.origin != SelectionOrigin::Viewport
            && installed.contains(&selected.key)
        {
            self.reveal_file(selected.key);
        }
        self.sync_solo(cx.selected_file.map(|s| s.key));
        Reduction::changed()
    }

    fn sync_solo(&mut self, selected: Option<FileKey>) -> bool {
        let continuous = self.continuous || self.text_compare.active;
        let Some(diff) = &mut self.diff else {
            return false;
        };
        if continuous {
            return diff.set_solo(None);
        }
        let session = diff.session();
        let live = |key: &FileKey| session.file(*key).is_some();
        let target = selected
            .filter(live)
            .or(diff.solo().filter(live))
            .or_else(|| session.order().iter().copied().find(live));
        let changed = diff.set_solo(target);
        if changed {
            self.shown_file = target;
        }
        changed
    }

    fn step_solo(&mut self, forward: bool) -> Reduction {
        let Some(diff) = &self.diff else {
            return Reduction::default();
        };
        let session = diff.session();
        let order = session.order();
        let at = diff
            .solo()
            .and_then(|key| order.iter().position(|k| *k == key));
        let live = |key: &&FileKey| session.file(**key).is_some();
        let next = match (at, forward) {
            (Some(at), true) => order[at + 1..].iter().find(live),
            (Some(at), false) => order[..at].iter().rev().find(live),
            (None, _) => order.iter().find(live),
        };
        let Some(&key) = next else {
            return Reduction::default();
        };
        self.sync_solo(Some(key));
        Reduction::changed().intent(Intent::SelectFile(Some(SelectedFile {
            key,
            origin: SelectionOrigin::Viewport,
        })))
    }

    pub fn set_modifiers(&mut self, shift: bool, primary: bool) {
        self.modifiers = LineModifiers {
            extend: shift,
            toggle: primary,
        };
    }

    pub fn take_line_actions(&mut self) -> Vec<MutationsAction> {
        std::mem::take(&mut self.line_actions)
    }

    fn line_pointer(&mut self, pointer: LinePointer) -> Reduction {
        let Some(hit) = self
            .diff
            .as_ref()
            .and_then(|diff| diff.line_at(pointer.x, pointer.y))
            .filter(|hit| {
                matches!(
                    hit.kind,
                    RowKind::Added
                        | RowKind::Removed
                        | RowKind::Modified
                        | RowKind::Merged
                        | RowKind::Moved
                        | RowKind::Reflowed
                )
            })
        else {
            return Reduction::default();
        };
        let target = SourceTarget {
            file: hit.file,
            revision: FileRevision(hit.revision.0),
            side: hit.side,
            lines: hit.line..hit.line + 1,
        };
        if pointer.drag && self.line_drag.as_ref() == Some(&target) {
            return Reduction::default();
        }
        let action = match (pointer.drag, self.modifiers) {
            (true, _) => MutationsAction::ExtendLineSelection(target.clone()),
            (false, LineModifiers { toggle: true, .. }) => {
                MutationsAction::ToggleLineSelection(target.clone())
            }
            (false, LineModifiers { extend: true, .. }) => {
                MutationsAction::ExtendLineSelection(target.clone())
            }
            (false, _) => MutationsAction::SelectLines(target.clone()),
        };
        self.line_drag = Some(target);
        self.line_actions.push(action);
        Reduction::changed()
    }

    fn move_to_change(&mut self, steps: i32) -> Reduction {
        let Some(diff) = &mut self.diff else {
            return Reduction::default();
        };
        let mut hit = None;
        for _ in 0..steps.unsigned_abs().min(MAX_REPEAT) {
            match diff.focus_change(steps > 0) {
                Some(next) => hit = Some(next),
                None => break,
            }
        }
        let Some(hit) = hit else {
            return Reduction::default();
        };
        let target = Some(SourceTarget {
            file: hit.file,
            revision: FileRevision(hit.revision.0),
            side: hit.side,
            lines: hit.line..hit.line + 1,
        });
        let mut reduction = Reduction::changed();
        if target != self.published {
            self.published = target.clone();
            reduction.intents.push(Intent::SelectSource(target));
        }
        self.follow(reduction, false)
    }

    pub fn take_source_requests(&mut self) -> Vec<(FileKey, SourcePurpose)> {
        std::mem::take(&mut self.source_requests)
    }

    fn request_sources(&mut self, unit: u32) {
        let Some(file) = self.sources.file(unit) else {
            return;
        };
        let wanted = self.diff.as_ref().is_some_and(|diff| {
            diff.session()
                .source_request(file.id, SourcePurpose::ExpandContext)
                .is_some()
        });
        let request = (file.id, SourcePurpose::ExpandContext);
        if wanted && !self.source_requests.contains(&request) {
            self.source_requests.push(request);
        }
    }

    fn install_view(&mut self, update: ViewportInstall, cx: &SharedContext) -> Reduction {
        match update {
            ViewportInstall::Begin { generation } => {
                if generation < self.generation {
                    return Reduction::default();
                }
                let shown = self.phase == Phase::Ready
                    && !self.text_compare.active
                    && self.diff.as_ref().is_some_and(|d| !d.session().is_empty());
                if shown && generation > self.generation {
                    let offset = self.diff.as_ref().map_or(0.0, |d| d.scroll_offset());
                    self.generation = generation;
                    self.stale_view = self.stale_view.or(Some(offset));
                    return Reduction::default();
                }
                self.text_compare.active = false;
                self.stale_view = None;
                self.reset(generation);
                Reduction::changed()
            }
            ViewportInstall::File { prepared, remap } => {
                let mut pending = Vec::new();
                self.queue_file(prepared, remap, &mut pending, cx);
                self.flush(pending, cx)
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
                if generation != self.generation {
                    return Reduction::default();
                }
                if files.is_empty() && self.phase == Phase::Loading {
                    self.phase = Phase::Ready;
                    return Reduction::changed();
                }
                if self.diff.is_none() {
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
            DiffOutcome::Changed | DiffOutcome::OpenFull { .. } => Reduction::changed(),
            DiffOutcome::Annotate { anchor } => {
                Reduction::changed().intent(Intent::RequestAnnotation(source_target(&anchor)))
            }
            DiffOutcome::Copy(text) if text.is_empty() => Reduction::default(),
            DiffOutcome::Copy(text) => Reduction::default().effect(ShellEffect::SetClipboard(text)),
            DiffOutcome::Definition { point, line } => {
                if !self.text_compare.active {
                    self.definition = Some((point, line));
                }
                Reduction::default()
            }
        }
    }

    pub fn request_definition(&mut self) -> Reduction {
        let outcome = self.key(DiffKey::Definition);
        self.outcome(outcome)
    }

    pub fn take_definition(&mut self) -> Option<(SourcePoint, String)> {
        self.definition.take()
    }

    pub fn hover_at(&self, x: f32, y: f32) -> Option<diffy_ui::HoverHit> {
        if self.text_compare.active {
            return None;
        }
        self.diff.as_ref()?.hover_at(x, y)
    }

    pub fn has_link(&self) -> bool {
        self.diff
            .as_ref()
            .is_some_and(DiffSessionViewState::has_link)
    }

    pub fn set_link(&mut self, link: Option<(SourcePoint, u32)>) -> bool {
        self.diff.as_mut().is_some_and(|diff| diff.set_link(link))
    }

    pub fn caret(&self) -> Option<SourcePoint> {
        self.diff.as_ref()?.caret().map(|(point, _)| point)
    }

    pub fn scroll_offset(&self) -> Option<f32> {
        self.diff.as_ref().map(DiffSessionViewState::scroll_offset)
    }

    pub fn restore_scroll(&mut self, offset: f32) -> Reduction {
        let outcome = match &mut self.diff {
            Some(diff) => diff.handle(DiffEvent::ScrollTo(offset)),
            None => DiffOutcome::Unchanged,
        };
        let reduction = self.outcome(outcome);
        self.follow(reduction, false)
    }

    pub fn reveal_definition(&mut self, point: SourcePoint, len: u32) -> Option<Reduction> {
        let reduction = self.reveal_source(point)?;
        self.diff.as_mut()?.select_source(point, len);
        Some(self.follow(reduction, false))
    }

    fn go_to_thread(&mut self, delta: i32, cx: &SharedContext) -> Reduction {
        let Some(review) = self.review.clone() else {
            return Reduction::default();
        };
        let order = |file: FileKey| {
            cx.manifest
                .as_ref()
                .and_then(|manifest| manifest.files.iter().position(|f| f.key == file))
                .unwrap_or(usize::MAX)
        };
        let mut threads: Vec<_> = review
            .annotations()
            .iter()
            .filter(|annotation| matches!(annotation.card, CardKey::Thread(_)))
            .collect();
        threads.sort_by_key(|annotation| {
            (order(annotation.target.file), annotation.target.lines.start)
        });
        let Ok(count) = i64::try_from(threads.len()) else {
            return Reduction::default();
        };
        if count == 0 {
            return Reduction::default();
        }
        let current = self.thread_cursor.as_ref().and_then(|card| {
            threads
                .iter()
                .position(|annotation| &annotation.card == card)
        });
        let next = match current {
            Some(index) => (index as i64 + i64::from(delta)).rem_euclid(count) as usize,
            None if delta < 0 => threads.len() - 1,
            None => 0,
        };
        let annotation = threads[next];
        self.thread_cursor = Some(annotation.card.clone());
        let lines = &annotation.target.lines;
        let point = SourcePoint {
            file: annotation.target.file,
            side: annotation.target.side,
            line: lines.end.saturating_sub(1).max(lines.start),
            byte: 0,
        };
        let mut reduction = match self.reveal_source(point) {
            Some(reduction) => self.follow(reduction, false),
            None => Reduction::changed(),
        };
        reduction
            .intents
            .push(Intent::FocusReviewCard(Some(annotation.card.clone())));
        reduction
    }

    fn reveal_source(&mut self, point: SourcePoint) -> Option<Reduction> {
        let live = self
            .diff
            .as_ref()
            .is_some_and(|diff| diff.session().file(point.file).is_some());
        if !live || self.text_compare.active {
            return None;
        }
        let mut reduction = Reduction::changed();
        let solo = self.diff.as_ref().and_then(DiffSessionViewState::solo);
        if solo.is_some() && solo != Some(point.file) && self.sync_solo(Some(point.file)) {
            reduction
                .intents
                .push(Intent::SelectFile(Some(SelectedFile {
                    key: point.file,
                    origin: SelectionOrigin::Viewport,
                })));
        }
        let diff = self.diff.as_mut()?;
        if !diff.reveal_target(DiffTarget::Source(point), RevealAlign::Center) {
            return None;
        }
        Some(reduction)
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
        if !self.continuous && !self.text_compare.active {
            match key {
                DiffKey::NextFile => return self.step_solo(true),
                DiffKey::PrevFile => return self.step_solo(false),
                _ => {}
            }
        }
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

    fn rerun_search(&mut self) -> Reduction {
        let options = self.find_options();
        let query = if self.search.open {
            self.search.field.text().to_owned()
        } else {
            String::new()
        };
        let Some(diff) = &mut self.diff else {
            return Reduction::default();
        };
        diff.set_find_query(&query, options);
        if query.is_empty() {
            return Reduction::default();
        }
        self.step_match(SearchDirection::Forward)
    }

    fn step_match(&mut self, direction: SearchDirection) -> Reduction {
        let Some(point) = self.diff.as_mut().and_then(|d| d.next_match(direction)) else {
            return Reduction::default();
        };
        let mut reduction = Reduction::changed();
        let solo = self.diff.as_ref().and_then(DiffSessionViewState::solo);
        if solo.is_some() && solo != Some(point.file) && self.sync_solo(Some(point.file)) {
            if let Some(diff) = &mut self.diff {
                diff.reveal_target(DiffTarget::Source(point), RevealAlign::Center);
            }
            reduction
                .intents
                .push(Intent::SelectFile(Some(SelectedFile {
                    key: point.file,
                    origin: SelectionOrigin::Viewport,
                })));
        }
        self.follow(reduction, false)
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

    fn apply_settings(&mut self, settings: &Settings, startup: bool, cx: &SharedContext) -> bool {
        let mut changed = self.apply_viewport_settings(&settings.viewport, startup);
        let semantic = settings
            .last_compare
            .as_ref()
            .is_some_and(|compare| compare.renderer == RendererKind::Semantic);
        if self.semantic != semantic {
            self.semantic = semantic;
            self.sync_presentation();
            changed = true;
        }
        if self.continuous != settings.continuous_scroll {
            self.continuous = settings.continuous_scroll;
            changed = true;
        }
        changed | self.sync_solo(cx.selected_file.map(|s| s.key))
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

    fn reduce_action(&mut self, action: Self::Action, cx: &SharedContext) -> Reduction {
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
            A::Diff(DiffInput(DiffEvent::Expand(gap, reveal))) => {
                let outcome = match &mut self.diff {
                    Some(diff) => diff.handle(DiffEvent::Expand(gap, reveal)),
                    None => DiffOutcome::Unchanged,
                };
                if outcome == DiffOutcome::Unchanged {
                    self.request_sources(gap.file);
                }
                self.outcome(outcome)
            }
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
            A::Line(LineInput(pointer)) => self.line_pointer(pointer),
            A::ClearSelection => {
                let Some(diff) = &mut self.diff else {
                    return Reduction::default();
                };
                if diff.selection().is_none() {
                    return Reduction::default();
                }
                diff.set_selection(None, Side::New);
                self.follow(Reduction::changed(), true)
            }
            A::MoveRowCursor(steps) => self.move_to_change(steps),
            A::ScrollLines(lines) => {
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
            A::GoToThread(delta) => self.go_to_thread(delta, cx),
            A::AnnotateSelection => match self.key(DiffKey::Annotate) {
                DiffOutcome::Unchanged => Reduction::default().intent(Intent::Toast(
                    ToastUpdate::info("Select a line to comment on."),
                )),
                outcome => self.outcome(outcome),
            },
            A::AnnotateLine(target) => {
                let covering = self
                    .diff
                    .as_ref()
                    .and_then(|diff| diff.selected_lines())
                    .filter(|selected| {
                        selected.file == target.file
                            && selected.side == target.side
                            && selected.lines.contains(&target.lines.start)
                    });
                let anchor = covering.unwrap_or_else(|| source_anchor(&target));
                Reduction::changed().intent(Intent::RequestAnnotation(source_target(&anchor)))
            }
            A::GoToPreviousHunk => self.navigate(DiffKey::PrevHunk),
            A::GoToNextFile => self.navigate(DiffKey::NextFile),
            A::GoToPreviousFile => self.navigate(DiffKey::PrevFile),
            A::OpenSearch => {
                self.search.open = true;
                let mut reduction = Reduction::changed().effect(focus(LogicalFocus::Search));
                reduction.merge(self.rerun_search());
                reduction
            }
            A::CloseSearch => {
                self.search.open = false;
                let mut reduction = Reduction::changed().effect(focus(LogicalFocus::Viewport));
                reduction.merge(self.rerun_search());
                reduction
            }
            A::SearchNext | A::SearchPrevious => {
                let direction = if action == A::SearchNext {
                    SearchDirection::Forward
                } else {
                    SearchDirection::Backward
                };
                self.step_match(direction)
            }
            A::ToggleSearchCaseSensitive => {
                self.search.case_sensitive = !self.search.case_sensitive;
                let mut reduction = Reduction::changed();
                reduction.merge(self.rerun_search());
                reduction
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
            } => {
                let changed = self.expand(file, hunk, direction, amount);
                if !changed && let Some(unit) = self.sources.unit_of(file) {
                    self.request_sources(unit);
                }
                Reduction {
                    changed,
                    ..Reduction::default()
                }
            }
            A::ExpandAllContext => Reduction {
                changed: self.diff.as_mut().is_some_and(|diff| diff.expand_all()),
                ..Reduction::default()
            },
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

    fn apply_intent(&mut self, intent: &Intent, cx: &SharedContext) -> Reduction {
        let changed = match intent {
            Intent::OpenTextCompare => {
                let open = InteractionAction::SetTextCompareView(TextCompareView::Edit);
                return self.reduce_action(open, cx);
            }
            Intent::Startup(startup) => self.apply_settings(&startup.settings, true, cx),
            Intent::SettingsChanged(settings) => self.apply_settings(&settings.settings, false, cx),
            Intent::SelectFile(Some(selected)) if selected.origin != SelectionOrigin::Viewport => {
                let solo = self.sync_solo(Some(selected.key));
                self.reveal_file(selected.key) | solo
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

    fn after_edit(&mut self, target: FocusId, _cx: &SharedContext) -> Reduction {
        if target != SEARCH {
            return Reduction::default();
        }
        std::mem::take(&mut self.search_reduction)
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
            let reduction = self.rerun_search();
            self.search_reduction.merge(reduction);
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
                    let reduction = self.rerun_search();
                    self.search_reduction.merge(reduction);
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
            CommandDescriptor::new("diff.expand_all", "Expand All Context", A::ExpandAllContext)
                .enabled(has_files),
            CommandDescriptor::new("diff.copy", "Copy Selection", A::CopySelection)
                .enabled(selected),
            CommandDescriptor::new("diff.comment", "Comment on Selection", A::AnnotateSelection)
                .shortcut(S::ReviewSelectedLines)
                .enabled(has_files),
            CommandDescriptor::new(
                "diff.text_compare",
                "New Text Compare",
                A::SetTextCompareView(TextCompareView::Edit),
            ),
        ]);
    }
}
