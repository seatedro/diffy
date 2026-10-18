use std::sync::Arc;

use diffy_diff::{
    ComparisonOptions, ContextPolicy, DiffDocument, DiffLimits, GapId, InlineOptions, Mode, Reveal,
};

use super::DiffViewState;
use super::navigation::FileId;
use super::presentation::DiffLayout;
use super::state::Segment;
use super::syntax::SyntaxBudget;

impl DiffViewState {
    /// Changed line pairs of `file` the whitespace policy shows as
    /// unchanged, for a "N whitespace-only changes hidden" status.
    pub fn hidden_whitespace_changes(&self, file: FileId) -> u32 {
        self.unit_of(file)
            .and_then(|unit| self.locate(unit))
            .map_or(0, |(seg, file)| {
                self.segments[seg]
                    .comparison
                    .hidden_whitespace_changes(file)
            })
    }

    /// Shows another document; expansion, selection, search position, and
    /// scroll reset. Annotations of the previous document become outdated.
    pub fn set_document(&mut self, doc: DiffDocument) {
        self.generations += 1;
        let revision = self.segments[0].revision + 1;
        let mut segment = Segment::new(
            Arc::new(doc),
            0,
            self.generations,
            self.mode,
            self.context_policy,
            self.comparison,
        );
        segment.revision = revision;
        self.segments[0] = segment;
        self.selection = None;
        self.focused = None;
        self.focused_index = None;
        self.painted.clear();
        self.content_w = [0.0; 2];
        self.syntax
            .reset(self.segments[0].doc.file_count() as usize);
        self.annotations.outdate_all();
        self.rerun_search();
        self.rebuild_rows(None);
        self.syntax
            .request(&self.segments[0].doc, self.segments[0].generation);
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.presentation.layout = match mode {
            Mode::Unified => DiffLayout::Unified,
            Mode::Split => DiffLayout::Split,
        };
        self.switch_mode(mode);
    }

    /// Changes the comparison layout, keeping the top source line in place.
    pub(super) fn switch_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        let anchor = self.anchor();
        self.mode = mode;
        for segment in &mut self.segments {
            segment.rebuild(mode);
        }
        self.rebuild_rows(anchor);
    }

    /// How far each reveal step goes and how few hidden lines a gap may
    /// keep; applies to every file, keeping what is revealed.
    pub fn set_context_policy(&mut self, policy: ContextPolicy) {
        let anchor = self.anchor();
        let mode = self.mode;
        for segment in &mut self.segments {
            segment.expansion.set_policy(policy);
            segment.rebuild(mode);
        }
        self.context_policy = policy;
        self.rebuild_rows(anchor);
    }

    /// Collapses a gap's revealed lines again. Returns whether any were
    /// revealed.
    pub fn collapse(&mut self, gap: GapId) -> bool {
        let Some((seg, file)) = self.locate(gap.file) else {
            return false;
        };
        if !self.segments[seg].expansion.collapse(GapId { file, ..gap }) {
            return false;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        self.segments[seg].rebuild(mode);
        self.rebuild_rows(anchor);
        true
    }

    /// Reveals every unchanged line of every file.
    pub fn expand_all(&mut self) -> bool {
        let anchor = self.anchor();
        let mode = self.mode;
        let mut changed = false;
        for segment in &mut self.segments {
            if segment.expansion.reveal_all(&segment.doc) {
                segment.rebuild(mode);
                changed = true;
            }
        }
        if changed {
            self.rebuild_rows(anchor);
        }
        changed
    }

    /// How removed and added lines are paired and which whitespace changes
    /// are hidden; see [`ComparisonOptions`]. Every file is compared again
    /// and the top source line stays in place. Copy, search, and patch
    /// export keep reading the exact sources.
    pub fn set_comparison(&mut self, options: ComparisonOptions) {
        if options == self.comparison {
            return;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        for segment in &mut self.segments {
            segment.compare(mode, options);
        }
        self.comparison = options;
        // Rows may pair with other lines now.
        self.painted.clear();
        self.rebuild_rows(anchor);
    }

    /// Which changed units inside a line pair are highlighted, and the
    /// limits past which none are. The shorter of
    /// [`InlineOptions::max_line_bytes`] and
    /// [`DiffLimits::inline_line_bytes`] applies.
    pub fn set_inline_options(&mut self, options: InlineOptions) {
        if options == self.inline_options {
            return;
        }
        self.inline_options = options;
        for segment in &mut self.segments {
            segment.clear_inline();
        }
        self.painted.clear();
        self.revision += 1;
    }

    pub fn set_limits(&mut self, limits: DiffLimits) {
        if limits != self.limits {
            self.limits = limits;
            let budget = SyntaxBudget {
                side_bytes: limits.syntax_file_bytes,
            };
            self.syntax.set_budget(budget);
            for bridge in &mut self.slot_syntax {
                bridge.set_budget(budget);
            }
            // Word diffs were computed under the old line limit.
            for segment in &mut self.segments {
                segment.clear_inline();
            }
            self.painted.clear();
            self.content_w = [0.0; 2];
            self.rerun_search();
            self.revision += 1;
        }
    }

    /// Reveals lines of a collapsed gap; see [`diffy_diff::Expansion::reveal`].
    /// The gap's `file` is the view's file index (the document's, for the
    /// static view).
    pub fn reveal(&mut self, gap: GapId, reveal: Reveal, amount: u32) -> bool {
        let Some((seg, file)) = self.locate(gap.file) else {
            return false;
        };
        let local = GapId { file, ..gap };
        let segment = &mut self.segments[seg];
        if !segment
            .expansion
            .reveal(&segment.doc, local, reveal, amount)
        {
            return false;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        self.segments[seg].rebuild(mode);
        self.rebuild_rows(anchor);
        true
    }
}
