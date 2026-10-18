//! A diff view over a [`DiffSession`]: files that change while they are
//! shown, keeping the reader's place.
//!
//! Each file revision is its own segment with its own expansion and
//! projection, so an update re-projects only its file. The row table is
//! then rebuilt from the segments, which walks row keys but does not diff,
//! project, or shape other files, and rows whose file did not change keep
//! their shaped paint.
//!
//! An update that carries a [`diffy_diff::SourceRemap`] keeps the top source
//! line, the selection, and annotation anchors on the lines the remap
//! proves unchanged. A replaced top line moves to the nearest surviving
//! line; a selection touching replaced text is cleared; an anchor on
//! replaced lines becomes outdated. Without a remap, the file's selection
//! is cleared and its annotations become outdated. Context the reader
//! expanded stays expanded where the remap proves its lines unchanged;
//! without a remap it collapses again.
//!
//! Rendering and interaction are the static view's: [`diff_session_view`]
//! draws with [`super::diff_view`], and events go through
//! [`DiffSessionViewState::handle`]. With syntax on
//! ([`DiffSessionViewState::enable_syntax`]) every file revision is
//! highlighted as it arrives, and sides an update leaves unchanged keep
//! their colors.

use std::rc::Rc;

use diffy_diff::{
    ComparisonOptions, ContextPolicy, DiffLimits, DiffSession, DiffUpdate, GapId, InlineOptions,
    LineMap, Mode, Reveal, Side, SourceRemap, UpdateError, UpdateOutcome,
};
use quark::selection::{Selection, SelectionPoint};
use quark_text::{LayoutCache, TextSystem};
use quark_ui::element::{AnyElement, ScrollHandle};
use quark_ui::theme::Theme;
use quark_ui::{Action, FocusId};

use diffy_diff::RowKind;
use diffy_syntax::{GrammarStore, HighlightWorker};

use super::annotations::{DiffAnchor, DiffAnnotation};
use super::decorator::DiffDecorator;
use super::navigation::{DiffTarget, FileId, RevealAlign, SourcePoint};
use super::prepared::{AnnotationId, DiffPreviewLimit, ViewFrame};
use super::presentation::{DiffAppearance, DiffPresentation};
use super::search::{FindOptions, SearchDirection, SearchSummary};
use super::selection::{CopyContent, block_key, decode_key};
use super::state::{Anchor, NONE, Segment, source_line, store_index};
use super::{DiffEvent, DiffOutcome, DiffStyle, DiffViewState};
use quark_components::CollectionEnv;

/// App-owned state of a session diff view. See the [module docs](self).
pub struct DiffSessionViewState {
    view: DiffViewState,
    session: DiffSession,
}

/// Forwards methods whose meaning is the same in both views.
macro_rules! forward {
    (mut $( fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty; )*) => {
        $(
            #[doc = concat!("See [`DiffViewState::", stringify!($name), "`].")]
            pub fn $name(&mut self $(, $arg: $ty)*) -> $ret {
                self.view.$name($($arg),*)
            }
        )*
    };
    ($( fn $name:ident($($arg:ident: $ty:ty),*) -> $ret:ty; )*) => {
        $(
            #[doc = concat!("See [`DiffViewState::", stringify!($name), "`].")]
            pub fn $name(&self $(, $arg: $ty)*) -> $ret {
                self.view.$name($($arg),*)
            }
        )*
    };
}

impl DiffSessionViewState {
    /// `id` names cache entries and accessibility ids (unique in the
    /// window); `focus` is the view's keyboard focus target.
    pub fn new(id: &'static str, focus: FocusId, session: DiffSession) -> Self {
        let mut view = DiffViewState::empty(id, focus);
        view.files.session = true;
        let mut state = Self { view, session };
        for index in 0..state.session.len() {
            let snapshot = state.session.at(index).expect("ordered file").clone();
            state.insert_segment(index as usize, snapshot.id, snapshot);
        }
        state.view.rebuild_rows(None);
        state
    }

    pub fn with_label(mut self, label: &'static str) -> Self {
        self.view.label = label;
        self
    }

    pub fn with_style(mut self, style: DiffStyle) -> Self {
        self.view.set_style(style);
        self
    }

    pub fn with_scrollbar_auto_hide(mut self) -> Self {
        self.view.scrollbar_auto_hide = true;
        self
    }

    pub fn session(&self) -> &DiffSession {
        &self.session
    }

    forward! {
        fn mode() -> Mode;
        fn style() -> DiffStyle;
        fn presentation() -> DiffPresentation;
        fn limits() -> DiffLimits;
        fn comparison() -> ComparisonOptions;
        fn inline_options() -> InlineOptions;
        fn hidden_whitespace_changes(file: FileId) -> u32;
        fn scroll_offset() -> f32;
        fn content_height() -> f32;
        fn selection() -> Option<Selection>;
        fn frame() -> Option<&Rc<ViewFrame>>;
        fn horizontal_scroll(side: Side) -> &ScrollHandle;
        fn wants_frame() -> bool;
        fn selected_text() -> String;
        fn copy(content: CopyContent) -> Option<String>;
        fn search_summary() -> SearchSummary;
        fn focused_target() -> Option<DiffTarget>;
        fn point_at(x: f32, y: f32) -> Option<SelectionPoint>;
        fn is_file_collapsed(file: FileId) -> bool;
    }

    forward! { mut
        fn set_mode(mode: Mode) -> ();
        fn set_style(style: DiffStyle) -> ();
        fn set_presentation(presentation: DiffPresentation) -> ();
        fn set_appearance(appearance: DiffAppearance) -> ();
        fn set_preview_limit(limit: Option<DiffPreviewLimit>) -> ();
        fn set_limits(limits: DiffLimits) -> ();
        fn set_comparison(options: ComparisonOptions) -> ();
        fn set_inline_options(options: InlineOptions) -> ();
        fn set_context_policy(policy: ContextPolicy) -> ();
        fn set_viewport(width: f32, height: f32) -> ();
        fn set_selection(selection: Option<Selection>, side: Side) -> ();
        fn select_all() -> ();
        fn collapse(gap: GapId) -> bool;
        fn expand_all() -> bool;
        fn reveal(gap: GapId, reveal: Reveal, amount: u32) -> bool;
        fn reveal_target(target: DiffTarget, align: RevealAlign) -> bool;
        fn set_find_query(query: &str, options: FindOptions) -> ();
        fn next_match(direction: SearchDirection) -> Option<SourcePoint>;
        fn set_annotations(annotations: Vec<DiffAnnotation>) -> ();
        fn set_annotation_height(id: AnnotationId, revision: u64, height: f32) -> bool;
        fn handle(event: DiffEvent) -> DiffOutcome;
        fn set_file_collapsed(file: FileId, collapsed: bool) -> bool;
        fn enable_syntax(store: GrammarStore) -> ();
        fn enable_syntax_shared(worker: &HighlightWorker, store: GrammarStore) -> ();
        fn finish_syntax() -> bool;
    }

    /// See [`DiffViewState::set_syntax_wake`].
    pub fn set_syntax_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.view.set_syntax_wake(wake);
    }

    /// Each annotation with whether it is outdated, in the order given.
    pub fn annotations(&self) -> impl Iterator<Item = (&DiffAnnotation, bool)> {
        self.view.annotations()
    }

    /// See [`DiffViewState::prepare`].
    pub fn prepare(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        now_ms: u64,
    ) {
        self.view.prepare(text, layouts, scale, now_ms);
    }

    /// Applies `update` to the session and shows the result. A refused
    /// update changes nothing; [`UpdateError::Stale`] and
    /// [`UpdateError::Removed`] are late arrivals the app can drop.
    pub fn apply_update(&mut self, update: DiffUpdate) -> Result<UpdateOutcome, UpdateError> {
        let outcome = self.session.apply(update)?;
        let anchor = self.view.anchor();
        let anchor = match &outcome {
            UpdateOutcome::Inserted { index } => {
                let snapshot = self.session.at(*index).expect("inserted file").clone();
                self.insert_segment(*index as usize, snapshot.id, snapshot);
                anchor
            }
            UpdateOutcome::Replaced { index, remap, .. } => {
                self.replace_segment(*index as usize, remap.as_ref(), anchor)
            }
            UpdateOutcome::Hydrated { index } => {
                self.replace_segment(*index as usize, None, anchor)
            }
            UpdateOutcome::Removed { index } => {
                if let Some(index) = index {
                    self.remove_segment(*index as usize);
                }
                anchor
            }
            UpdateOutcome::Reordered => {
                self.reorder_segments();
                anchor
            }
        };
        let view = &mut self.view;
        // Line numbers may need another digit.
        view.metrics = None;
        view.rerun_search();
        view.rebuild_rows(anchor);
        Ok(outcome)
    }

    /// Adds a segment for a new file at `index`, with a fresh slot.
    fn insert_segment(&mut self, index: usize, id: FileId, snapshot: diffy_diff::FileDiffSnapshot) {
        let view = &mut self.view;
        let slot = view.files.slot_ids.len() as u32;
        view.files.slot_ids.push(id);
        view.files.id_slots.insert(id, slot);
        view.files.slot_seg.push(NONE);
        view.generations += 1;
        let mut segment = Segment::new(
            snapshot.diff.clone(),
            slot,
            view.generations,
            view.mode,
            view.context_policy,
            view.comparison,
        );
        segment.revision = snapshot.revision.0;
        view.segments.insert(index, segment);
        self.index_slots();
        self.view.recheck_annotations(id);
        self.view.request_slot_syntax(index);
    }

    /// Swaps segment `index` for the session's current revision of its
    /// file, carrying the selection, the scroll anchor, and annotations
    /// across through `remap`. A hydration (`remap` of `None`, same
    /// revision) carries them by source line, which it keeps. Returns the
    /// anchor to restore.
    fn replace_segment(
        &mut self,
        index: usize,
        remap: Option<&SourceRemap>,
        anchor: Option<Anchor>,
    ) -> Option<Anchor> {
        let slot = self.view.segments[index].slot;
        let id = self.view.files.slot_ids[slot as usize];
        let snapshot = self.session.file(id).expect("replaced file").clone();
        let view = &mut self.view;
        let old_doc = view.segments[index].doc.clone();
        let same_revision = view.segments[index].revision == snapshot.revision.0;
        let new_doc = snapshot.diff.clone();
        // A line of the old document as a store index of the new one.
        let carry = |side: Side, store: u32| -> Option<u32> {
            let line = source_line(&old_doc, 0, side, store);
            let line = match remap {
                Some(remap) => match remap.map_line(side, line) {
                    LineMap::Kept(line) => line,
                    LineMap::Replaced { .. } => return None,
                },
                None if same_revision => line,
                None => return None,
            };
            store_index(&new_doc, 0, side, line)
        };

        // The selection, when it touches this file.
        if let Some(s) = view.selection {
            let moved = |point: SelectionPoint| -> Option<Option<SelectionPoint>> {
                let (side, unit, store) = decode_key(point.block);
                if unit != slot {
                    return Some(Some(point));
                }
                let index = carry(side, store)?;
                Some(Some(SelectionPoint::new(
                    block_key(side, unit, index),
                    point.byte,
                )))
            };
            let range_kept = match (decode_key(s.anchor.block), decode_key(s.focus.block)) {
                ((sa, ua, ia), (sb, ub, ib)) if ua == slot && ub == slot && sa == sb => {
                    let (a, b) = (
                        source_line(&old_doc, 0, sa, ia),
                        source_line(&old_doc, 0, sa, ib),
                    );
                    match remap {
                        Some(remap) => remap.map_range(sa, a.min(b)..a.max(b) + 1).is_some(),
                        None => same_revision,
                    }
                }
                _ => true,
            };
            view.selection = match (moved(s.anchor), moved(s.focus)) {
                (Some(Some(a)), Some(Some(f))) if range_kept => Some(Selection::new(a, f)),
                _ => None,
            };
        }

        // The scroll anchor, when it shows a line of this file.
        let anchor = anchor.map(|mut a| {
            if let Some((unit, side, store)) = a.line
                && unit == slot
            {
                let line = source_line(&old_doc, 0, side, store);
                let line = match remap.map(|r| r.map_line(side, line)) {
                    Some(LineMap::Kept(line)) => Some(line),
                    Some(LineMap::Replaced { at }) => Some(at),
                    None if same_revision => Some(line),
                    None => None,
                };
                let store = line.and_then(|line| {
                    // A replacement running to the end lands on the last
                    // line.
                    let last = new_doc.text(0, side).line_count().saturating_sub(1);
                    store_index(&new_doc, 0, side, line.min(last))
                });
                a.line = store.map(|store| (unit, side, store));
                if store.is_none() {
                    a.delta = 0.0;
                }
            }
            a
        });

        view.generations += 1;
        let mut segment = Segment::new(
            new_doc,
            slot,
            view.generations,
            view.mode,
            view.context_policy,
            view.comparison,
        );
        segment.revision = snapshot.revision.0;
        if let Some(remap) = remap {
            carry_expansion(&view.segments[index], &mut segment, remap, view.mode);
        }
        view.segments[index] = segment;
        view.request_slot_syntax(index);
        if let Some((unit, side, store)) = anchor.and_then(|a| a.line)
            && unit == slot
        {
            // Keep the anchored line on screen if its context folded.
            view.reveal_line(index, 0, side, store);
        }

        // Annotations of this file.
        let to = snapshot.revision;
        for entry in view.annotations.entries_mut() {
            let a = &mut entry.annotation.anchor;
            if a.file != id || a.revision == to {
                continue;
            }
            let carried = match remap {
                Some(remap) if a.revision == remap.from => remap.map_range(a.side, a.lines.clone()),
                _ => None,
            };
            match carried {
                Some(lines) => {
                    a.lines = lines;
                    a.revision = to;
                    entry.outdated = false;
                }
                None => entry.outdated = true,
            }
        }
        if same_revision {
            // A hydration: anchors keep their source lines.
            view.recheck_annotations(id);
        }
        anchor
    }

    fn remove_segment(&mut self, index: usize) {
        let view = &mut self.view;
        let slot = view.segments[index].slot;
        view.segments.remove(index);
        view.drop_slot_syntax(slot);
        if let Some(s) = view.selection {
            let touches = |p: SelectionPoint| decode_key(p.block).1 == slot;
            if touches(s.anchor) || touches(s.focus) {
                view.selection = None;
            }
        }
        self.index_slots();
    }

    /// Puts the segments in the session's order.
    fn reorder_segments(&mut self) {
        let view = &mut self.view;
        let mut segments: Vec<Option<Segment>> = view.segments.drain(..).map(Some).collect();
        let position = |slot: u32, segments: &[Option<Segment>]| {
            segments
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| s.slot == slot))
        };
        for &id in self.session.order() {
            let Some(&slot) = view.files.id_slots.get(&id) else {
                continue;
            };
            if let Some(at) = position(slot, &segments) {
                view.segments
                    .push(segments[at].take().expect("unmoved segment"));
            }
        }
        self.index_slots();
    }

    /// Points each slot at its segment's position.
    fn index_slots(&mut self) {
        let files = &mut self.view.files;
        files.slot_seg.fill(NONE);
        for (position, segment) in self.view.segments.iter().enumerate() {
            files.slot_seg[segment.slot as usize] = position as u32;
        }
    }

    /// An anchor on `lines` of `file`'s current revision, when they exist.
    pub fn anchor_for(
        &self,
        file: FileId,
        side: Side,
        lines: std::ops::Range<u32>,
    ) -> Option<DiffAnchor> {
        self.view.anchor_for(file, side, lines)
    }
}

/// Reveals in `new` the gap lines `old` showed revealed whose text
/// `remap` proves unchanged, so an update does not fold context the
/// reader opened. Reveals run from the gap's ends, as expanding does.
fn carry_expansion(old: &Segment, new: &mut Segment, remap: &SourceRemap, mode: Mode) {
    let p = &old.projection;
    // New-side store indices, in `new`, of lines `old` showed from gaps.
    let mut shown: Vec<u32> = (0..p.len())
        .filter(|&row| p.kind[row as usize] == RowKind::Context && p.hunk[row as usize] == NONE)
        .filter_map(|row| {
            let index = p.line(row, Side::New)?;
            let line = source_line(&old.doc, 0, Side::New, index);
            match remap.map_line(Side::New, line) {
                LineMap::Kept(line) => store_index(&new.doc, 0, Side::New, line),
                LineMap::Replaced { .. } => None,
            }
        })
        .collect();
    if shown.is_empty() {
        return;
    }
    shown.sort_unstable();
    let is_shown = |index: u32| shown.binary_search(&index).is_ok();
    let mut changed = false;
    for gap in new.projection.gaps.clone() {
        let lines = gap.new_start..gap.new_start + gap.hidden;
        let top = lines.clone().take_while(|&i| is_shown(i)).count() as u32;
        let bottom = lines.rev().take_while(|&i| is_shown(i)).count() as u32;
        if top > 0 {
            changed |= new.expansion.reveal(&new.doc, gap.id, Reveal::Down, top);
        }
        if bottom > 0 && top < gap.hidden && gap.id.hunk.is_some() {
            changed |= new.expansion.reveal(&new.doc, gap.id, Reveal::Up, bottom);
        }
    }
    if changed {
        new.rebuild(mode);
    }
}

/// The session diff at its viewport size, as of the last
/// [`DiffSessionViewState::prepare`]; see [`super::diff_view`].
pub fn diff_session_view(
    state: &mut DiffSessionViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    super::diff_view(&mut state.view, theme, env, on_event)
}

/// [`diff_session_view`] with the app's `decorator`; see
/// [`super::diff_view_with`].
pub fn diff_session_view_with(
    state: &mut DiffSessionViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
    decorator: Option<Rc<dyn DiffDecorator>>,
) -> AnyElement {
    super::diff_view_with(&mut state.view, theme, env, on_event, decorator)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use diffy_diff::{FileDiffSnapshot, FileId, Revision, diff_texts};
    use quark_text::{LayoutCache, TextSystem};
    use quark_ui::FocusId;

    use super::*;

    fn numbered(lines: std::ops::Range<u32>) -> String {
        lines.map(|i| format!("line {i}\n")).collect()
    }

    /// Revision `rev` of file 1, one line of context.
    fn snap(rev: u64, old: &str, new: &str) -> FileDiffSnapshot {
        let doc = diff_texts(Some("f.txt"), Some("f.txt"), Some(old), Some(new), 1);
        FileDiffSnapshot::new(FileId(1), Revision(rev), Arc::new(doc)).unwrap()
    }

    /// New-side source lines of the prepared rows, in order.
    fn shown(state: &mut DiffSessionViewState, text: &mut TextSystem) -> Vec<u32> {
        state.prepare(text, &mut LayoutCache::default(), 1.0, 0);
        state
            .frame()
            .unwrap()
            .rows
            .iter()
            .filter_map(|r| r.paint.source_lines[1])
            .collect()
    }

    // Catches an update folding context the reader opened: lines the
    // remap proves unchanged stay revealed, around the new hunk too.
    #[test]
    fn opened_context_stays_open_across_an_update_elsewhere() {
        let base = numbered(0..40);
        let first = base.replace("line 20\n", "twenty\n");
        let second = first.replace("line 35\n", "thirty-five\n");
        let (a, b) = (snap(1, &base, &first), snap(2, &base, &second));
        let remap = SourceRemap::between(&a, &b).unwrap();
        let mut session = DiffSession::new();
        session
            .apply(DiffUpdate::Upsert {
                file: a,
                remap: None,
            })
            .unwrap();
        let mut state = DiffSessionViewState::new("test.diff", FocusId::new(1), session);
        state.set_viewport(600.0, 4000.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        shown(&mut state, &mut text);
        let gap = state
            .frame()
            .unwrap()
            .rows
            .iter()
            .find_map(|r| r.paint.gap)
            .unwrap();
        assert!(state.reveal(gap, Reveal::All, 0));
        let opened = shown(&mut state, &mut text);
        assert_eq!(opened[..3], [0, 1, 2]);

        state
            .apply_update(DiffUpdate::Upsert {
                file: b,
                remap: Some(remap),
            })
            .unwrap();

        // Lines 0 to 19 stay above the edited line 20.
        assert_eq!(shown(&mut state, &mut text)[..20], opened[..20]);
    }
}
