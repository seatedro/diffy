use std::ops::Range;

use diffy_diff::{Mode, Side};
use quark::selection::BlockKey;
use quark_render::scene::Rect;

use super::DiffViewState;
use super::frame::column_slot;
use super::navigation::SourcePoint;
use super::selection::block_key;
use super::state::{RowRef, source_line, store_index};

#[derive(Debug, Clone, PartialEq)]
pub struct HoverHit {
    pub point: SourcePoint,
    pub len: u32,
    pub line: String,
    pub rect: Rect,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn word_around(text: &str, at: usize) -> Option<Range<usize>> {
    let at = at.min(text.len());
    if !text.is_char_boundary(at) {
        return None;
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(i, _)| at + i);
    let first = text[start..end].chars().next()?;
    (start < end && !first.is_ascii_digit()).then_some(start..end)
}

impl DiffViewState {
    pub fn hover_at(&self, x: f32, y: f32) -> Option<HoverHit> {
        let bounds = self.bounds.get();
        let (x, y) = self.local(x, y);
        if x < 0.0 || y < 0.0 || x >= bounds.width || y >= bounds.height {
            return None;
        }
        let frame = self.frame.as_ref()?;
        let row = frame
            .rows
            .iter()
            .find(|r| y >= r.top && y < r.top + r.height)?;
        let Some(RowRef::Line { seg, row: r }) = self.refs.get(row.index as usize) else {
            return None;
        };
        let wanted = match self.mode {
            Mode::Split => self.side_at(x),
            Mode::Unified => Side::New,
        };
        let (side, index) = self.shown_line(seg as usize, r, wanted)?;
        let paint = row.paint.sides[side as usize].as_ref()?;
        if paint.merge.is_some() {
            return None;
        }
        let column = frame.columns.of(side);
        if x < column.text_x {
            return None;
        }
        let scroll = if frame.wrap {
            0.0
        } else {
            self.hscroll[column_slot(frame.columns.mode, side)]
                .offset_f64()
                .0
        };
        let (start, window_x) = paint.window.map_or((0, 0.0), |w| (w.start, w.x));
        let tx = f64::from(x - column.text_x - frame.metrics.text_pad) + scroll - window_x;
        let layout = &paint.layout;
        let hit = layout.hit(tx as f32, y - row.top).get();
        let word = word_around(layout.text(), hit)?;
        let rect = layout.selection_rects(word.clone()).next()?;
        let left =
            rect.x as f64 + f64::from(column.text_x + frame.metrics.text_pad) - scroll + window_x;
        if (tx as f32) < rect.x || tx as f32 > rect.x + rect.width {
            return None;
        }
        let segment = &self.segments[seg as usize];
        let file = segment.projection.file[r as usize];
        let unit = segment.unit(file);
        let text = segment.doc.text(file, side).display_line(index)?.to_owned();
        Some(HoverHit {
            point: SourcePoint {
                file: self.file_id(unit),
                side,
                line: source_line(&segment.doc, file, side, index),
                byte: u32::try_from(start + word.start).ok()?,
            },
            len: (word.end - word.start) as u32,
            line: text,
            rect: Rect {
                x: bounds.x + left as f32,
                y: bounds.y + row.top + rect.y,
                width: rect.width,
                height: rect.height,
            },
        })
    }

    pub fn set_link(&mut self, link: Option<(SourcePoint, u32)>) -> bool {
        let link = link.and_then(|(point, len)| {
            let unit = self.unit_of(point.file)?;
            let (seg, file) = self.locate(unit)?;
            let index = store_index(&self.segments[seg].doc, file, point.side, point.line)?;
            let start = point.byte as usize;
            Some((
                block_key(point.side, unit, index),
                start..start + len as usize,
            ))
        });
        if link == self.link {
            return false;
        }
        self.link = link;
        self.revision += 1;
        true
    }

    pub fn has_link(&self) -> bool {
        self.link.is_some()
    }

    pub(crate) fn row_link(
        &self,
        seg: usize,
        row: u32,
        side: Side,
        start: usize,
        len: usize,
    ) -> Option<(usize, usize)> {
        let (key, range): &(BlockKey, Range<usize>) = self.link.as_ref()?;
        let (shown, index) = self.shown_line(seg, row, side)?;
        let segment = &self.segments[seg];
        let unit = segment.unit(segment.projection.file[row as usize]);
        if block_key(shown, unit, index) != *key || shown != side {
            return None;
        }
        let lo = range.start.checked_sub(start)?;
        let hi = (range.end - start).min(len);
        (lo < hi).then_some((lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::word_around;

    #[test]
    fn words_are_identifiers_around_the_pointer() {
        let line = "let total = sum_of(items);";
        assert_eq!(word_around(line, 6), Some(4..9));
        assert_eq!(word_around(line, 13), Some(12..18));
        assert_eq!(word_around(line, 10), None);
        assert_eq!(word_around("x = 42;", 5), None);
    }
}
