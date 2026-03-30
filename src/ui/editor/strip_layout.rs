use std::ops::Range;

use super::render_doc::DisplayRow;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StripLayout {
    pub strip_id: u32,
    pub top_px: u32,
    pub height_px: u32,
    pub row_start: usize,
    pub row_end: usize,
}

impl StripLayout {
    pub fn bottom_px(&self) -> u32 {
        self.top_px.saturating_add(self.height_px)
    }
}

pub fn build_strip_layouts(
    rows: &[DisplayRow],
    target_height_px: u32,
    output: &mut Vec<StripLayout>,
) {
    output.clear();
    if rows.is_empty() {
        return;
    }

    output.reserve(rows.len().saturating_div(16).max(1));

    let mut row_start = 0usize;
    while row_start < rows.len() {
        let top_px = rows[row_start].y_px;
        let mut row_end = row_start;
        let mut bottom_px = top_px;

        while row_end < rows.len() {
            bottom_px = rows[row_end].bottom_px();
            row_end += 1;
            if row_end < rows.len() && bottom_px.saturating_sub(top_px) >= target_height_px {
                break;
            }
        }

        output.push(StripLayout {
            strip_id: row_start as u32,
            top_px,
            height_px: bottom_px.saturating_sub(top_px),
            row_start,
            row_end,
        });
        row_start = row_end;
    }
}

pub fn visible_strip_range(
    strips: &[StripLayout],
    viewport_top_px: u32,
    viewport_height_px: u32,
    overscan: usize,
) -> Range<usize> {
    if strips.is_empty() {
        return 0..0;
    }

    let viewport_bottom = viewport_top_px.saturating_add(viewport_height_px.max(1));
    let first_visible = strips.partition_point(|strip| strip.bottom_px() <= viewport_top_px);
    let last_visible = strips.partition_point(|strip| strip.top_px < viewport_bottom);
    let start = first_visible.saturating_sub(overscan);
    let end = last_visible.saturating_add(overscan).min(strips.len());
    start..end
}

#[cfg(test)]
mod tests {
    use super::{StripLayout, build_strip_layouts, visible_strip_range};
    use crate::ui::editor::render_doc::DisplayRow;

    #[test]
    fn strip_layouts_break_on_row_boundaries_instead_of_fixed_pixels() {
        let rows = vec![
            DisplayRow {
                y_px: 0,
                h_px: 32,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 32,
                h_px: 24,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 56,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 78,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 100,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 122,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 144,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 166,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 188,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 210,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 232,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 254,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 276,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 298,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 320,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 342,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 364,
                h_px: 22,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 386,
                h_px: 22,
                ..DisplayRow::default()
            },
        ];
        let mut strips = Vec::new();

        build_strip_layouts(&rows, 384, &mut strips);

        assert_eq!(
            strips,
            vec![
                StripLayout {
                    strip_id: 0,
                    top_px: 0,
                    height_px: 386,
                    row_start: 0,
                    row_end: 17,
                },
                StripLayout {
                    strip_id: 17,
                    top_px: 386,
                    height_px: 22,
                    row_start: 17,
                    row_end: 18,
                },
            ]
        );
    }

    #[test]
    fn strip_layouts_cover_rows_without_overlap_or_gaps() {
        let rows = vec![
            DisplayRow {
                y_px: 0,
                h_px: 20,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 20,
                h_px: 20,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 40,
                h_px: 60,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 100,
                h_px: 20,
                ..DisplayRow::default()
            },
            DisplayRow {
                y_px: 120,
                h_px: 20,
                ..DisplayRow::default()
            },
        ];
        let mut strips = Vec::new();

        build_strip_layouts(&rows, 64, &mut strips);

        assert_eq!(strips.first().map(|strip| strip.row_start), Some(0));
        assert_eq!(strips.last().map(|strip| strip.row_end), Some(rows.len()));
        for pair in strips.windows(2) {
            assert_eq!(pair[0].row_end, pair[1].row_start);
            assert_eq!(pair[0].bottom_px(), pair[1].top_px);
        }
    }

    #[test]
    fn visible_strip_range_includes_overscan_neighbors() {
        let strips = vec![
            StripLayout {
                strip_id: 0,
                top_px: 0,
                height_px: 200,
                row_start: 0,
                row_end: 5,
            },
            StripLayout {
                strip_id: 5,
                top_px: 200,
                height_px: 200,
                row_start: 5,
                row_end: 10,
            },
            StripLayout {
                strip_id: 10,
                top_px: 400,
                height_px: 200,
                row_start: 10,
                row_end: 15,
            },
            StripLayout {
                strip_id: 15,
                top_px: 600,
                height_px: 200,
                row_start: 15,
                row_end: 20,
            },
        ];

        assert_eq!(visible_strip_range(&strips, 250, 100, 1), 0..3);
        assert_eq!(visible_strip_range(&strips, 610, 50, 1), 2..4);
        assert_eq!(visible_strip_range(&strips, 0, 50, 1), 0..2);
    }
}
