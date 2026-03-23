use std::mem;

use crate::core::rendering::flat_rows::DiffRowType;
use crate::core::rendering::prepared_rows::PreparedRow;

#[derive(Debug, Clone, PartialEq)]
pub struct DiffDisplayRow {
    pub row_type: DiffRowType,
    pub row_index: i32,
    pub file_index: i32,
    pub hunk_index: i32,
    pub left_row_index: i32,
    pub right_row_index: i32,
    pub y: f64,
    pub height: f64,
    pub wrap_line_count: i32,
    pub left_wrap_line_count: i32,
    pub right_wrap_line_count: i32,
}

impl Default for DiffDisplayRow {
    fn default() -> Self {
        Self {
            row_type: DiffRowType::FileHeader,
            row_index: -1,
            file_index: -1,
            hunk_index: -1,
            left_row_index: -1,
            right_row_index: -1,
            y: 0.0,
            height: 0.0,
            wrap_line_count: 1,
            left_wrap_line_count: 1,
            right_wrap_line_count: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiffLayoutConfig {
    pub mode: String,
    pub row_height: f64,
    pub file_header_height: f64,
    pub hunk_height: f64,
    pub gutter_width: f64,
    pub available_width: f64,
    pub wrap_enabled: bool,
    pub wrap_column: i32,
    pub char_width: f64,
}

impl Default for DiffLayoutConfig {
    fn default() -> Self {
        Self {
            mode: "unified".to_owned(),
            row_height: 20.0,
            file_header_height: 28.0,
            hunk_height: 24.0,
            gutter_width: 50.0,
            available_width: 800.0,
            wrap_enabled: false,
            wrap_column: 0,
            char_width: 8.0,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct DiffLayoutEngine {
    rows: Vec<DiffDisplayRow>,
    total_height: f64,
    max_text_width: f64,
    alt_rows: Vec<DiffDisplayRow>,
    alt_total_height: f64,
    alt_max_text_width: f64,
}

impl DiffLayoutEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rebuild(&mut self, prepared: &[PreparedRow], config: &DiffLayoutConfig) {
        let (rows, total_height, max_text_width) = build_rows(prepared, config);
        self.rows = rows;
        self.total_height = total_height;
        self.max_text_width = max_text_width;
    }

    pub fn rows(&self) -> &[DiffDisplayRow] {
        &self.rows
    }

    pub fn total_height(&self) -> f64 {
        self.total_height
    }

    pub fn max_text_width(&self) -> f64 {
        self.max_text_width
    }

    pub fn row_at_y(&self, y: f64) -> Option<usize> {
        row_at_y_in(&self.rows, y)
    }

    pub fn y_for_row(&self, index: usize) -> f64 {
        self.rows.get(index).map_or(self.total_height, |row| row.y)
    }

    pub fn first_visible_row(&self, viewport_y: f64) -> usize {
        self.row_at_y(viewport_y).unwrap_or(0)
    }

    pub fn last_visible_row(&self, viewport_y: f64, viewport_height: f64) -> usize {
        self.row_at_y(viewport_y + viewport_height.max(0.0))
            .unwrap_or(0)
    }

    pub fn rebuild_alternate(&mut self, prepared: &[PreparedRow], config: &DiffLayoutConfig) {
        let (rows, total_height, max_text_width) = build_rows(prepared, config);
        self.alt_rows = rows;
        self.alt_total_height = total_height;
        self.alt_max_text_width = max_text_width;
    }

    pub fn swap_alternate(&mut self) {
        mem::swap(&mut self.rows, &mut self.alt_rows);
        mem::swap(&mut self.total_height, &mut self.alt_total_height);
        mem::swap(&mut self.max_text_width, &mut self.alt_max_text_width);
    }
}

fn build_rows(
    prepared: &[PreparedRow],
    config: &DiffLayoutConfig,
) -> (Vec<DiffDisplayRow>, f64, f64) {
    let mut rows = if config.mode == "split" {
        build_split_rows(prepared)
    } else {
        build_unified_rows(prepared)
    };

    let mut y = 0.0;
    let mut max_text_width: f64 = 0.0;
    for row in &mut rows {
        row.y = y;
        if row.row_type == DiffRowType::FileHeader {
            row.height = config.file_header_height;
        } else if row.row_type == DiffRowType::HunkSeparator {
            row.height = config.hunk_height;
        } else if config.mode == "split" {
            row.left_wrap_line_count = wrap_count(
                prepared,
                row.left_row_index,
                split_wrap_width(config),
                config.wrap_enabled,
            );
            row.right_wrap_line_count = wrap_count(
                prepared,
                row.right_row_index,
                split_wrap_width(config),
                config.wrap_enabled,
            );
            row.wrap_line_count = row.left_wrap_line_count.max(row.right_wrap_line_count);
            row.height = config.row_height * f64::from(row.wrap_line_count.max(1));
        } else {
            row.wrap_line_count = wrap_count(
                prepared,
                row.row_index,
                unified_wrap_width(config),
                config.wrap_enabled,
            );
            row.left_wrap_line_count = row.wrap_line_count;
            row.right_wrap_line_count = row.wrap_line_count;
            row.height = config.row_height * f64::from(row.wrap_line_count.max(1));
        }
        y += row.height;
        if let Some(width) = active_row_width_with_prepared(prepared, row) {
            max_text_width = max_text_width.max(width);
        }
    }

    (rows, y, max_text_width)
}

fn build_unified_rows(prepared: &[PreparedRow]) -> Vec<DiffDisplayRow> {
    let mut rows = Vec::with_capacity(prepared.len());
    for (index, row) in prepared.iter().enumerate() {
        rows.push(DiffDisplayRow {
            row_type: row.flat.row_type,
            row_index: index as i32,
            file_index: row.flat.file_index,
            hunk_index: row.flat.hunk_index,
            ..DiffDisplayRow::default()
        });
    }
    rows
}

fn build_split_rows(prepared: &[PreparedRow]) -> Vec<DiffDisplayRow> {
    let mut rows = Vec::with_capacity(prepared.len());
    let mut index = 0;

    while index < prepared.len() {
        let current = &prepared[index];
        match current.flat.row_type {
            DiffRowType::FileHeader | DiffRowType::HunkSeparator => {
                rows.push(DiffDisplayRow {
                    row_type: current.flat.row_type,
                    row_index: index as i32,
                    file_index: current.flat.file_index,
                    hunk_index: current.flat.hunk_index,
                    ..DiffDisplayRow::default()
                });
                index += 1;
            }
            DiffRowType::Context => {
                rows.push(DiffDisplayRow {
                    row_type: DiffRowType::Context,
                    row_index: index as i32,
                    file_index: current.flat.file_index,
                    hunk_index: current.flat.hunk_index,
                    left_row_index: index as i32,
                    right_row_index: index as i32,
                    ..DiffDisplayRow::default()
                });
                index += 1;
            }
            DiffRowType::Modified => {
                rows.push(DiffDisplayRow {
                    row_type: DiffRowType::Modified,
                    row_index: index as i32,
                    file_index: current.flat.file_index,
                    hunk_index: current.flat.hunk_index,
                    left_row_index: index as i32,
                    right_row_index: index as i32,
                    ..DiffDisplayRow::default()
                });
                index += 1;
            }
            DiffRowType::Removed | DiffRowType::Added => {
                let block_start = index;
                let mut left_rows = Vec::new();
                let mut right_rows = Vec::new();

                while index < prepared.len() {
                    match prepared[index].flat.row_type {
                        DiffRowType::Removed => left_rows.push(index as i32),
                        DiffRowType::Added => right_rows.push(index as i32),
                        _ => break,
                    }
                    index += 1;
                }

                let row_count = left_rows.len().max(right_rows.len());
                for pair_index in 0..row_count {
                    let left_row_index = left_rows.get(pair_index).copied().unwrap_or(-1);
                    let right_row_index = right_rows.get(pair_index).copied().unwrap_or(-1);
                    let row_type = match (left_row_index >= 0, right_row_index >= 0) {
                        (true, true) => DiffRowType::Modified,
                        (true, false) => DiffRowType::Removed,
                        (false, true) => DiffRowType::Added,
                        (false, false) => current.flat.row_type,
                    };
                    rows.push(DiffDisplayRow {
                        row_type,
                        row_index: block_start as i32,
                        file_index: current.flat.file_index,
                        hunk_index: current.flat.hunk_index,
                        left_row_index,
                        right_row_index,
                        ..DiffDisplayRow::default()
                    });
                }
            }
        }
    }

    rows
}

fn unified_wrap_width(config: &DiffLayoutConfig) -> f64 {
    apply_wrap_column(
        (config.available_width - config.gutter_width).max(0.0),
        config,
    )
}

fn split_wrap_width(config: &DiffLayoutConfig) -> f64 {
    let width = ((config.available_width - (config.gutter_width * 2.0)) / 2.0).max(0.0);
    apply_wrap_column(width, config)
}

fn apply_wrap_column(width: f64, config: &DiffLayoutConfig) -> f64 {
    if config.wrap_column > 0 && config.char_width > 0.0 {
        width.min(f64::from(config.wrap_column) * config.char_width)
    } else {
        width
    }
}

fn wrap_count(
    prepared: &[PreparedRow],
    row_index: i32,
    available_width: f64,
    wrap_enabled: bool,
) -> i32 {
    if row_index < 0 {
        return 1;
    }
    let Some(row) = usize::try_from(row_index)
        .ok()
        .and_then(|index| prepared.get(index))
    else {
        return 1;
    };
    if wrap_enabled {
        line_count_for_width(row.measured_width, available_width)
    } else {
        1
    }
}

fn line_count_for_width(text_width: f64, available_width: f64) -> i32 {
    if text_width <= 0.0 || available_width <= 0.0 {
        return 1;
    }
    (text_width / available_width).ceil().max(1.0) as i32
}

fn active_row_width_with_prepared(prepared: &[PreparedRow], row: &DiffDisplayRow) -> Option<f64> {
    match row.row_type {
        DiffRowType::FileHeader | DiffRowType::HunkSeparator => None,
        _ if row.left_row_index >= 0 || row.right_row_index >= 0 => Some(
            prepared_width(prepared, row.left_row_index)
                .max(prepared_width(prepared, row.right_row_index)),
        ),
        _ => Some(prepared_width(prepared, row.row_index)),
    }
}

fn prepared_width(prepared: &[PreparedRow], row_index: i32) -> f64 {
    usize::try_from(row_index)
        .ok()
        .and_then(|index| prepared.get(index))
        .map_or(0.0, |row| row.measured_width)
}

fn row_at_y_in(rows: &[DiffDisplayRow], y: f64) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let index = rows.partition_point(|row| row.y <= y);
    Some(index.saturating_sub(1).min(rows.len() - 1))
}

#[cfg(test)]
mod tests {
    use super::{DiffLayoutConfig, DiffLayoutEngine};
    use crate::core::diff::types::{DiffLine, FileDiff, Hunk, LineKind};
    use crate::core::rendering::flat_rows::flatten_file_diff;
    use crate::core::rendering::prepared_rows::prepare_rows;
    use crate::core::text::buffer::TextBuffer;

    fn prepare(
        file: &FileDiff,
        text_buffer: &TextBuffer,
    ) -> Vec<crate::core::rendering::prepared_rows::PreparedRow> {
        let flat = flatten_file_diff(file, 0);
        prepare_rows(&flat, std::slice::from_ref(file), text_buffer, &|text| {
            text.len() as f64 * 10.0
        })
    }

    fn append_line(
        text_buffer: &mut TextBuffer,
        kind: LineKind,
        old: Option<i32>,
        new: Option<i32>,
        text: &str,
    ) -> DiffLine {
        DiffLine {
            kind,
            old_line_number: old,
            new_line_number: new,
            text_range: text_buffer.append(text),
            ..DiffLine::default()
        }
    }

    #[test]
    fn rebuild_unified_assigns_y_positions_and_heights() {
        let mut text_buffer = TextBuffer::default();
        let file = FileDiff {
            path: "src/example.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1,2 @@".to_owned(),
                lines: vec![
                    append_line(&mut text_buffer, LineKind::Context, Some(1), Some(1), "ctx"),
                    append_line(&mut text_buffer, LineKind::Added, None, Some(2), "added"),
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };
        let prepared = prepare(&file, &text_buffer);
        let mut engine = DiffLayoutEngine::new();
        let config = DiffLayoutConfig {
            row_height: 10.0,
            file_header_height: 14.0,
            hunk_height: 12.0,
            ..DiffLayoutConfig::default()
        };

        engine.rebuild(&prepared, &config);

        let rows = engine.rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].y, 0.0);
        assert_eq!(rows[0].height, 14.0);
        assert_eq!(rows[1].y, 14.0);
        assert_eq!(rows[1].height, 12.0);
        assert_eq!(rows[2].y, 26.0);
        assert_eq!(rows[2].height, 10.0);
        assert_eq!(rows[3].y, 36.0);
        assert_eq!(rows[3].height, 10.0);
        assert_eq!(engine.total_height(), 46.0);
    }

    #[test]
    fn rebuild_split_pairs_left_and_right_rows() {
        let mut text_buffer = TextBuffer::default();
        let file = FileDiff {
            path: "src/example.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -10,3 +10,2 @@".to_owned(),
                lines: vec![
                    append_line(
                        &mut text_buffer,
                        LineKind::Removed,
                        Some(10),
                        None,
                        "old one",
                    ),
                    append_line(
                        &mut text_buffer,
                        LineKind::Removed,
                        Some(11),
                        None,
                        "old two",
                    ),
                    append_line(&mut text_buffer, LineKind::Added, None, Some(10), "new one"),
                    append_line(
                        &mut text_buffer,
                        LineKind::Context,
                        Some(12),
                        Some(11),
                        "ctx",
                    ),
                ],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };
        let prepared = prepare(&file, &text_buffer);
        let mut engine = DiffLayoutEngine::new();
        let config = DiffLayoutConfig {
            mode: "split".to_owned(),
            row_height: 10.0,
            file_header_height: 14.0,
            hunk_height: 12.0,
            available_width: 120.0,
            gutter_width: 10.0,
            wrap_enabled: true,
            ..DiffLayoutConfig::default()
        };

        engine.rebuild(&prepared, &config);

        let rows = engine.rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[2].left_row_index, 2);
        assert_eq!(rows[2].right_row_index, 2);
        assert_eq!(
            rows[2].row_type,
            crate::core::rendering::flat_rows::DiffRowType::Modified
        );
        assert_eq!(rows[3].left_row_index, 3);
        assert_eq!(rows[3].right_row_index, -1);
        assert_eq!(
            rows[3].row_type,
            crate::core::rendering::flat_rows::DiffRowType::Removed
        );
        assert_eq!(rows[4].left_row_index, 4);
        assert_eq!(rows[4].right_row_index, 4);
        assert_eq!(
            rows[4].row_type,
            crate::core::rendering::flat_rows::DiffRowType::Context
        );
    }

    #[test]
    fn binary_search_coordinate_lookups_match_row_boundaries() {
        let mut text_buffer = TextBuffer::default();
        let file = FileDiff {
            path: "src/example.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![append_line(
                    &mut text_buffer,
                    LineKind::Context,
                    Some(1),
                    Some(1),
                    "ctx",
                )],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };
        let prepared = prepare(&file, &text_buffer);
        let mut engine = DiffLayoutEngine::new();
        let config = DiffLayoutConfig {
            row_height: 10.0,
            file_header_height: 14.0,
            hunk_height: 12.0,
            ..DiffLayoutConfig::default()
        };

        engine.rebuild(&prepared, &config);

        assert_eq!(engine.row_at_y(-5.0), Some(0));
        assert_eq!(engine.row_at_y(0.0), Some(0));
        assert_eq!(engine.row_at_y(13.9), Some(0));
        assert_eq!(engine.row_at_y(14.0), Some(1));
        assert_eq!(engine.row_at_y(27.0), Some(2));
        assert_eq!(engine.first_visible_row(14.0), 1);
        assert_eq!(engine.last_visible_row(14.0, 11.0), 2);
        assert_eq!(engine.y_for_row(2), 26.0);
    }

    #[test]
    fn wrapping_uses_available_width() {
        let mut text_buffer = TextBuffer::default();
        let file = FileDiff {
            path: "src/example.rs".to_owned(),
            hunks: vec![Hunk {
                header: "@@ -1 +1 @@".to_owned(),
                lines: vec![append_line(
                    &mut text_buffer,
                    LineKind::Added,
                    None,
                    Some(1),
                    "abcdefghijkl",
                )],
                ..Hunk::default()
            }],
            ..FileDiff::default()
        };
        let prepared = prepare(&file, &text_buffer);
        let mut engine = DiffLayoutEngine::new();
        let config = DiffLayoutConfig {
            row_height: 10.0,
            file_header_height: 14.0,
            hunk_height: 12.0,
            available_width: 60.0,
            gutter_width: 10.0,
            wrap_enabled: true,
            ..DiffLayoutConfig::default()
        };

        engine.rebuild(&prepared, &config);

        let rows = engine.rows();
        assert_eq!(rows[2].wrap_line_count, 3);
        assert_eq!(rows[2].height, 30.0);
        assert_eq!(engine.total_height(), 56.0);
    }
}
