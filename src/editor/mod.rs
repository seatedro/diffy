use glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Wrap};

const LINE_HEIGHT_FACTOR: f32 = 1.35;

#[derive(Debug, Clone, Copy)]
pub struct SelectionRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CursorState {
    pub x: f32,
    pub y: f32,
}

pub struct Editor {
    text: String,
    cursor: usize,
    anchor: usize,
    buffer: Option<Buffer>,
    dirty: bool,
    desired_x: Option<f32>,
    reveal_cursor_on_flush: bool,
    pub scroll_y: f32,
    pub cursor_pos: CursorState,
    pub cursor_moved_at_ms: u64,
    font_size: f32,
    last_width: f32,
    last_height: f32,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            anchor: 0,
            buffer: None,
            dirty: true,
            desired_x: None,
            reveal_cursor_on_flush: false,
            scroll_y: 0.0,
            cursor_pos: CursorState::default(),
            cursor_moved_at_ms: 0,
            font_size: 14.0,
            last_width: 0.0,
            last_height: 0.0,
        }
    }
}

impl Clone for Editor {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            buffer: None,
            dirty: true,
            desired_x: self.desired_x,
            reveal_cursor_on_flush: self.reveal_cursor_on_flush,
            scroll_y: self.scroll_y,
            cursor_pos: self.cursor_pos,
            cursor_moved_at_ms: self.cursor_moved_at_ms,
            font_size: self.font_size,
            last_width: self.last_width,
            last_height: self.last_height,
        }
    }
}

impl std::fmt::Debug for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Editor")
            .field("initialized", &self.buffer.is_some())
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .field("scroll_y", &self.scroll_y)
            .finish()
    }
}

fn next_char_boundary(text: &str, offset: usize) -> usize {
    let mut i = offset + 1;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i.min(text.len())
}

fn prev_char_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let mut i = offset - 1;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn prev_word_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let before = &text[..offset];
    let mut chars = before.char_indices().rev();
    while let Some((i, ch)) = chars.next() {
        if is_word_char(ch) {
            let mut result = i;
            for (j, c) in chars.by_ref() {
                if !is_word_char(c) {
                    result = j + c.len_utf8();
                    break;
                }
                result = j;
            }
            return result;
        }
    }
    0
}

fn next_word_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    let after = &text[offset..];
    let mut chars = after.char_indices().peekable();
    if chars.peek().is_some_and(|(_, ch)| is_word_char(*ch)) {
        for (i, ch) in chars.by_ref() {
            if !is_word_char(ch) {
                return offset + i;
            }
        }
        return text.len();
    }
    for (_i, ch) in chars.by_ref() {
        if is_word_char(ch) {
            for (j, c) in chars {
                if !is_word_char(c) {
                    return offset + j;
                }
            }
            return text.len();
        }
    }
    text.len()
}

impl Editor {
    fn line_height(&self) -> f32 {
        self.font_size * LINE_HEIGHT_FACTOR
    }

    fn metrics(&self) -> Metrics {
        Metrics::new(self.font_size, self.line_height())
    }

    fn mark_cursor_moved(&mut self) {
        self.cursor_moved_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
    }

    fn note_cursor_activity(&mut self) {
        self.reveal_cursor_on_flush = true;
        self.mark_cursor_moved();
    }

    fn global_to_line_col(&self, offset: usize) -> (usize, usize) {
        let mut line = 0;
        let mut line_start = 0;
        for (i, ch) in self.text.char_indices() {
            if i >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        if offset > self.text.len() {
            let last_newline = self.text.rfind('\n');
            match last_newline {
                Some(pos) => (line, self.text.len() - pos - 1),
                None => (0, self.text.len()),
            }
        } else {
            (line, offset - line_start)
        }
    }

    fn line_start(&self, target_line: usize) -> usize {
        if target_line == 0 {
            return 0;
        }
        let mut line = 0;
        for (i, ch) in self.text.char_indices() {
            if ch == '\n' {
                line += 1;
                if line == target_line {
                    return i + 1;
                }
            }
        }
        self.text.len()
    }

    fn line_end(&self, target_line: usize) -> usize {
        let mut line = 0;
        for (i, ch) in self.text.char_indices() {
            if ch == '\n' {
                if line == target_line {
                    return i;
                }
                line += 1;
            }
        }
        self.text.len()
    }

    fn has_selection(&self) -> bool {
        self.anchor != self.cursor
    }

    fn selection_range(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        let (start, end) = self.selection_range();
        self.text.drain(start..end);
        self.cursor = start;
        self.anchor = start;
        self.dirty = true;
        true
    }

    fn offset_to_point(&self, offset: usize) -> (f32, f32) {
        let buffer = match &self.buffer {
            Some(b) => b,
            None => return (0.0, 0.0),
        };
        let (target_line, target_col) = self.global_to_line_col(offset);
        let line_height = self.line_height();
        let mut y = 0.0_f32;

        for (line_idx, line) in buffer.lines.iter().enumerate() {
            if let Some(layout_lines) = line.layout_opt() {
                let num_layouts = layout_lines.len();
                for (layout_idx, layout_line) in layout_lines.iter().enumerate() {
                    if line_idx == target_line {
                        let first_start = layout_line.glyphs.first().map(|g| g.start).unwrap_or(0);
                        let last_end = layout_line.glyphs.last().map(|g| g.end).unwrap_or(0);
                        let is_last_layout = layout_idx == num_layouts - 1;
                        let on_this_layout = layout_line.glyphs.is_empty()
                            || (target_col >= first_start
                                && (target_col <= last_end || is_last_layout));

                        if on_this_layout {
                            let mut x = 0.0_f32;
                            for glyph in layout_line.glyphs.iter() {
                                if target_col <= glyph.start {
                                    x = glyph.x;
                                    return (x, y);
                                }
                                x = glyph.x + glyph.w;
                            }
                            return (x, y);
                        }
                    }
                    y += line_height;
                }
            } else {
                if line_idx == target_line {
                    return (0.0, y);
                }
                y += line_height;
            }
        }
        (0.0, y)
    }

    fn point_to_offset(&self, px: f32, py: f32) -> usize {
        let buffer = match &self.buffer {
            Some(b) => b,
            None => return 0,
        };
        let line_height = self.line_height();
        let mut y = 0.0_f32;
        let mut global_offset = 0_usize;

        for (line_idx, line) in buffer.lines.iter().enumerate() {
            let line_text = line.text();
            if let Some(layout_lines) = line.layout_opt() {
                let last_layout_idx = layout_lines.len().saturating_sub(1);
                for (layout_idx, layout_line) in layout_lines.iter().enumerate() {
                    let next_y = y + line_height;
                    let is_last_visual_line =
                        line_idx == buffer.lines.len() - 1 && layout_idx == last_layout_idx;
                    if py < next_y || (is_last_visual_line && py >= y) {
                        if layout_line.glyphs.is_empty() {
                            let first_start = 0;
                            return global_offset + first_start;
                        }
                        for glyph in layout_line.glyphs.iter() {
                            let mid = glyph.x + glyph.w * 0.5;
                            if px < mid {
                                return global_offset + glyph.start;
                            }
                        }
                        if let Some(last) = layout_line.glyphs.last() {
                            return global_offset + last.end;
                        }
                        return global_offset;
                    }
                    y = next_y;
                }
            } else {
                let next_y = y + line_height;
                let is_last_visual_line = line_idx == buffer.lines.len() - 1;
                if py < next_y || (is_last_visual_line && py >= y) {
                    return global_offset;
                }
                y = next_y;
            }
            global_offset += line_text.len() + line.ending().as_str().len();
        }
        self.text.len()
    }

    fn offset_at_visual_line_x(&self, target_y: f32, target_x: f32) -> usize {
        let buffer = match &self.buffer {
            Some(b) => b,
            None => return 0,
        };
        let line_height = self.line_height();
        let mut y = 0.0_f32;
        let mut global_offset = 0_usize;

        for (_line_idx, line) in buffer.lines.iter().enumerate() {
            let line_text = line.text();
            if let Some(layout_lines) = line.layout_opt() {
                for layout_line in layout_lines.iter() {
                    if (y - target_y).abs() < 0.5 {
                        if layout_line.glyphs.is_empty() {
                            return global_offset;
                        }
                        for glyph in layout_line.glyphs.iter() {
                            let mid = glyph.x + glyph.w * 0.5;
                            if target_x < mid {
                                return global_offset + glyph.start;
                            }
                        }
                        if let Some(last) = layout_line.glyphs.last() {
                            return global_offset + last.end;
                        }
                        return global_offset;
                    }
                    y += line_height;
                }
            } else {
                if (y - target_y).abs() < 0.5 {
                    return global_offset;
                }
                y += line_height;
            }
            global_offset += line_text.len() + line.ending().as_str().len();
        }
        self.text.len()
    }

    fn collect_visual_line_ys(&self) -> Vec<f32> {
        let buffer = match &self.buffer {
            Some(b) => b,
            None => return vec![0.0],
        };
        let line_height = self.line_height();
        let mut ys = Vec::new();
        let mut y = 0.0_f32;
        for line in buffer.lines.iter() {
            if let Some(layout_lines) = line.layout_opt() {
                for _ in layout_lines.iter() {
                    ys.push(y);
                    y += line_height;
                }
            } else {
                ys.push(y);
                y += line_height;
            }
        }
        if ys.is_empty() {
            ys.push(0.0);
        }
        ys
    }

    pub fn set_font_size(&mut self, font_system: &mut glyphon::FontSystem, font_size: f32) {
        if (self.font_size - font_size).abs() < 0.01 {
            return;
        }
        self.font_size = font_size;
        if let Some(buffer) = &mut self.buffer {
            let metrics = Metrics::new(font_size, font_size * LINE_HEIGHT_FACTOR);
            buffer.set_metrics(font_system, metrics);
        }
    }

    fn ensure_init(&mut self, font_system: &mut glyphon::FontSystem) {
        if self.buffer.is_none() {
            let mut buffer = Buffer::new(font_system, self.metrics());
            buffer.set_wrap(font_system, Wrap::WordOrGlyph);
            self.buffer = Some(buffer);
            self.dirty = true;
        }
    }

    pub fn request_clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.scroll_y = 0.0;
        self.desired_x = None;
        self.dirty = true;
        self.note_cursor_activity();
    }

    pub fn sync_size(&mut self, font_system: &mut glyphon::FontSystem, width: f32, height: f32) {
        let size_changed =
            (self.last_width - width).abs() > 0.5 || (self.last_height - height).abs() > 0.5;
        if !size_changed {
            return;
        }
        self.last_width = width;
        self.last_height = height;
        self.set_size(font_system, width, height);
    }

    pub fn flush(&mut self, font_system: &mut glyphon::FontSystem) {
        self.ensure_init(font_system);
        if self.last_width > 0.0 {
            self.set_size(font_system, self.last_width, self.last_height);
        }
        let buffer = self.buffer.as_mut().unwrap();
        if self.dirty {
            self.dirty = false;
            let attrs = Attrs::new().family(Family::SansSerif);
            buffer.set_text(font_system, &self.text, &attrs, Shaping::Advanced, None);
        }
        buffer.shape_until_scroll(font_system, false);
        let (x, y) = self.offset_to_point(self.cursor);
        self.cursor_pos = CursorState { x, y };
        let max_scroll = (self.content_height() - self.last_height).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        if std::mem::take(&mut self.reveal_cursor_on_flush) && self.last_height > 0.0 {
            let line_height = self.line_height();
            let cursor_bottom = y + line_height;
            if cursor_bottom > self.scroll_y + self.last_height {
                self.scroll_y = cursor_bottom - self.last_height;
            } else if y < self.scroll_y {
                self.scroll_y = y;
            }
            self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
        }
    }

    pub fn set_size(&mut self, font_system: &mut glyphon::FontSystem, width: f32, height: f32) {
        self.ensure_init(font_system);
        let buffer = self.buffer.as_mut().unwrap();
        let _ = height;
        // Leave buffer height unconstrained so cosmic-text lays out the full
        // document. The viewport height is tracked separately in `last_height`
        // for our own scrolling logic.
        buffer.set_size(font_system, Some(width.max(1.0)), None);
    }

    pub fn text(&self) -> String {
        self.text.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn content_height(&self) -> f32 {
        let line_height = self.line_height();
        let Some(buffer) = &self.buffer else {
            return line_height;
        };
        let mut total = 0.0_f32;
        for line in buffer.lines.iter() {
            if let Some(layout) = line.layout_opt() {
                total += layout.len() as f32 * line_height;
            } else {
                total += line_height;
            }
        }
        total.max(line_height)
    }

    pub fn scroll_line_height_px(&self) -> f32 {
        self.line_height()
    }

    pub fn selected_text(&self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let (start, end) = self.selection_range();
        let s = &self.text[start..end];
        if s.is_empty() {
            None
        } else {
            Some(s.to_string())
        }
    }

    pub fn selection_rects(&self) -> Vec<SelectionRect> {
        if !self.has_selection() {
            return Vec::new();
        }
        let Some(buffer) = &self.buffer else {
            return Vec::new();
        };
        let (sel_start, sel_end) = self.selection_range();
        let (start_line, start_col) = self.global_to_line_col(sel_start);
        let (end_line, end_col) = self.global_to_line_col(sel_end);
        let line_height = self.line_height();
        let mut rects = Vec::new();
        let mut y = 0.0_f32;

        for (line_idx, line) in buffer.lines.iter().enumerate() {
            if let Some(layout_lines) = line.layout_opt() {
                for layout_line in layout_lines.iter() {
                    let first_start = layout_line.glyphs.first().map(|g| g.start).unwrap_or(0);
                    let last_end = layout_line.glyphs.last().map(|g| g.end).unwrap_or(0);

                    let in_selection = if start_line == end_line && start_line == line_idx {
                        last_end > 0 && first_start < end_col && last_end > start_col
                    } else if line_idx == start_line {
                        last_end > start_col
                    } else if line_idx == end_line {
                        first_start < end_col
                    } else {
                        line_idx > start_line && line_idx < end_line
                    };

                    if in_selection {
                        let local_sel_start = if line_idx == start_line { start_col } else { 0 };
                        let local_sel_end = if line_idx == end_line {
                            end_col
                        } else {
                            usize::MAX
                        };

                        let mut x_start = 0.0_f32;
                        let mut x_end = 0.0_f32;
                        let mut found_start = false;

                        for glyph in layout_line.glyphs.iter() {
                            if !found_start && glyph.end > local_sel_start {
                                x_start = glyph.x;
                                found_start = true;
                            }
                            if glyph.start < local_sel_end {
                                x_end = glyph.x + glyph.w;
                            }
                        }

                        if !found_start && layout_line.glyphs.is_empty() {
                            x_start = 0.0;
                            x_end = line_height * 0.3;
                        }

                        if x_end > x_start || layout_line.glyphs.is_empty() {
                            rects.push(SelectionRect {
                                x: x_start,
                                y,
                                w: x_end - x_start,
                                h: line_height,
                            });
                        }
                    }

                    y += line_height;
                }
            } else {
                y += line_height;
            }
        }

        rects
    }

    pub fn buffer(&self) -> Option<&Buffer> {
        self.buffer.as_ref()
    }

    pub fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.anchor = self.cursor;
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn insert_text(&mut self, s: &str) {
        self.delete_selection();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
        self.anchor = self.cursor;
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn delete_backward(&mut self) {
        if self.delete_selection() {
            self.desired_x = None;
            self.note_cursor_activity();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.text, self.cursor);
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
        self.anchor = prev;
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            self.desired_x = None;
            self.note_cursor_activity();
            return;
        }
        if self.cursor >= self.text.len() {
            return;
        }
        let next = next_char_boundary(&self.text, self.cursor);
        self.text.drain(self.cursor..next);
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn delete_backward_word(&mut self) {
        if self.delete_selection() {
            self.desired_x = None;
            self.note_cursor_activity();
            return;
        }
        let target = prev_word_boundary(&self.text, self.cursor);
        if target == self.cursor {
            return;
        }
        self.text.drain(target..self.cursor);
        self.cursor = target;
        self.anchor = target;
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn delete_forward_word(&mut self) {
        if self.delete_selection() {
            self.desired_x = None;
            self.note_cursor_activity();
            return;
        }
        let target = next_word_boundary(&self.text, self.cursor);
        if target == self.cursor {
            return;
        }
        self.text.drain(self.cursor..target);
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn delete_backward_line(&mut self) {
        if self.delete_selection() {
            self.desired_x = None;
            self.note_cursor_activity();
            return;
        }
        let (line, _col) = self.global_to_line_col(self.cursor);
        let start = self.line_start(line);
        if start == self.cursor {
            return;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
        self.anchor = start;
        self.dirty = true;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn move_left(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        if !selecting && self.has_selection() {
            let (start, _end) = self.selection_range();
            self.cursor = start;
            self.anchor = self.cursor;
            self.desired_x = None;
            if self.cursor != previous_cursor || self.anchor != previous_anchor {
                self.note_cursor_activity();
            }
            return;
        }
        if self.cursor > 0 {
            self.cursor = prev_char_boundary(&self.text, self.cursor);
        }
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_right(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        if !selecting && self.has_selection() {
            let (_start, end) = self.selection_range();
            self.cursor = end;
            self.anchor = self.cursor;
            self.desired_x = None;
            if self.cursor != previous_cursor || self.anchor != previous_anchor {
                self.note_cursor_activity();
            }
            return;
        }
        if self.cursor < self.text.len() {
            self.cursor = next_char_boundary(&self.text, self.cursor);
        }
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_word_left(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        if !selecting && self.has_selection() {
            let (start, _end) = self.selection_range();
            self.cursor = start;
            self.anchor = self.cursor;
            self.desired_x = None;
            if self.cursor != previous_cursor || self.anchor != previous_anchor {
                self.note_cursor_activity();
            }
            return;
        }
        self.cursor = prev_word_boundary(&self.text, self.cursor);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_word_right(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        if !selecting && self.has_selection() {
            let (_start, end) = self.selection_range();
            self.cursor = end;
            self.anchor = self.cursor;
            self.desired_x = None;
            if self.cursor != previous_cursor || self.anchor != previous_anchor {
                self.note_cursor_activity();
            }
            return;
        }
        self.cursor = next_word_boundary(&self.text, self.cursor);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_home(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (line, _col) = self.global_to_line_col(self.cursor);
        self.cursor = self.line_start(line);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_end(&mut self, selecting: bool) {
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (line, _col) = self.global_to_line_col(self.cursor);
        self.cursor = self.line_end(line);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_soft_home(&mut self, selecting: bool) {
        if self.buffer.is_none() {
            self.move_home(selecting);
            return;
        }
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (cur_x, cur_y) = self.offset_to_point(self.cursor);
        let _ = cur_x;
        let ys = self.collect_visual_line_ys();
        let mut target_y = 0.0_f32;
        for &vy in &ys {
            if (vy - cur_y).abs() < 0.5 {
                target_y = vy;
                break;
            }
        }
        self.cursor = self.offset_at_visual_line_x(target_y, 0.0);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_soft_end(&mut self, selecting: bool) {
        if self.buffer.is_none() {
            self.move_end(selecting);
            return;
        }
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (_cur_x, cur_y) = self.offset_to_point(self.cursor);
        self.cursor = self.offset_at_visual_line_x(cur_y, f32::MAX);
        if !selecting {
            self.anchor = self.cursor;
        }
        self.desired_x = None;
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_up(&mut self, selecting: bool) {
        if self.buffer.is_none() {
            return;
        }
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (cur_x, cur_y) = self.offset_to_point(self.cursor);
        let target_x = self.desired_x.unwrap_or(cur_x);
        let ys = self.collect_visual_line_ys();
        let mut prev_y: Option<f32> = None;
        for &vy in &ys {
            if (vy - cur_y).abs() < 0.5 {
                break;
            }
            prev_y = Some(vy);
        }
        if let Some(py) = prev_y {
            self.cursor = self.offset_at_visual_line_x(py, target_x);
            if self.desired_x.is_none() {
                self.desired_x = Some(target_x);
            }
        } else if !selecting {
            self.cursor = 0;
            self.desired_x = None;
        }
        if !selecting {
            self.anchor = self.cursor;
        }
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn move_down(&mut self, selecting: bool) {
        if self.buffer.is_none() {
            return;
        }
        let previous_cursor = self.cursor;
        let previous_anchor = self.anchor;
        let (cur_x, cur_y) = self.offset_to_point(self.cursor);
        let target_x = self.desired_x.unwrap_or(cur_x);
        let ys = self.collect_visual_line_ys();
        let mut found_current = false;
        let mut next_y: Option<f32> = None;
        for &vy in &ys {
            if found_current {
                next_y = Some(vy);
                break;
            }
            if (vy - cur_y).abs() < 0.5 {
                found_current = true;
            }
        }
        if let Some(ny) = next_y {
            self.cursor = self.offset_at_visual_line_x(ny, target_x);
            if self.desired_x.is_none() {
                self.desired_x = Some(target_x);
            }
        } else if !selecting {
            self.cursor = self.text.len();
            self.desired_x = None;
        }
        if !selecting {
            self.anchor = self.cursor;
        }
        if self.cursor != previous_cursor || self.anchor != previous_anchor {
            self.note_cursor_activity();
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn click(&mut self, x: i32, y: i32) {
        let layout_y = y as f32 + self.scroll_y;
        let offset = self.point_to_offset(x as f32, layout_y);
        self.cursor = offset.min(self.text.len());
        self.anchor = self.cursor;
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn drag(&mut self, x: i32, y: i32) {
        let layout_y = y as f32 + self.scroll_y;
        let offset = self.point_to_offset(x as f32, layout_y);
        self.cursor = offset.min(self.text.len());
        self.desired_x = None;
        self.note_cursor_activity();
    }

    pub fn scroll(&mut self, delta_px: f32) {
        let max_scroll = (self.content_height() - self.last_height).max(0.0);
        self.scroll_y = (self.scroll_y + delta_px).clamp(0.0, max_scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_editor(width: f32, height: f32) -> (glyphon::FontSystem, Editor) {
        let mut font_system = glyphon::FontSystem::new();
        crate::fonts::configure_font_system(&mut font_system);

        let mut editor = Editor::default();
        editor.sync_size(&mut font_system, width, height);

        (font_system, editor)
    }

    #[test]
    fn manual_scroll_persists_across_flush() {
        let (mut font_system, mut editor) = make_editor(220.0, 2.0 * 14.0 * LINE_HEIGHT_FACTOR);

        editor.insert_text("line0\nline1\nline2\nline3");
        editor.flush(&mut font_system);

        let line_height = editor.scroll_line_height_px();
        assert!((editor.scroll_y - line_height * 2.0).abs() < 0.5);

        editor.scroll(-line_height);
        editor.flush(&mut font_system);

        assert!(
            (editor.scroll_y - line_height).abs() < 0.5,
            "expected manual scroll to persist, got {}",
            editor.scroll_y
        );
    }

    #[test]
    fn click_uses_visible_coordinates_after_manual_scroll() {
        let (mut font_system, mut editor) = make_editor(220.0, 2.0 * 14.0 * LINE_HEIGHT_FACTOR);

        editor.insert_text("line0\nline1\nline2\nline3");
        editor.flush(&mut font_system);

        let line_height = editor.scroll_line_height_px();
        editor.scroll(-line_height);
        editor.flush(&mut font_system);

        editor.click(0, 0);
        editor.flush(&mut font_system);

        assert_eq!(editor.cursor, "line0\n".len());
        assert_eq!(editor.anchor, "line0\n".len());
    }

    #[test]
    fn content_height_covers_cursor_line_for_trailing_newline() {
        let (mut font_system, mut editor) = make_editor(160.0, 2.0 * 14.0 * LINE_HEIGHT_FACTOR);

        editor.insert_text("first line\nsecond line\n");
        editor.flush(&mut font_system);

        let line_height = editor.scroll_line_height_px();
        let cursor_bottom = editor.cursor_pos.y + line_height;

        assert!(
            editor.content_height() + 0.5 >= cursor_bottom,
            "content height {} did not cover cursor bottom {}",
            editor.content_height(),
            cursor_bottom
        );
    }

    #[test]
    fn wrapped_content_beyond_viewport_height_is_fully_counted() {
        let (mut font_system, mut editor) = make_editor(120.0, 2.0 * 14.0 * LINE_HEIGHT_FACTOR);

        editor.insert_text(
            "asdf asf asdf asdf fasd fasd fasdf sdaf asdf sadf \
             sdaf asdf asdf asdf asd fasdf asdf asdf asdf asdf \
             asdf asf asdf asdf fasd fasd fasdf sdaf asdf sadf \
             sdaf asdf asdf asdf asd fasdf asdf asdf asdf asdf",
        );
        editor.flush(&mut font_system);

        let line_height = editor.scroll_line_height_px();
        let cursor_bottom = editor.cursor_pos.y + line_height;
        let viewport_height = 2.0 * 14.0 * LINE_HEIGHT_FACTOR;

        assert!(
            cursor_bottom > viewport_height + 0.5,
            "test text did not extend beyond the viewport: cursor_bottom={cursor_bottom} viewport_height={viewport_height}"
        );
        assert!(
            editor.content_height() + 0.5 >= cursor_bottom,
            "wrapped content height {} did not cover offscreen cursor bottom {}",
            editor.content_height(),
            cursor_bottom
        );
        assert!(
            (editor.scroll_y - (cursor_bottom - viewport_height)).abs() < line_height + 0.5,
            "expected scroll to reveal bottom line, got scroll_y={} cursor_bottom={} viewport_height={viewport_height}",
            editor.scroll_y,
            cursor_bottom
        );
    }

    #[test]
    fn click_can_target_lower_wrapped_visual_line() {
        let (mut font_system, mut editor) = make_editor(120.0, 10.0 * 14.0 * LINE_HEIGHT_FACTOR);

        editor.insert_text(
            "asdf asf asdf asdf fasd fasd fasdf sdaf asdf sadf \
             sdaf asdf asdf asdf asd fasdf asdf asdf asdf asdf",
        );
        editor.flush(&mut font_system);

        let line_height = editor.scroll_line_height_px();
        let buffer = editor.buffer().expect("buffer");
        let layout_lines = buffer.lines[0].layout_opt().expect("layout lines");
        assert!(layout_lines.len() >= 2, "expected wrapped text");
        let first_glyph = layout_lines[1]
            .glyphs
            .first()
            .expect("glyph on wrapped line");
        let target_offset = first_glyph.start;
        let (_target_x, target_y) = editor.offset_to_point(target_offset);
        let click_x = (first_glyph.x + first_glyph.w * 0.25) as i32;
        let click_y = (target_y - editor.scroll_y + line_height * 0.5) as i32;

        editor.click(click_x, click_y);
        editor.flush(&mut font_system);

        assert_eq!(editor.cursor, target_offset);
    }
}
