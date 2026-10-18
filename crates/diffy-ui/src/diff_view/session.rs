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
//! ([`DiffSessionViewState::enable_syntax`]) file revisions are
//! highlighted once they are visible, and sides an update leaves unchanged
//! keep their colors.

use std::sync::Arc;

use diffy_diff::{
    ComparisonOptions, ContextPolicy, DiffLimits, DiffSession, DiffUpdate, GapId, InlineOptions,
    LineMap, Mode, Reveal, Side, Slot, SlotState, SourceRemap, UpdateError, UpdateOutcome,
};
use quark::selection::{Selection, SelectionPoint};
use quark_ui::FocusId;

use diffy_diff::RowKind;

use super::navigation::FileId;
use super::prepared_update::{Preparation, PreparedSegment, segment_for};

struct SlotParts {
    doc: Arc<diffy_diff::DiffDocument>,
    revision: diffy_diff::Revision,
    pending: Option<(SlotState, u32)>,
}
use super::selection::{block_key, decode_key};
#[cfg(doc)]
use super::session_interaction::diff_session_view;
use super::state::{Anchor, NONE, Segment, source_line, store_index};
use super::{DiffStyle, DiffViewState};

/// App-owned state of a session diff view. See the [module docs](self).
pub struct DiffSessionViewState {
    pub(super) view: DiffViewState,
    pub(super) session: DiffSession,
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

pub(super) use forward;

impl DiffSessionViewState {
    /// `id` names cache entries and accessibility ids (unique in the
    /// window); `focus` is the view's keyboard focus target.
    pub fn new(id: &'static str, focus: FocusId, session: DiffSession) -> Self {
        let mut view = DiffViewState::empty(id, focus);
        view.files.session = true;
        let mut state = Self { view, session };
        for index in 0..state.session.len() {
            state.insert_segment(index as usize, None);
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
        fn limits() -> DiffLimits;
        fn comparison() -> ComparisonOptions;
        fn inline_options() -> InlineOptions;
        fn hidden_whitespace_changes(file: FileId) -> u32;
    }

    forward! { mut
        fn set_mode(mode: Mode) -> ();
        fn set_limits(limits: DiffLimits) -> ();
        fn set_comparison(options: ComparisonOptions) -> ();
        fn set_inline_options(options: InlineOptions) -> ();
        fn set_context_policy(policy: ContextPolicy) -> ();
        fn collapse(gap: GapId) -> bool;
        fn expand_all() -> bool;
        fn reveal(gap: GapId, reveal: Reveal, amount: u32) -> bool;
    }

    /// Applies `update` to the session and shows the result. A refused
    /// update changes nothing; [`UpdateError::Stale`] and
    /// [`UpdateError::Removed`] are late arrivals the app can drop.
    pub fn apply_update(&mut self, update: DiffUpdate) -> Result<UpdateOutcome, UpdateError> {
        self.apply(update, None)
    }

    pub fn preparation(&self) -> Preparation {
        Preparation {
            mode: self.view.mode,
            policy: self.view.context_policy,
            comparison: self.view.comparison,
        }
    }

    pub fn install_prepared(
        &mut self,
        prepared: Arc<PreparedSegment>,
        remap: Option<SourceRemap>,
    ) -> Result<UpdateOutcome, UpdateError> {
        let file = prepared.snapshot().clone();
        self.apply(DiffUpdate::Upsert { file, remap }, Some(prepared))
    }

    pub fn apply_updates(
        &mut self,
        updates: impl IntoIterator<Item = (DiffUpdate, Option<Arc<PreparedSegment>>)>,
    ) -> Vec<Result<UpdateOutcome, UpdateError>> {
        let mut anchor = self.view.anchor();
        let mut changed = false;
        let results = updates
            .into_iter()
            .map(|(update, prepared)| {
                let (outcome, next) = self.apply_one(update, prepared, anchor)?;
                anchor = next;
                changed = true;
                Ok(outcome)
            })
            .collect();
        if changed {
            self.finish(anchor);
        }
        results
    }

    pub fn set_solo(&mut self, file: Option<FileId>) -> bool {
        let slot = file.and_then(|file| self.view.files.id_slots.get(&file).copied());
        if file.is_some() && slot.is_none() || slot == self.view.files.solo {
            return false;
        }
        let anchor = self.view.anchor();
        self.view.files.solo = slot;
        self.view.rebuild_rows(anchor);
        true
    }

    pub fn solo(&self) -> Option<FileId> {
        let slot = self.view.files.solo?;
        self.view.files.slot_ids.get(slot as usize).copied()
    }

    pub fn slot_state(&self, file: FileId) -> Option<(SlotState, u32)> {
        match self.session.slot(file)? {
            Slot::Pending(placeholder) => Some((placeholder.state, placeholder.estimated_lines)),
            Slot::Live(_) => None,
        }
    }

    fn apply(
        &mut self,
        update: DiffUpdate,
        prepared: Option<Arc<PreparedSegment>>,
    ) -> Result<UpdateOutcome, UpdateError> {
        let anchor = self.view.anchor();
        let (outcome, anchor) = self.apply_one(update, prepared, anchor)?;
        self.finish(anchor);
        Ok(outcome)
    }

    fn apply_one(
        &mut self,
        update: DiffUpdate,
        prepared: Option<Arc<PreparedSegment>>,
        anchor: Option<Anchor>,
    ) -> Result<(UpdateOutcome, Option<Anchor>), UpdateError> {
        let outcome = self.session.apply(update)?;
        let anchor = match &outcome {
            UpdateOutcome::Inserted { index } => {
                self.insert_segment(*index as usize, prepared);
                anchor
            }
            UpdateOutcome::Replaced { index, remap, .. } => {
                self.replace_segment(*index as usize, remap.as_ref(), anchor, prepared)
            }
            UpdateOutcome::Loaded { index } => {
                self.replace_segment(*index as usize, None, anchor, prepared)
            }
            UpdateOutcome::Hydrated { index } | UpdateOutcome::Pending { index } => {
                self.replace_segment(*index as usize, None, anchor, None)
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
        Ok((outcome, anchor))
    }

    fn finish(&mut self, anchor: Option<Anchor>) {
        let view = &mut self.view;
        // Line numbers may need another digit.
        view.metrics = None;
        view.rerun_search();
        view.rebuild_rows(anchor);
    }

    fn slot_parts(&self, id: FileId) -> Option<SlotParts> {
        Some(match self.session.slot(id)? {
            Slot::Live(snapshot) => SlotParts {
                doc: snapshot.diff.clone(),
                revision: snapshot.revision,
                pending: None,
            },
            Slot::Pending(placeholder) => SlotParts {
                doc: placeholder.header().clone(),
                revision: placeholder.revision,
                pending: Some((placeholder.state, placeholder.estimated_lines)),
            },
        })
    }

    /// Adds a segment for a new file at `index`, with a fresh slot.
    fn insert_segment(&mut self, index: usize, prepared: Option<Arc<PreparedSegment>>) {
        let id = self.session.order()[index];
        let Some(parts) = self.slot_parts(id) else {
            return;
        };
        let view = &mut self.view;
        let slot = view.files.slot_ids.len() as u32;
        view.files.slot_ids.push(id);
        view.files.id_slots.insert(id, slot);
        view.files.slot_seg.push(NONE);
        view.generations += 1;
        let mut segment = segment_for(view, &parts.doc, slot, prepared);
        segment.revision = parts.revision.0;
        segment.pending = parts.pending;
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
        prepared: Option<Arc<PreparedSegment>>,
    ) -> Option<Anchor> {
        let slot = self.view.segments[index].slot;
        let id = self.view.files.slot_ids[slot as usize];
        let Some(parts) = self.slot_parts(id) else {
            return anchor;
        };
        let view = &mut self.view;
        let old_doc = view.segments[index].doc.clone();
        let same_revision = view.segments[index].revision == parts.revision.0
            && view.segments[index].pending.is_none()
            && parts.pending.is_none();
        let new_doc = parts.doc.clone();
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
        let mut segment = segment_for(view, &new_doc, slot, prepared);
        segment.revision = parts.revision.0;
        segment.pending = parts.pending;
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
        let to = parts.revision;
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

    #[test]
    fn worker_prepared_segments_install_like_direct_updates() {
        fn sendable<T: Send>(_: &T) {}
        let base = numbered(0..30);
        let edited = base.replace("line 12\n", "twelve\n");
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut direct =
            DiffSessionViewState::new("direct.diff", FocusId::new(1), DiffSession::new());
        let mut prepared =
            DiffSessionViewState::new("prepared.diff", FocusId::new(2), DiffSession::new());
        for state in [&mut direct, &mut prepared] {
            state.set_viewport(600.0, 4000.0);
        }
        direct
            .apply_update(DiffUpdate::Upsert {
                file: snap(1, &base, &edited),
                remap: None,
            })
            .unwrap();
        let segment = PreparedSegment::prepare(snap(1, &base, &edited), prepared.preparation());
        sendable(&segment);
        let outcome = std::thread::spawn(move || segment).join().unwrap();
        assert!(matches!(
            prepared.install_prepared(Arc::new(outcome), None),
            Ok(UpdateOutcome::Inserted { index: 0 })
        ));
        assert_eq!(
            shown(&mut prepared, &mut text),
            shown(&mut direct, &mut text)
        );

        let stale = PreparedSegment::prepare(snap(1, &base, &base), prepared.preparation());
        assert!(matches!(
            prepared.install_prepared(Arc::new(stale), None),
            Err(UpdateError::Stale { .. })
        ));
        let mut other = prepared.preparation();
        other.mode = match other.mode {
            Mode::Unified => Mode::Split,
            Mode::Split => Mode::Unified,
        };
        let mismatched = PreparedSegment::prepare(snap(2, &base, &edited), other);
        prepared
            .install_prepared(Arc::new(mismatched), None)
            .unwrap();
        assert_eq!(
            shown(&mut prepared, &mut text),
            shown(&mut direct, &mut text)
        );
    }

    #[test]
    fn prepared_segments_switch_layouts_without_reprojecting() {
        let base = numbered(0..30);
        let edited = base
            .replace("line 3\n", "three\nextra\n")
            .replace("line 20\n", "");
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut direct =
            DiffSessionViewState::new("direct.diff", FocusId::new(1), DiffSession::new());
        let mut prepared =
            DiffSessionViewState::new("prepared.diff", FocusId::new(2), DiffSession::new());
        for state in [&mut direct, &mut prepared] {
            state.set_viewport(600.0, 4000.0);
        }
        direct
            .apply_update(DiffUpdate::Upsert {
                file: snap(1, &base, &edited),
                remap: None,
            })
            .unwrap();
        let segment = PreparedSegment::prepare(snap(1, &base, &edited), prepared.preparation());
        prepared.install_prepared(Arc::new(segment), None).unwrap();
        assert!(prepared.view.segments[0].alternate.is_some());
        for mode in [Mode::Split, Mode::Unified, Mode::Split] {
            direct.set_mode(mode);
            prepared.set_mode(mode);
            assert_eq!(
                shown(&mut prepared, &mut text),
                shown(&mut direct, &mut text),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn placeholders_keep_order_and_eviction_keeps_the_reader_in_place() {
        use diffy_diff::{FileMeta, Placeholder, SlotState};
        let base = numbered(0..60);
        let edited = base.replace("line 30\n", "thirty\n");
        let file = |id: u64| {
            let doc = diff_texts(Some("f.txt"), Some("f.txt"), Some(&base), Some(&edited), 3);
            FileDiffSnapshot::new(FileId(id), Revision(1), Arc::new(doc)).unwrap()
        };
        let mut state = DiffSessionViewState::new("p.diff", FocusId::new(3), DiffSession::new());
        state.set_viewport(600.0, 4000.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let meta = FileMeta {
            old_path: Some("g.txt".into()),
            new_path: Some("g.txt".into()),
            ..FileMeta::default()
        };
        let results = state.apply_updates([
            (
                DiffUpdate::Upsert {
                    file: file(1),
                    remap: None,
                },
                None,
            ),
            (
                DiffUpdate::Placeholder {
                    file: FileId(2),
                    placeholder: Placeholder::new(Revision(1), SlotState::Unloaded, 40, meta),
                },
                None,
            ),
            (
                DiffUpdate::Upsert {
                    file: file(3),
                    remap: None,
                },
                None,
            ),
        ]);
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(state.slot_state(FileId(2)), Some((SlotState::Unloaded, 40)));
        assert_eq!(state.view.segments.len(), 3);
        let before = shown(&mut state, &mut text);
        let estimate = file(1).estimated_lines();
        state
            .apply_update(DiffUpdate::Evict {
                file: FileId(1),
                revision: Revision(1),
                estimated_lines: estimate,
            })
            .unwrap();
        assert_eq!(
            state.slot_state(FileId(1)),
            Some((SlotState::Evicted, estimate))
        );
        assert_eq!(state.session().order(), [FileId(1), FileId(2), FileId(3)]);
        let evicted = shown(&mut state, &mut text);
        assert_eq!(evicted, before[before.len() / 2..]);
        state
            .apply_update(DiffUpdate::Upsert {
                file: file(1),
                remap: None,
            })
            .unwrap();
        assert_eq!(state.slot_state(FileId(1)), None);
        assert_eq!(shown(&mut state, &mut text), before);
    }

    #[test]
    fn solo_mode_shows_only_the_chosen_file() {
        let base = numbered(0..10);
        let edited = base.replace("line 5\n", "five\n");
        let file = |id: u64| {
            let doc = diff_texts(Some("f.txt"), Some("f.txt"), Some(&base), Some(&edited), 1);
            FileDiffSnapshot::new(FileId(id), Revision(1), Arc::new(doc)).unwrap()
        };
        let mut state = DiffSessionViewState::new("s.diff", FocusId::new(4), DiffSession::new());
        state.set_viewport(600.0, 4000.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        state.apply_updates([1, 2].map(|id| {
            (
                DiffUpdate::Upsert {
                    file: file(id),
                    remap: None,
                },
                None,
            )
        }));
        let both = shown(&mut state, &mut text);
        assert!(state.set_solo(Some(FileId(2))));
        assert!(!state.set_solo(Some(FileId(2))));
        assert!(!state.set_solo(Some(FileId(9))));
        assert_eq!(state.solo(), Some(FileId(2)));
        let one = shown(&mut state, &mut text);
        assert_eq!(one.len() * 2, both.len());
        assert!(state.set_solo(None));
        assert_eq!(shown(&mut state, &mut text), both);
    }

    fn rss_mib() -> f64 {
        let pid = std::process::id().to_string();
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|text| text.trim().parse::<f64>().ok())
            .map_or(0.0, |kib| kib / 1024.0)
    }

    #[test]
    #[ignore = "measurement, prints a report"]
    fn report_prepared_install_costs() {
        use std::time::Instant;

        use diffy_diff::{FileMeta, Placeholder, SlotState};

        let lines = 100_000;
        let old: String = (0..lines)
            .map(|i| format!("let value_{i} = {i};\n"))
            .collect();
        let new: String = (0..lines)
            .map(|i| {
                if i % 97 == 0 {
                    format!("let value_{i} = {} + 1;\n", i)
                } else {
                    format!("let value_{i} = {i};\n")
                }
            })
            .collect();
        let start = Instant::now();
        let doc = diff_texts(Some("big.rs"), Some("big.rs"), Some(&old), Some(&new), 3);
        let snapshot = FileDiffSnapshot::new(FileId(1), Revision(1), Arc::new(doc)).unwrap();
        let diff_ms = start.elapsed().as_secs_f64() * 1e3;
        let rss_doc = rss_mib();

        let mut direct = DiffSessionViewState::new("d.diff", FocusId::new(1), DiffSession::new());
        direct.set_viewport(1200.0, 900.0);
        let start = Instant::now();
        direct
            .apply_update(DiffUpdate::Upsert {
                file: snapshot.clone(),
                remap: None,
            })
            .unwrap();
        let direct_ms = start.elapsed().as_secs_f64() * 1e3;

        let mut prepared = DiffSessionViewState::new("p.diff", FocusId::new(2), DiffSession::new());
        prepared.set_viewport(1200.0, 900.0);
        let preparation = prepared.preparation();
        let start = Instant::now();
        let segment = std::thread::spawn(move || PreparedSegment::prepare(snapshot, preparation))
            .join()
            .unwrap();
        let worker_ms = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        prepared.install_prepared(Arc::new(segment), None).unwrap();
        let install_ms = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        prepared.set_mode(Mode::Split);
        let swap_ms = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        direct.set_mode(Mode::Split);
        let reproject_ms = start.elapsed().as_secs_f64() * 1e3;
        let rss_views = rss_mib();

        let files = 5_000u64;
        let mut sparse = DiffSessionViewState::new("s.diff", FocusId::new(3), DiffSession::new());
        sparse.set_viewport(1200.0, 900.0);
        let start = Instant::now();
        let placeholders: Vec<_> = (0..files)
            .map(|id| {
                let path: std::sync::Arc<str> = format!("dir{}/file{id}.rs", id % 50).into();
                let meta = FileMeta {
                    old_path: Some(path.clone()),
                    new_path: Some(path),
                    ..FileMeta::default()
                };
                Placeholder::new(Revision(1), SlotState::Unloaded, 40, meta)
            })
            .collect();
        let build_ms = start.elapsed().as_secs_f64() * 1e3;
        let mut worst_batch_ms = 0.0f64;
        let start = Instant::now();
        for (chunk, batch) in placeholders.chunks(256).enumerate() {
            let begin = Instant::now();
            let base = chunk as u64 * 256;
            sparse.apply_updates(batch.iter().enumerate().map(|(i, placeholder)| {
                (
                    DiffUpdate::Placeholder {
                        file: FileId(base + i as u64),
                        placeholder: placeholder.clone(),
                    },
                    None,
                )
            }));
            worst_batch_ms = worst_batch_ms.max(begin.elapsed().as_secs_f64() * 1e3);
        }
        let sparse_ms = start.elapsed().as_secs_f64() * 1e3;
        let rss_sparse = rss_mib();

        println!(
            "100k-line file: diff {diff_ms:.1} ms (rss {rss_doc:.0} MiB)\n\
             UI install without preparation: {direct_ms:.1} ms\n\
             worker preparation (both layouts): {worker_ms:.1} ms\n\
             UI install of prepared segment: {install_ms:.1} ms\n\
             layout switch: prepared swap {swap_ms:.2} ms, re-projection {reproject_ms:.1} ms\n\
             rss after both views: {rss_views:.0} MiB\n\
             {files} placeholders: build {build_ms:.1} ms (worker side), install {sparse_ms:.1} ms \
             in 256-file batches, worst batch {worst_batch_ms:.2} ms, rss {rss_sparse:.0} MiB"
        );
    }
}
