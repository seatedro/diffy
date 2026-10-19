use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{
    ChangeIntensity, DiffDocument, FileStatus, InlineDetail, InlineMode, InlineOptions, Mode,
    RowKind, Side, line_detail, merge_pair,
};
use diffy_syntax::HighlightKind;
use quark_render::FontKind;
use quark_text::{LayoutCache, TextParams, TextSpan, TextStyle, TextSystem};
use quark_ui::virtual_list::VariableList;

use super::prepared::{
    Columns, FileFact, FrameRow, LineDetail, LineMerge, LinePaint, LineWindow, Metrics,
    PreparedKind, RowPaint, ViewFrame, WordDetail,
};
use super::prepared_update::file_facts;
use super::state::{RowRef, Segment, source_line};
use super::{DiffViewState, PrepareKey};

/// How a metadata row reads.
pub(crate) fn fact_title(fact: &FileFact) -> String {
    match fact {
        FileFact::Binary => "Binary file not shown".to_owned(),
        FileFact::ModeChange { old, new } => format!("File mode changed from {old} to {new}"),
        FileFact::RenameOnly => "Renamed without changes".to_owned(),
        FileFact::CopyOnly => "Copied without changes".to_owned(),
        FileFact::NoNewlineAtEof(Side::Old) => "No newline at end of the old file".to_owned(),
        FileFact::NoNewlineAtEof(Side::New) => "No newline at end of the new file".to_owned(),
    }
}

/// With wrap on, lines longer than this wrap only a prefix of this many
/// bytes (reported as [`LineDetail::Prefix`]): wrapping lays a line out in
/// full, and a line of megabytes would wrap into a row of millions of
/// points.
pub(crate) const WRAP_LINE_BYTES: usize = 64 << 10;

/// Bytes of `text` a line limited to `limit` bytes shows; see
/// [`diffy_diff::line_detail`].
pub(crate) fn shown_len(text: &str, limit: usize) -> usize {
    match line_detail(text, limit) {
        LineDetail::Complete => text.len(),
        LineDetail::Prefix { shown_bytes, .. } => shown_bytes as usize,
    }
}

/// Largest char boundary at or below `byte`, clamped to the text.
pub(crate) fn floor_boundary(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn plain_span(range: Range<usize>) -> TextSpan {
    TextSpan {
        range,
        weight: None,
        style: None,
        kind: None,
        size: None,
        letter_spacing: None,
        keep_together: false,
    }
}

/// `@@ -a,b +c,d @@ section` of a hunk.
fn hunk_title(doc: &DiffDocument, hunk: u32) -> String {
    let (h, i) = (doc.hunks(), hunk as usize);
    let section = &h.section[i];
    format!(
        "@@ -{},{} +{},{} @@{}{section}",
        h.old_start[i],
        h.old_len[i],
        h.new_start[i],
        h.new_len[i],
        if section.is_empty() { "" } else { " " }
    )
}

/// Sizes for a font of `font_size` points whose digits are `char_w` wide.
fn metrics_for(font_size: f32, line_height: f32, char_w: f32, segments: &[Segment]) -> Metrics {
    let lines = segments
        .iter()
        .flat_map(|s| {
            let doc = &s.doc;
            (0..doc.file_count())
                .flat_map(move |f| [doc.text(f, Side::Old), doc.text(f, Side::New)])
        })
        .map(|t| t.line_count())
        .max()
        .unwrap_or(0)
        .max(1);
    // Partial stores hold fewer lines than the files; leave room for a
    // few more digits than they need.
    let digits = (lines.ilog10() + 1).max(3) as f32;
    Metrics {
        font_size,
        line_h: (font_size * line_height).round(),
        char_w,
        number_w: (digits * char_w + char_w * 2.0).ceil(),
        sign_w: (char_w * 2.0).ceil(),
        text_pad: (char_w * 0.5).ceil(),
    }
}

/// The scroll handle a side's text column uses: unified has one column.
pub(crate) fn column_slot(mode: Mode, side: Side) -> usize {
    match mode {
        Mode::Unified => Side::New as usize,
        Mode::Split => side as usize,
    }
}

impl DiffViewState {
    /// Advances a drag's autoscroll to `now_ms`, shapes the rows entering
    /// the window, and builds the frame the view paints. Does nothing (and
    /// allocates nothing) when the window, the state, and the scale are
    /// what the last call saw. `scale` must be the frame's scale factor.
    pub fn prepare(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        now_ms: u64,
    ) {
        if self.poll_syntax() {
            self.revision += 1;
        }
        if self.long_lines.poll() {
            self.revision += 1;
        }
        let scrolled = self.autoscroll(now_ms);
        self.build_frame(text, layouts, scale);
        // Rows moved under the held pointer: its selection end follows.
        if scrolled && self.follow_pointer() {
            self.build_frame(text, layouts, scale);
        }
    }

    pub(crate) fn overscan(&self) -> f32 {
        self.metrics().line_h * 8.0
    }

    pub(crate) fn metrics(&self) -> Metrics {
        self.metrics
            .map_or_else(|| self.estimated_metrics(), |(m, _)| m)
    }

    /// Metrics before the font has been measured.
    fn estimated_metrics(&self) -> Metrics {
        let font_size = self.style.font_size;
        metrics_for(
            font_size,
            self.style.line_height,
            font_size * 0.6,
            &self.segments,
        )
    }

    /// Follows an automatic layout across the viewport width (see
    /// [`super::presentation::auto_mode`]), keeping the top source line in
    /// place. Explicit layouts stay put.
    fn resolve_layout(&mut self) {
        let m = self.metrics();
        if let Some(mode) = super::presentation::auto_mode(
            self.presentation.layout,
            self.mode,
            self.viewport.0,
            &m,
            &self.presentation,
        ) {
            self.switch_mode(mode);
        }
    }

    pub(crate) fn build_frame(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
    ) {
        let measured = (scale.to_bits(), text.font_epoch());
        if self.metrics.is_none_or(|(_, key)| key != measured) {
            self.measure_font(text, layouts, scale);
        }
        self.resolve_layout();
        let m = self.metrics();
        let columns = Columns::new(self.mode, self.viewport.0, &m, &self.presentation);
        if self.style.wrap {
            self.measure_window(text, layouts, scale, &columns);
        }
        let window = self.list.window(self.overscan()).range;
        let grid = super::long_lines::GRID_COLUMNS as f64 * f64::from(m.char_w);
        let hgrid = [0, 1].map(|slot| {
            let scroll = self.long_lines.expected_scroll(slot, self.frame_id);
            (scroll.unwrap_or_else(|| self.hscroll[slot].offset_f64().0) / grid) as u32
        });
        let key = PrepareKey {
            window: (window.start, window.end),
            scroll: self.list.scroll_offset().to_bits(),
            revision: self.revision,
            scale: scale.to_bits(),
            hgrid,
        };
        if self.prepared == Some(key) && self.frame.is_some() {
            return;
        }
        self.prepared = Some(key);
        self.frame_id += 1;
        self.prioritize_syntax(window.clone());
        let scroll = self.list.scroll_offset();
        let ordered = self.ordered_selection();
        let mut kept = HashMap::with_capacity(window.len());
        let mut rows = Vec::with_capacity(window.len());
        let mut next_top: Option<f32> = None;
        for index in window {
            let r = self.refs.at(index);
            let key = self.ref_key(r);
            let stamp = self.stamp(r, &columns, scale);
            let paint = match self.painted.remove(&key) {
                Some(p) if p.stamp == stamp => p,
                _ => Rc::new(self.build_row(r, stamp, text, layouts, scale, &columns)),
            };
            for (side, line) in [Side::Old, Side::New].into_iter().zip(&paint.sides) {
                let Some(line) = line else {
                    continue;
                };
                let slot = column_slot(self.mode, side);
                let width = line
                    .window
                    .map_or(f64::from(line.layout.size().0), |w| w.width);
                let pad = f64::from(m.text_pad * 2.0 + m.char_w);
                self.content_w[slot] = self.content_w[slot].max((width + pad).ceil());
            }
            let (selected, search) = match r {
                RowRef::Line { seg, row } => {
                    let selected = [Side::Old, Side::New].map(|side| {
                        let line = paint.sides[side as usize].as_ref()?;
                        if let Some(merge) = &line.merge {
                            let (lo, hi) =
                                self.row_selection(ordered, (seg, row), side, merge.source_len())?;
                            let lo = merge.to_layout(Side::New, lo)?;
                            let hi = merge.to_layout(Side::New, hi)?;
                            return (lo < hi).then_some((lo, hi));
                        }
                        // In line bytes, then in the bytes of a window's text.
                        let start = line.window.map_or(0, |w| w.start);
                        let len = line.layout.text().len();
                        let (lo, mut hi) =
                            self.row_selection(ordered, (seg, row), side, start + len)?;
                        if line.window.is_some() {
                            hi = hi.min(start + len);
                        }
                        let (lo, hi) = (lo.max(start) - start, hi.saturating_sub(start));
                        (lo < hi).then_some((lo, hi))
                    });
                    (selected, self.search_marks(seg as usize, row, &paint))
                }
                _ => Default::default(),
            };
            let rows_table = self.list.rows();
            let height = rows_table.height_at(index);
            // Rows stack from the first row's top, taken relative to the
            // scroll in f64 before it becomes a viewport position.
            let top = *next_top
                .get_or_insert_with(|| (rows_table.offset_of_index(index) - scroll) as f32);
            next_top = Some(top + height);
            rows.push(FrameRow {
                key,
                index: index as u32,
                top,
                height,
                paint: paint.clone(),
                selected,
                search,
                focused: self.focused == Some(key),
            });
            kept.insert(key, paint);
        }
        let sticky_header = self.sticky_header(scroll, &columns, scale, text, layouts, &mut kept);
        self.painted = kept;
        self.frame = Some(Rc::new(ViewFrame {
            id: self.id,
            label: self.label,
            focus: self.focus,
            viewport: self.viewport,
            metrics: m,
            columns,
            presentation: self.presentation,
            appearance: self.appearance,
            wrap: self.style.wrap,
            rows,
            sticky_header,
            content_w: self.content_w,
            // A grid step before the scroll's: windows reach that far left,
            // so content between the origin and the scroll is all there.
            content_origin: hgrid.map(|step| f64::from(step.saturating_sub(1)) * grid),
            scroll,
            total: self.list.rows().total_extent(),
            row_count: self.refs.len() as u32,
            hscroll: self.hscroll.clone(),
            scrollbar_auto_hide: self.scrollbar_auto_hide,
            scrollbar: self.scrollbar.clone(),
        }));
    }

    /// The header to pin at the top: with sticky headers on, the file
    /// header of the top row's file when that header is above the
    /// viewport. Its paint is kept with the window's.
    #[allow(clippy::too_many_arguments)]
    fn sticky_header(
        &mut self,
        scroll: f64,
        columns: &Columns,
        scale: f32,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        kept: &mut HashMap<u64, Rc<RowPaint>>,
    ) -> Option<FrameRow> {
        if !self.presentation.sticky_headers {
            return None;
        }
        let top = self.list.rows().row_at(scroll)?;
        let (seg, file) = match self.refs.at(top) {
            RowRef::Line { seg, row } => (
                seg,
                self.segments[seg as usize].projection.file[row as usize],
            ),
            RowRef::Fact { seg, file, .. } => (seg, file),
            _ => return None,
        };
        let segment = &self.segments[seg as usize];
        let header = *segment.projection.file_rows.get(file as usize)?;
        let index = segment.list_rows.index(header)?;
        if index as usize > top {
            return None;
        }
        let r = RowRef::Line { seg, row: header };
        let key = self.ref_key(r);
        let stamp = self.stamp(r, columns, scale);
        let paint = match kept.get(&key).or_else(|| self.painted.get(&key)) {
            Some(p) if p.stamp == stamp => p.clone(),
            _ => Rc::new(self.build_row(r, stamp, text, layouts, scale, columns)),
        };
        kept.insert(key, paint.clone());
        let height = self.list.rows().height_at(index as usize);
        Some(FrameRow {
            key,
            index,
            top: 0.0,
            height,
            paint,
            selected: [None, None],
            search: Default::default(),
            focused: self.focused == Some(key),
        })
    }

    /// Tells the syntax workers which files are on screen, so theirs are
    /// highlighted first. Does nothing for bridges whose files did not
    /// move in or out of view.
    fn prioritize_syntax(&mut self, window: Range<usize>) {
        let refs = || self.refs.range(window.clone());
        // A long side still streaming colors the surroundings of its top
        // line on screen first (headers and the other side's rows have
        // none).
        for side in [Side::Old, Side::New] {
            let top = refs().find_map(|r| match r {
                RowRef::Line { seg, row } => {
                    let p = &self.segments[seg as usize].projection;
                    Some((seg as usize, row, p.line(row, side)?))
                }
                _ => None,
            });
            if let Some((seg, row, index)) = top {
                let segment = &self.segments[seg];
                let file = segment.projection.file[row as usize];
                let start = segment.doc.text(file, side).line_range(index);
                if let (Some((bridge, file)), Some(start)) = (self.syntax_of(seg, file), start) {
                    bridge.set_focus(file, side, start.start);
                }
            }
        }
        let seg_file = |r: RowRef| match r {
            RowRef::Line { seg, row } => Some((
                seg,
                self.segments[seg as usize].projection.file[row as usize],
            )),
            RowRef::Fact { seg, file, .. } => Some((seg, file)),
            _ => None,
        };
        let (Some(first), Some(last)) =
            (refs().find_map(seg_file), refs().rev().find_map(seg_file))
        else {
            return;
        };
        if self.files.is_static() {
            self.syntax.set_visible_files(first.1..last.1 + 1);
            return;
        }
        for (seg, segment) in self.segments.iter().enumerate() {
            if let Some(bridge) = self.slot_syntax.get_mut(segment.slot as usize) {
                let shown = (first.0..=last.0).contains(&(seg as u32));
                bridge.set_visible_files(0..u32::from(shown));
                if shown && bridge.is_enabled() && bridge.requested() != segment.generation {
                    bridge.request(&segment.doc, segment.generation);
                }
            }
        }
    }

    /// Heights of rows entering the window, from their wrapped layouts.
    fn measure_window(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) {
        let m = self.metrics();
        let overscan = self.overscan();
        let mut list = std::mem::replace(&mut self.list, VariableList::new(1.0, 0.0));
        let mut painted = std::mem::take(&mut self.painted);
        let this = &*self;
        // Keys of the indexed row table are list indices.
        list.measure_visible(this.viewport.0, overscan, |index, _| {
            let Some(r) = this.refs.get(index as usize) else {
                return m.line_h;
            };
            let key = this.ref_key(r);
            if let RowRef::Annotation { index } = r {
                return this.annotation_height(index, &m);
            }
            let stamp = this.stamp(r, columns, scale);
            match painted.get(&key) {
                Some(p) if p.stamp == stamp => p.height(&m, &this.presentation),
                _ => {
                    let p = this.build_row(r, stamp, text, layouts, scale, columns);
                    let height = p.height(&m, &this.presentation);
                    painted.insert(key, Rc::new(p));
                    height
                }
            }
        });
        self.list = list;
        self.painted = painted;
    }

    /// Identifies everything a row's paint depends on besides its key.
    fn stamp(&self, r: RowRef, columns: &Columns, scale: f32) -> u64 {
        let mut h = std::hash::DefaultHasher::new();
        (
            scale.to_bits(),
            self.style.font_size.to_bits(),
            self.style.line_height.to_bits(),
            self.mode,
            self.limits,
        )
            .hash(&mut h);
        match r {
            RowRef::Line { seg, row } => {
                let segment = &self.segments[seg as usize];
                let p = &segment.projection;
                let file = p.file[row as usize];
                (segment.generation, segment.slot).hash(&mut h);
                if let Some((syntax, file)) = self.syntax_of(seg as usize, file) {
                    syntax.file_generation(file).hash(&mut h);
                }
                if self.style.wrap {
                    let m = self.metrics();
                    for side in [Side::Old, Side::New] {
                        super::presentation::wrap_width(columns, side, &m, &self.presentation)
                            .to_bits()
                            .hash(&mut h);
                    }
                } else if p.kind[row as usize].is_line() {
                    for side in [Side::Old, Side::New] {
                        let Some(index) = p.line(row, side) else {
                            continue;
                        };
                        let store = segment.doc.text(file, side);
                        let range = store.display_range(index).unwrap_or(0..0);
                        if range.len() > super::long_lines::WINDOW_LINE_BYTES {
                            let window = self.line_window(store, range, side, columns);
                            window.map(|w| w.start).hash(&mut h);
                        }
                    }
                }
                if matches!(
                    p.kind[row as usize],
                    RowKind::Modified | RowKind::Removed | RowKind::Added
                ) {
                    self.long_lines.generation().hash(&mut h);
                }
                if p.kind[row as usize] == RowKind::FileHeader {
                    let unit = segment.unit(file);
                    (self.collapsed.contains(&unit), self.file_annotations(unit)).hash(&mut h);
                }
                if let Some(gap) = p.gap(row) {
                    // Its count changes as lines are revealed.
                    gap.hidden.hash(&mut h);
                    self.gap_annotations(segment, gap).hash(&mut h);
                }
            }
            RowRef::Fact { seg, .. } => self.segments[seg as usize].generation.hash(&mut h),
            RowRef::Annotation { index } => self.annotations.entry(index).hash(&mut h),
            RowRef::More { hidden } => hidden.hash(&mut h),
            RowRef::Pending { seg } => self.segments[seg as usize].generation.hash(&mut h),
        }
        h.finish()
    }

    fn measure_font(&mut self, text: &mut TextSystem, layouts: &mut LayoutCache, scale: f32) {
        let style = self.text_style();
        let params = TextParams::new("0000000000", style).scale_factor(scale);
        let char_w = layouts
            .layout(text, &params)
            .map_or(self.style.font_size * 0.6, |l| l.size().0 / 10.0);
        let m = metrics_for(
            self.style.font_size,
            self.style.line_height,
            char_w,
            &self.segments,
        );
        let fonts = text.font_epoch();
        // The metrics the rows were built with: measured, or estimated
        // before the first frame.
        let old = self.metrics();
        let new_fonts = self.metrics.is_some_and(|(_, (_, old))| old != fonts);
        self.metrics = Some((m, (scale.to_bits(), fonts)));
        if new_fonts {
            // Rows were shaped and wrapped with the old fonts.
            self.painted.clear();
            self.content_w = [0.0; 2];
        }
        // Row heights follow the line height; wrapped rows also follow the
        // text width, which the digit width moves. Anything else (the
        // first frame measuring the font, mostly) keeps the rows: a
        // rebuild costs a pass over every row.
        if old.line_h != m.line_h {
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        } else if old != m || new_fonts {
            if self.style.wrap {
                self.list.invalidate_all();
            }
            self.revision += 1;
        }
    }

    fn text_style(&self) -> TextStyle {
        TextStyle::new(self.style.font_size)
            .kind(FontKind::Mono)
            .line_height((self.style.font_size * self.style.line_height).round())
    }

    fn build_row(
        &self,
        r: RowRef,
        stamp: u64,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) -> RowPaint {
        let blank = |kind: PreparedKind, file: u32| RowPaint {
            stamp,
            kind,
            file,
            sides: [None, None],
            source_lines: [None, None],
            title: Arc::from(""),
            gap: None,
            hidden: 0,
            status: FileStatus::Modified,
            stats: (0, 0),
            binary: false,
            syntax: None,
            collapsed: false,
            annotations: 0,
        };
        match r {
            RowRef::Line { seg, row } => {
                self.build_line_row(seg as usize, row, stamp, text, layouts, scale, columns)
            }
            RowRef::Fact { seg, file, fact } => {
                let segment = &self.segments[seg as usize];
                let facts = file_facts(&segment.doc, file);
                let fact = facts[fact as usize].clone();
                RowPaint {
                    title: fact_title(&fact).into(),
                    ..blank(PreparedKind::Fact(fact), segment.unit(file))
                }
            }
            RowRef::Annotation { index } => {
                let (slot, unit, title) = self.annotation_slot(index);
                RowPaint {
                    title,
                    ..blank(PreparedKind::Annotation(slot), unit)
                }
            }
            RowRef::More { hidden } => {
                let rows = if hidden == 1 { "row" } else { "rows" };
                RowPaint {
                    title: format!("{hidden} more {rows}").into(),
                    ..blank(
                        PreparedKind::More {
                            hidden_rows: hidden,
                        },
                        0,
                    )
                }
            }
            RowRef::Pending { seg } => {
                let segment = &self.segments[seg as usize];
                let header = segment.projection.file_rows.first().copied().unwrap_or(0);
                let (state, estimated_lines) = segment
                    .pending
                    .unwrap_or((diffy_diff::SlotState::Unloaded, 1));
                RowPaint {
                    kind: PreparedKind::Pending {
                        state,
                        estimated_lines,
                    },
                    ..self.build_line_row(
                        seg as usize,
                        header,
                        stamp,
                        text,
                        layouts,
                        scale,
                        columns,
                    )
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_line_row(
        &self,
        seg: usize,
        row: u32,
        stamp: u64,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) -> RowPaint {
        let segment = &self.segments[seg];
        let (doc, p) = (&segment.doc, &segment.projection);
        let r = row as usize;
        let (kind, file) = (p.kind[r], p.file[r]);
        let meta = &doc.files().meta[file as usize];
        let mut paint = RowPaint {
            stamp,
            kind: PreparedKind::Diff(kind),
            file: segment.unit(file),
            sides: [None, None],
            source_lines: [None, None],
            title: Arc::from(""),
            gap: None,
            hidden: 0,
            status: meta.status,
            stats: (
                doc.files().additions[file as usize],
                doc.files().deletions[file as usize],
            ),
            binary: meta.binary,
            syntax: None,
            collapsed: false,
            annotations: 0,
        };
        match kind {
            RowKind::FileHeader => {
                let unit = segment.unit(file);
                paint.syntax = self
                    .syntax_of(seg, file)
                    .and_then(|(syntax, file)| syntax.file_status(file));
                paint.collapsed = self.collapsed.contains(&unit);
                paint.annotations = self.file_annotations(unit);
                paint.title = match (&meta.old_path, &meta.new_path) {
                    (Some(old), Some(new)) if old != new => format!("{old} \u{2192} {new}").into(),
                    _ => doc.path(file).into(),
                };
            }
            RowKind::HunkHeader => paint.title = hunk_title(doc, p.hunk[r]).into(),
            RowKind::Gap => {
                let gap = p.gap(row).expect("gap row");
                paint.hidden = gap.hidden;
                paint.gap = Some(diffy_diff::GapId {
                    file: segment.unit(file),
                    ..gap.id
                });
                let lines = if gap.hidden == 1 { "line" } else { "lines" };
                let mut title = format!("{} unchanged {lines}", gap.hidden);
                let notes = self.gap_annotations(segment, gap);
                if notes > 0 {
                    let s = if notes == 1 { "" } else { "s" };
                    title.push_str(&format!(", {notes} annotation{s}"));
                }
                if let Some(h) = gap.id.hunk {
                    title.push_str("    ");
                    title.push_str(&hunk_title(doc, h));
                }
                paint.title = title.into();
            }
            RowKind::Merged
                if let Some(line) = self.merged_line(seg, row, text, layouts, scale, columns) =>
            {
                for side in [Side::Old, Side::New] {
                    paint.source_lines[side as usize] =
                        p.line(row, side).map(|i| source_line(doc, file, side, i));
                }
                paint.sides[Side::New as usize] = Some(line);
            }
            _ => {
                let m = self.metrics();
                let (words, word_detail) = self.word_ranges(segment, row);
                for side in [Side::Old, Side::New] {
                    let Some(index) = p.line(row, side) else {
                        continue;
                    };
                    paint.source_lines[side as usize] = Some(source_line(doc, file, side, index));
                    if p.mode == Mode::Unified && kind == RowKind::Context && side == Side::Old {
                        // Unified context shows the new side's text.
                        continue;
                    }
                    let store = doc.text(file, side);
                    let line = store.display_line(index).unwrap_or("");
                    // Wrapping lays out a line in full, so a huge one wraps
                    // only a prefix (and says so); without wrap a long line
                    // shapes the window in view.
                    let cap = if self.style.wrap {
                        self.limits.shaped_line_bytes.min(WRAP_LINE_BYTES)
                    } else {
                        self.limits.shaped_line_bytes
                    };
                    let detail = line_detail(line, cap);
                    let shown = shown_len(line, cap);
                    let range = store.display_range(index).unwrap_or(0..0);
                    let window =
                        self.line_window(store, range.start..range.start + shown, side, columns);
                    let (from, to) = window.map_or((0, shown), |w| (w.start, w.end));
                    let line = &line[from..to];
                    let range = range.start + from..range.start + to;
                    let (spans, tones) = match self.syntax_of(seg, file) {
                        Some((syntax, file)) => syntax.spans(file, side, range),
                        None => (Vec::new(), Arc::from([])),
                    };
                    let params = TextParams::new(line, self.text_style())
                        .spans(spans)
                        .scale_factor(scale)
                        .wrap_width(self.style.wrap.then(|| {
                            super::presentation::wrap_width(columns, side, &m, &self.presentation)
                        }));
                    let Ok(layout) = layouts.layout(text, &params) else {
                        continue;
                    };
                    let mut words = super::paint::join_whitespace_gaps(
                        match &window {
                            Some(w) => super::long_lines::clip_to_window(&words[side as usize], w),
                            None => clip_ranges(&words[side as usize], shown),
                        },
                        line,
                    );
                    let whole = super::paint::covers_text(&words, line);
                    if whole {
                        words.clear();
                    }
                    paint.sides[side as usize] = Some(LinePaint {
                        layout,
                        tones,
                        words,
                        whole,
                        detail,
                        word_detail,
                        window,
                        struck: Vec::new(),
                        removed_spans: Arc::from([]),
                        merge: None,
                    });
                }
            }
        }
        paint
    }

    fn merged_line(
        &self,
        seg: usize,
        row: u32,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        columns: &Columns,
    ) -> Option<LinePaint> {
        let segment = &self.segments[seg];
        let (doc, p) = (&segment.doc, &segment.projection);
        let file = p.file[row as usize];
        let index = [p.line(row, Side::Old)?, p.line(row, Side::New)?];
        let lines = [Side::Old, Side::New].map(|side| {
            doc.text(file, side)
                .display_line(index[side as usize])
                .unwrap_or("")
        });
        let spans = [Side::Old, Side::New].map(|side| {
            let line = source_line(doc, file, side, index[side as usize]);
            doc.semantic_spans(file, side, line)
        });
        let pieces = merge_pair(lines[0], lines[1], spans[0], spans[1])?;
        let syntax = self.syntax_of(seg, file);
        let mut composed = String::new();
        let mut text_spans = Vec::new();
        let mut tones = Vec::new();
        let mut removed = Vec::new();
        let mut words = Vec::new();
        let mut struck = Vec::new();
        let mut merge = LineMerge::default();
        let mut previous_old: Option<usize> = None;
        for piece in pieces {
            if let Some(end) = previous_old.take()
                && piece.side == Side::New
                && piece.changed
            {
                let at = composed.len();
                composed.push(' ');
                merge.push(at..composed.len(), Side::Old, end..end);
            }
            let side = piece.side as usize;
            let range = piece.range.start as usize..piece.range.end as usize;
            let start = composed.len();
            composed.push_str(&lines[side][range.clone()]);
            let shown = start..composed.len();
            if piece.side == Side::Old {
                text_spans.push(plain_span(shown.clone()));
                tones.push(HighlightKind::Normal);
                removed.push(true);
            } else if let Some((syntax, file)) = &syntax {
                let base = doc
                    .text(*file, Side::New)
                    .display_range(index[side])
                    .map_or(0, |r| r.start);
                let (piece_spans, piece_tones) =
                    syntax.spans(*file, Side::New, base + range.start..base + range.end);
                for (span, tone) in piece_spans.into_iter().zip(piece_tones.iter()) {
                    text_spans.push(TextSpan {
                        range: span.range.start + start..span.range.end + start,
                        ..span
                    });
                    tones.push(*tone);
                    removed.push(false);
                }
            }
            if piece.side == Side::Old {
                previous_old = Some(range.end);
            }
            match (piece.side, piece.changed) {
                (Side::Old, _) => struck.push(shown.clone()),
                (Side::New, true) => words.push(shown.clone()),
                (Side::New, false) => {}
            }
            merge.push(shown, piece.side, range);
        }
        let m = self.metrics();
        let params = TextParams::new(composed, self.text_style())
            .spans(text_spans)
            .scale_factor(scale)
            .wrap_width(self.style.wrap.then(|| {
                super::presentation::wrap_width(columns, Side::New, &m, &self.presentation)
            }));
        let layout = layouts.layout(text, &params).ok()?;
        Some(LinePaint {
            layout,
            tones: tones.into(),
            words,
            detail: LineDetail::Complete,
            word_detail: WordDetail::Done(InlineDetail::Exact),
            window: None,
            struck,
            removed_spans: removed.into(),
            merge: Some(Arc::new(merge)),
            whole: false,
        })
    }

    /// Changed words of `row`'s lines, by side, and how complete they are:
    /// the pair's inline diff, shared by both unified rows that show it. A
    /// long pair's comes from the word thread; none until it lands.
    fn word_ranges(&self, segment: &Segment, row: u32) -> ([Vec<Range<usize>>; 2], WordDetail) {
        let file = segment.projection.file[row as usize];
        if self.inline_options.mode != InlineMode::Off && segment.doc.semantics(file).is_some() {
            return (
                semantic_words(segment, row, file),
                WordDetail::Done(InlineDetail::Exact),
            );
        }
        let Some(pair) = segment.projection.line_pair(row) else {
            return (Default::default(), WordDetail::Unpaired);
        };
        let options = InlineOptions {
            max_line_bytes: self
                .inline_options
                .max_line_bytes
                .min(self.limits.inline_line_bytes),
            ..self.inline_options
        };
        let line = |side, i| {
            let store = segment.doc.text(pair.file, side);
            (store.shared(), store.display_range(i).unwrap_or(0..0))
        };
        let (a, b) = (line(Side::Old, pair.old), line(Side::New, pair.new));
        let long = a.1.len() + b.1.len() > super::long_lines::SYNC_PAIR_BYTES;
        if long && a.1.len().max(b.1.len()) <= options.max_line_bytes {
            let wake = self.syntax_wake.clone();
            return match self.long_lines.words(a, b, &options, wake) {
                Some(words) => (words.0.clone(), WordDetail::Done(words.1)),
                None => (Default::default(), WordDetail::Pending),
            };
        }
        let d = segment.inline_diff(pair, &options);
        let bytes = |v: &[Range<u32>]| -> Vec<Range<usize>> {
            v.iter().map(|r| r.start as usize..r.end as usize).collect()
        };
        ([bytes(&d.old), bytes(&d.new)], WordDetail::Done(d.detail))
    }

    /// Scrolls the column showing `side` sideways so byte `byte` of a long
    /// line (store index `index` of `file`) is in view, unless it is. Short
    /// lines, and wrapped ones, stay put.
    pub(crate) fn reveal_byte(
        &mut self,
        seg: usize,
        file: u32,
        side: Side,
        index: u32,
        byte: usize,
    ) {
        if self.style.wrap {
            return;
        }
        let store = self.segments[seg].doc.text(file, side);
        let Some(range) = store.display_range(index) else {
            return;
        };
        if range.len() <= super::long_lines::WINDOW_LINE_BYTES {
            return;
        }
        let m = self.metrics();
        let x = self.long_lines.x_of(store.shared(), range, byte, m.char_w);
        let columns = Columns::new(self.mode, self.viewport.0, &m, &self.presentation);
        let view_w = f64::from(columns.of(side).text_w - m.text_pad * 2.0);
        let handle = &self.hscroll[column_slot(self.mode, side)];
        let (at, _) = handle.offset_f64();
        if x < at || x > at + view_w - f64::from(m.char_w * 8.0) {
            let to = (x - view_w / 3.0).max(0.0);
            handle.set_offset(to, 0.0);
            // The handle moves only as the next frame paints; windows built
            // before then must already be the ones it will show.
            self.long_lines
                .expect_scroll(column_slot(self.mode, side), to, self.frame_id);
            self.revision += 1;
        }
    }

    /// The window of a long line (`line`, a range of `store`'s text) to
    /// shape at its column's sideways scroll; `None` with wrap on or for a
    /// line short enough to shape whole.
    fn line_window(
        &self,
        store: &diffy_diff::TextStore,
        line: Range<usize>,
        side: Side,
        columns: &Columns,
    ) -> Option<LineWindow> {
        if self.style.wrap {
            return None;
        }
        let m = self.metrics();
        let slot = column_slot(self.mode, side);
        let scroll = self
            .long_lines
            .expected_scroll(slot, self.frame_id)
            .unwrap_or_else(|| self.hscroll[slot].offset_f64().0);
        let view_w = columns.of(side).text_w;
        self.long_lines
            .window(store.shared(), line, scroll, view_w, m.char_w)
    }
}

/// `ranges` cut to the first `len` bytes.
fn clip_ranges(ranges: &[Range<usize>], len: usize) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter(|r| r.start < len)
        .map(|r| r.start..r.end.min(len))
        .collect()
}

fn semantic_words(segment: &Segment, row: u32, file: u32) -> [Vec<Range<usize>>; 2] {
    let (doc, p) = (&segment.doc, &segment.projection);
    [Side::Old, Side::New].map(|side| {
        let Some(index) = p.line(row, side) else {
            return Vec::new();
        };
        let line = source_line(doc, file, side, index);
        doc.semantic_spans(file, side, line)
            .iter()
            .filter(|span| span.intensity != ChangeIntensity::UnchangedContext)
            .map(|span| span.start as usize..span.end as usize)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use quark_ui::FocusId;

    use super::*;
    use crate::diff_view::{DiffEvent, DiffStyle};

    // Catches rows shaped and wrapped before a font change being kept: a
    // wrapped diff must lay out as a fresh view does in the new fonts.
    #[test]
    fn font_change_reshapes_and_rewraps_every_row() {
        let long = "let wrapped = some_function(first_argument, second_argument, third);";
        let patch = format!(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-{long}\n+{long} // x\n"
        );
        let doc = || diffy_diff::parse_unified(&patch).unwrap();
        let open = || {
            let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc());
            state.set_style(DiffStyle {
                wrap: true,
                ..DiffStyle::default()
            });
            state.set_viewport(260.0, 400.0);
            state
        };
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        // Row heights, and each side's text in pixels wide.
        let layout =
            |state: &mut DiffViewState, text: &mut TextSystem, layouts: &mut LayoutCache| {
                state.prepare(text, layouts, 1.0, 0);
                let frame = state.frame.as_ref().unwrap();
                let rows: Vec<String> = frame
                    .rows
                    .iter()
                    .map(|row| {
                        let widths: Vec<f32> = row
                            .paint
                            .sides
                            .iter()
                            .flatten()
                            .map(|side| side.layout.size().0)
                            .collect();
                        format!("{}:{widths:?}", row.height)
                    })
                    .collect();
                rows.join(" ")
            };
        let mut state = open();
        let before = layout(&mut state, &mut text, &mut layouts);

        text.set_font_settings(&quark_text::FontSettings {
            // Proportional, so the lines wrap elsewhere.
            mono_family: "Inter".into(),
            ..Default::default()
        });
        let after = layout(&mut state, &mut text, &mut layouts);

        let expected = layout(&mut open(), &mut text, &mut layouts);
        assert_ne!(before, expected, "the fonts lay the rows out differently");
        assert_eq!(after, expected);
    }

    // Catches the rows being rebuilt whenever the font metrics are
    // measured again, though row heights did not change (here a font the
    // diff does not use was loaded; the first frame of every view measures
    // too). The rebuild dropped the measured heights of wrapped rows off
    // screen, so the content height fell back to estimates, and it costs a
    // pass over every row of a huge diff.
    #[test]
    fn remeasuring_fonts_keeps_wrapped_rows_measured_off_screen() {
        let long = "word ".repeat(40);
        let new: String = (0..40).map(|i| format!("{long}{i}\n")).collect();
        let doc = diffy_diff::diff_texts(Some("a.txt"), Some("a.txt"), Some(""), Some(&new), 3);
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc);
        state.set_style(DiffStyle {
            wrap: true,
            ..DiffStyle::default()
        });
        state.set_viewport(300.0, 200.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        state.prepare(&mut text, &mut layouts, 1.0, 0);
        state.handle(DiffEvent::ScrollTo(f32::MAX));
        state.prepare(&mut text, &mut layouts, 1.0, 0);
        let before = state.content_height();

        let font: &'static [u8] = include_bytes!("../../../../assets/fonts/Geist-Bold.otf");
        text.load_font_data(Arc::new(font));
        state.prepare(&mut text, &mut layouts, 1.0, 0);

        assert_eq!(state.content_height(), before);
    }

    /// Each prepared line row as `old | new`, changed words in brackets
    /// and wholly changed lines marked.
    fn split_rows(state: &DiffViewState) -> Vec<String> {
        let frame = state.frame().unwrap();
        let side = |row: &FrameRow, side: Side| {
            let Some(line) = &row.paint.sides[side as usize] else {
                return String::new();
            };
            let text = line.layout.text();
            let mut out = String::new();
            let mut at = 0;
            for r in &line.words {
                out.push_str(&text[at..r.start]);
                out.push_str(&format!("[{}]", &text[r.clone()]));
                at = r.end;
            }
            out.push_str(&text[at..]);
            if line.whole {
                out.push_str(" (whole)");
            }
            out
        };
        frame
            .rows
            .iter()
            .filter(|row| row.paint.kind.is_line())
            .map(|row| format!("{} | {}", side(row, Side::Old), side(row, Side::New)))
            .collect()
    }

    // Catches the view pairing changed lines by position: a log line
    // inserted above an edited return must not sit beside the old return,
    // and the return's highlight must be its own edit.
    #[test]
    fn an_edited_line_pairs_with_its_similar_replacement() {
        let old = "fn f() {\n    return a + b;\n}\n";
        let new = "fn f() {\n    log(\"x\");\n    return a + b * 2;\n}\n";
        let doc = diffy_diff::diff_texts(Some("f.rs"), Some("f.rs"), Some(old), Some(new), 3);
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), doc);
        state.set_mode(Mode::Split);
        state.set_viewport(800.0, 400.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        state.prepare(&mut text, &mut layouts, 1.0, 0);

        assert_eq!(
            split_rows(&state),
            [
                "fn f() { | fn f() {",
                " |     log(\"x\");",
                "    return a + b; |     return a + b[ * 2];",
                "} | }",
            ]
        );
    }

    #[test]
    fn semantic_spans_replace_word_highlights() {
        use diffy_diff::{
            AlignedRow, DocumentBuilder, FileMeta, SemanticSpan, SourceCoverage, TextStore,
        };
        let span = |start, end, intensity| SemanticSpan {
            start,
            end,
            intensity,
        };
        let row = |old, new| AlignedRow { old, new };
        let mut builder = DocumentBuilder::new();
        let mut file = builder.file(
            FileMeta {
                old_path: Some("f.rs".into()),
                new_path: Some("f.rs".into()),
                status: FileStatus::Modified,
                ..FileMeta::default()
            },
            SourceCoverage::Full,
            TextStore::new("fn f() {\n    foo(1)\n}\n"),
            TextStore::new("fn f() {\n    foo(2)\n    bar()\n}\n"),
        );
        file.structural(true)
            .hunk(0, 0)
            .context(1)
            .change(1, 2)
            .context(1)
            .row(row(Some(0), Some(0)))
            .row(row(Some(1), Some(1)))
            .row(row(None, Some(2)))
            .row(row(Some(2), Some(3)))
            .spans(
                Side::Old,
                1,
                &[
                    span(4, 8, ChangeIntensity::UnchangedContext),
                    span(8, 9, ChangeIntensity::NovelWord),
                ],
            )
            .spans(Side::New, 1, &[span(8, 9, ChangeIntensity::Novel)])
            .spans(Side::New, 2, &[span(4, 9, ChangeIntensity::Novel)]);
        file.finish().unwrap();
        let mut state = DiffViewState::new("test.diff", FocusId::new(1), builder.build());
        state.set_mode(Mode::Split);
        state.set_viewport(800.0, 400.0);
        let mut text = TextSystem::vendored_only(&Default::default());
        let mut layouts = LayoutCache::default();
        state.prepare(&mut text, &mut layouts, 1.0, 0);

        assert_eq!(
            split_rows(&state),
            [
                "fn f() { | fn f() {",
                "    foo([1]) |     foo([2])",
                " |     bar() (whole)",
                "} | }",
            ]
        );
    }
}
