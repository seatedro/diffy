use diffy_diff::{Mode, Side};
use quark::selection::Selection;
use quark_ui::element::WHEEL_LINE_PX;

#[cfg(doc)]
use super::diff_view;
use super::navigation::FileId;
use super::prepared::DiffPreviewLimit;
use super::presentation::{DiffAppearance, DiffLayout, DiffPresentation};
use super::search::SearchDirection;
use super::{DiffEvent, DiffKey, DiffOutcome, DiffStyle, DiffViewState, Drag};

/// Autoscroll speed per point the pointer is past the edge band, in
/// points per millisecond.
const AUTOSCROLL_GAIN: f32 = 0.02;
/// Fastest autoscroll, in points per millisecond.
const AUTOSCROLL_MAX: f32 = 4.0;
/// Longest frame gap autoscroll integrates over, so a stalled frame does
/// not jump the view.
const AUTOSCROLL_MAX_DT_MS: u64 = 50;
/// Frame interval the view asks for while a drag autoscrolls.
pub(crate) const AUTOSCROLL_FRAME_MS: u64 = 16;

impl DiffViewState {
    pub fn set_style(&mut self, style: DiffStyle) {
        if style == self.style {
            return;
        }
        let anchor = self.anchor();
        self.style = style;
        self.metrics = None;
        self.painted.clear();
        self.content_w = [0.0; 2];
        self.rebuild_rows(anchor);
    }

    /// Changes how the diff is drawn; see [`DiffPresentation`]. An explicit
    /// layout switches the mode as [`Self::set_mode`] does; an automatic one
    /// is resolved against the viewport on the next [`Self::prepare`].
    pub fn set_presentation(&mut self, presentation: DiffPresentation) {
        if presentation == self.presentation {
            return;
        }
        let columns_moved = (
            presentation.numbers,
            presentation.markers,
            presentation.wrap_column,
        ) != (
            self.presentation.numbers,
            self.presentation.markers,
            self.presentation.wrap_column,
        );
        let bands_moved = (presentation.separators, presentation.headers)
            != (self.presentation.separators, self.presentation.headers);
        self.presentation = presentation;
        match presentation.layout {
            DiffLayout::Unified => self.switch_mode(Mode::Unified),
            DiffLayout::Split => self.switch_mode(Mode::Split),
            DiffLayout::Auto { .. } => {}
        }
        if columns_moved && self.style.wrap {
            // Wrapped rows were measured for the old text width.
            self.painted.clear();
            self.list.invalidate_all();
        }
        if bands_moved {
            // Header and separator rows change height.
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        }
        self.revision += 1;
    }

    /// Local color overrides over the theme's.
    pub fn set_appearance(&mut self, appearance: DiffAppearance) {
        if appearance != self.appearance {
            self.appearance = appearance;
            self.revision += 1;
        }
    }

    /// Bounds the view to a compact preview of at most `limit.max_rows`
    /// rows, ending in a row that counts the rest and offers
    /// [`DiffOutcome::OpenFull`]; `None` shows every row.
    pub fn set_preview_limit(&mut self, limit: Option<DiffPreviewLimit>) {
        if limit != self.preview {
            self.preview = limit;
            self.rebuild_rows(None);
        }
    }

    /// The view's size in points.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if self.viewport != (width, height) {
            if self.style.wrap && self.viewport.0 != width {
                self.painted.clear();
            }
            self.viewport = (width.max(0.0), height.max(0.0));
            self.list.set_viewport_height(self.viewport.1);
            self.revision += 1;
        }
    }

    /// Folds a file's rows under its header, or unfolds them. Returns
    /// whether that changed anything.
    pub fn set_file_collapsed(&mut self, file: FileId, collapsed: bool) -> bool {
        let Some(unit) = self.unit_of(file).filter(|&u| self.locate(u).is_some()) else {
            return false;
        };
        let changed = if collapsed {
            self.collapsed.insert(unit)
        } else {
            self.collapsed.remove(&unit)
        };
        if changed {
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        }
        changed
    }

    pub fn is_file_collapsed(&self, file: FileId) -> bool {
        self.unit_of(file)
            .is_some_and(|unit| self.collapsed.contains(&unit))
    }

    pub fn set_selection(&mut self, selection: Option<Selection>, side: Side) {
        self.selection = selection;
        self.selection_side = side;
        self.revision += 1;
    }

    pub(super) fn set_scroll(&mut self, offset: f64) -> bool {
        let before = self.list.scroll_offset();
        if self.list.set_scroll_offset(offset) == before {
            return false;
        }
        self.revision += 1;
        true
    }

    // ---- Input ---------------------------------------------------------

    /// Applies an event from [`diff_view`].
    pub fn handle(&mut self, event: DiffEvent) -> DiffOutcome {
        let changed = match event {
            DiffEvent::Press { x, y } => {
                let (x, y) = self.local(x, y);
                self.selection_side = self.side_at(x);
                self.drag = Some(Drag {
                    pointer: (x, y),
                    last_ms: None,
                });
                self.selection = self.point_at(x, y).map(Selection::collapsed);
                self.revision += 1;
                true
            }
            DiffEvent::Drag { x, y } => {
                let (x, y) = self.local(x, y);
                if let Some(drag) = &mut self.drag {
                    drag.pointer = (x, y);
                }
                match (self.selection, self.point_at(x, y)) {
                    (Some(s), Some(point)) if s.focus != point => {
                        self.selection = Some(Selection::new(s.anchor, point));
                        self.revision += 1;
                        true
                    }
                    _ => false,
                }
            }
            DiffEvent::Release => {
                self.drag = None;
                false
            }
            DiffEvent::Scroll(lines) => self.set_scroll(
                self.list.scroll_offset() + f64::from(lines) * f64::from(WHEEL_LINE_PX),
            ),
            DiffEvent::ScrollTo(offset) => self.set_scroll(f64::from(offset)),
            DiffEvent::Expand(gap, reveal) => {
                self.reveal(gap, reveal, self.context_policy.reveal_step)
            }
            DiffEvent::Key(key) => return self.key(key),
            DiffEvent::OpenFull => {
                return match self.preview_target() {
                    Some(target) => DiffOutcome::OpenFull { target },
                    None => DiffOutcome::Unchanged,
                };
            }
            DiffEvent::Annotate { file, side, line } => {
                let file = self.file_id(file);
                return match self.anchor_for(file, side, line..line + 1) {
                    Some(anchor) => DiffOutcome::Annotate { anchor },
                    None => DiffOutcome::Unchanged,
                };
            }
            DiffEvent::ToggleFile { file } => {
                let collapsed = self.collapsed.contains(&file);
                let id = self.file_id(file);
                self.set_file_collapsed(id, !collapsed)
            }
            DiffEvent::AnnotationMeasured {
                id,
                revision,
                height,
            } => self.set_annotation_height(id, revision, height),
        };
        if changed {
            DiffOutcome::Changed
        } else {
            DiffOutcome::Unchanged
        }
    }

    fn key(&mut self, key: DiffKey) -> DiffOutcome {
        let line = f64::from(self.metrics().line_h);
        let page = (f64::from(self.viewport.1) - line * 2.0).max(line);
        let offset = self.list.scroll_offset();
        let moved = match key {
            DiffKey::NextHunk => self.jump(true, true),
            DiffKey::PrevHunk => self.jump(false, true),
            DiffKey::NextFile => self.jump(true, false),
            DiffKey::PrevFile => self.jump(false, false),
            DiffKey::LineUp => self.set_scroll(offset - line),
            DiffKey::LineDown => self.set_scroll(offset + line),
            DiffKey::PageUp => self.set_scroll(offset - page),
            DiffKey::PageDown => self.set_scroll(offset + page),
            DiffKey::Home => self.set_scroll(0.0),
            DiffKey::End => self.set_scroll(f64::MAX),
            DiffKey::Copy => return DiffOutcome::Copy(self.selected_text()),
            DiffKey::SelectAll => {
                self.select_all();
                true
            }
            DiffKey::NextMatch => self.next_match(SearchDirection::Forward).is_some(),
            DiffKey::PrevMatch => self.next_match(SearchDirection::Backward).is_some(),
            DiffKey::Annotate => {
                return match self.annotate_selection_or_focus() {
                    Some(anchor) => DiffOutcome::Annotate { anchor },
                    None => DiffOutcome::Unchanged,
                };
            }
        };
        if moved {
            DiffOutcome::Changed
        } else {
            DiffOutcome::Unchanged
        }
    }

    // ---- Autoscroll ----------------------------------------------------

    /// A drag is held past the top or bottom edge: the view scrolls each
    /// frame, so it needs another one.
    pub fn wants_frame(&self) -> bool {
        self.drag
            .is_some_and(|drag| self.autoscroll_velocity(drag.pointer.1) != 0.0)
    }

    /// Signed autoscroll speed in points per millisecond for a pointer at
    /// view-local `y`: zero away from the edges, growing with the distance
    /// into or past a one-line band at the top or bottom.
    fn autoscroll_velocity(&self, y: f32) -> f32 {
        let (edge, height) = (self.metrics().line_h, self.viewport.1);
        let past = if y < edge {
            y - edge
        } else if y > height - edge {
            y - (height - edge)
        } else {
            0.0
        };
        (past * AUTOSCROLL_GAIN).clamp(-AUTOSCROLL_MAX, AUTOSCROLL_MAX)
    }

    /// Scroll a held drag past an edge by the time since its last step.
    /// Returns whether the view moved.
    pub(super) fn autoscroll(&mut self, now_ms: u64) -> bool {
        let Some(mut drag) = self.drag else {
            return false;
        };
        let velocity = self.autoscroll_velocity(drag.pointer.1);
        let mut moved = false;
        if velocity == 0.0 {
            drag.last_ms = None;
        } else {
            if let Some(last) = drag.last_ms {
                let dt = now_ms.saturating_sub(last).min(AUTOSCROLL_MAX_DT_MS);
                let offset = self.list.scroll_offset() + f64::from(velocity) * dt as f64;
                moved = self.set_scroll(offset);
            }
            drag.last_ms = Some(now_ms);
        }
        self.drag = Some(drag);
        moved
    }

    /// Move the selection's focus to whatever now sits under the held
    /// pointer. Returns whether it changed.
    pub(super) fn follow_pointer(&mut self) -> bool {
        let (Some(drag), Some(selection)) = (self.drag, self.selection) else {
            return false;
        };
        match self.point_at(drag.pointer.0, drag.pointer.1) {
            Some(point) if point != selection.focus => {
                self.selection = Some(Selection::new(selection.anchor, point));
                self.revision += 1;
                true
            }
            _ => false,
        }
    }
}
