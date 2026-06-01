use super::decoration::{BlockPlacement, BlockRegistry, decoration_for_kind};
use super::render_doc::{DisplayRow, INVALID_U32, RenderDoc, RenderRowKind};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplayLayoutConfig {
    pub split_mode: bool,
    pub wrap_enabled: bool,
    pub wrap_column: u32,
    pub show_file_headers: bool,
    pub char_width_px: f64,
    pub unified_text_width_px: f64,
    pub split_text_width_px: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayLayoutMetrics {
    pub body_row_height_px: u16,
    pub file_header_height_px: u16,
    pub hunk_height_px: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayLayoutSummary {
    pub gutter_digits: u32,
    pub content_height_px: u32,
    pub max_cols: u32,
}

pub fn effective_wrap_cols(wrap_enabled: bool, wrap_cols: u16) -> u16 {
    if wrap_enabled {
        wrap_cols.max(1)
    } else {
        u16::MAX
    }
}

pub fn wrap_count(cols: u32, wrap_cols: u16) -> u16 {
    if cols == 0 {
        return 1;
    }
    let wrap_cols = u32::from(wrap_cols.max(1));
    ((cols + wrap_cols - 1) / wrap_cols).max(1) as u16
}

pub fn compute_gutter_digits(doc: &RenderDoc) -> u32 {
    doc.lines
        .iter()
        .flat_map(|line| [line.old_line_no, line.new_line_no])
        .filter(|line_no| *line_no != INVALID_U32)
        .max()
        .map(|line_no| line_no.to_string().len() as u32)
        .unwrap_or(3)
        .max(3)
}

pub fn rebuild_display_rows(
    doc: &RenderDoc,
    config: DisplayLayoutConfig,
    metrics: DisplayLayoutMetrics,
    blocks: &BlockRegistry,
    out: &mut Vec<DisplayRow>,
) -> DisplayLayoutSummary {
    out.clear();
    out.reserve(doc.lines.len() + blocks.len());

    let gutter_digits = compute_gutter_digits(doc);
    let unified_wrap_cols = wrap_cols_for_width(
        config.wrap_enabled,
        config.wrap_column,
        config.char_width_px,
        config.unified_text_width_px,
    );
    let split_wrap_cols = wrap_cols_for_width(
        config.wrap_enabled,
        config.wrap_column,
        config.char_width_px,
        config.split_text_width_px,
    );

    let mut y_px = 0_u32;
    let mut max_cols = 0_u32;

    for (line_index, line) in doc.lines.iter().enumerate() {
        let kind = line.row_kind();
        if kind == RenderRowKind::FileHeader && !config.show_file_headers {
            continue;
        }

        let anchor = line_index as u32;
        y_px = push_blocks(
            blocks,
            BlockPlacement::Above(anchor),
            metrics,
            anchor,
            y_px,
            out,
        );
        let (wrap_left, wrap_right, h_px) = if kind == RenderRowKind::FileHeader {
            (1_u16, 1_u16, metrics.file_header_height_px)
        } else if let Some(deco) = decoration_for_kind(kind) {
            (1_u16, 1_u16, deco.height(&metrics))
        } else if config.split_mode {
            let left_cols = split_wrap_cols.max(1);
            let right_cols = split_wrap_cols.max(1);
            let wrap_left = if line.left_text.is_valid() {
                wrap_count(line.left_cols, left_cols)
            } else {
                1
            };
            let wrap_right = if line.right_text.is_valid() {
                wrap_count(line.right_cols, right_cols)
            } else {
                1
            };
            (
                wrap_left,
                wrap_right,
                metrics
                    .body_row_height_px
                    .saturating_mul(wrap_left.max(wrap_right).max(1)),
            )
        } else if kind == RenderRowKind::Modified {
            let wrap_left = if line.left_text.is_valid() {
                wrap_count(line.left_cols, unified_wrap_cols.max(1))
            } else {
                1
            };
            let wrap_right = if line.right_text.is_valid() {
                wrap_count(line.right_cols, unified_wrap_cols.max(1))
            } else {
                1
            };
            (
                wrap_left,
                wrap_right,
                metrics
                    .body_row_height_px
                    .saturating_mul(wrap_left.max(1).saturating_add(wrap_right.max(1))),
            )
        } else {
            let wrap = wrap_count(line.primary_cols(), unified_wrap_cols.max(1));
            (
                wrap,
                wrap,
                metrics.body_row_height_px.saturating_mul(wrap.max(1)),
            )
        };

        max_cols = max_cols.max(line.left_cols.max(line.right_cols));
        out.push(DisplayRow {
            line_index: anchor,
            y_px,
            h_px,
            wrap_left,
            wrap_right,
            kind: line.kind,
            block_index: 0,
        });
        y_px = y_px.saturating_add(u32::from(h_px));

        y_px = push_blocks(
            blocks,
            BlockPlacement::Below(anchor),
            metrics,
            anchor,
            y_px,
            out,
        );
    }

    DisplayLayoutSummary {
        gutter_digits,
        content_height_px: y_px,
        max_cols,
    }
}

fn push_blocks(
    blocks: &BlockRegistry,
    placement: BlockPlacement,
    metrics: DisplayLayoutMetrics,
    anchor_line: u32,
    mut y_px: u32,
    out: &mut Vec<DisplayRow>,
) -> u32 {
    for block_index in blocks.indices_at(placement) {
        let h_px = blocks
            .get(block_index as usize)
            .map(|deco| deco.height(&metrics))
            .unwrap_or(0);
        out.push(DisplayRow {
            line_index: anchor_line,
            y_px,
            h_px,
            wrap_left: 1,
            wrap_right: 1,
            kind: RenderRowKind::Block as u8,
            block_index,
        });
        y_px = y_px.saturating_add(u32::from(h_px));
    }
    y_px
}

fn wrap_cols_for_width(
    wrap_enabled: bool,
    wrap_column: u32,
    char_width_px: f64,
    width_px: f64,
) -> u16 {
    if !wrap_enabled {
        return effective_wrap_cols(false, 0);
    }
    let mut cols = (width_px / char_width_px.max(1.0)).floor() as u32;
    if wrap_column > 0 {
        cols = cols.min(wrap_column);
    }
    effective_wrap_cols(true, cols.max(1) as u16)
}

#[cfg(test)]
mod tests {
    use super::{
        DisplayLayoutConfig, DisplayLayoutMetrics, compute_gutter_digits, effective_wrap_cols,
        rebuild_display_rows, wrap_count,
    };
    use crate::editor::diff::decoration::{
        BlockDecoration, BlockPaintCtx, BlockPlacement, BlockRegistry,
    };
    use crate::editor::diff::render_doc::{
        ByteRange, INVALID_U32, RenderDoc, RenderLine, RenderRowKind,
    };

    #[derive(Debug)]
    struct FixedHeightBlock(u16);

    impl BlockDecoration for FixedHeightBlock {
        fn height(&self, _metrics: &DisplayLayoutMetrics) -> u16 {
            self.0
        }

        fn paint(&self, _ctx: &mut BlockPaintCtx) {}
    }

    fn valid_range() -> ByteRange {
        ByteRange { start: 0, len: 1 }
    }

    #[test]
    fn wrap_count_stays_one_when_disabled_or_empty() {
        assert_eq!(wrap_count(0, 10), 1);
        assert_eq!(wrap_count(5, 10), 1);
        assert_eq!(wrap_count(11, 10), 2);
    }

    #[test]
    fn no_wrap_mode_uses_effectively_infinite_wrap_width() {
        assert_eq!(effective_wrap_cols(false, 1), u16::MAX);
        assert_eq!(wrap_count(15, effective_wrap_cols(false, 1)), 1);
        assert_eq!(effective_wrap_cols(true, 12), 12);
    }

    #[test]
    fn gutter_digits_track_largest_visible_line_number() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    old_line_no: 99,
                    new_line_no: 101,
                    left_text: ByteRange::invalid(),
                    right_text: ByteRange::invalid(),
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Added as u8,
                    old_line_no: INVALID_U32,
                    new_line_no: 1234,
                    left_text: ByteRange::invalid(),
                    right_text: ByteRange::invalid(),
                    ..RenderLine::default()
                },
            ],
        };

        assert_eq!(compute_gutter_digits(&doc), 4);
    }

    #[test]
    fn no_wrap_mode_keeps_body_rows_single_height() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                left_cols: 120,
                right_cols: 120,
                left_text: valid_range(),
                right_text: valid_range(),
                ..RenderLine::default()
            }],
        };
        let mut rows = Vec::new();

        let summary = rebuild_display_rows(
            &doc,
            DisplayLayoutConfig {
                split_mode: false,
                wrap_enabled: false,
                wrap_column: 0,
                show_file_headers: false,
                char_width_px: 8.0,
                unified_text_width_px: 100.0,
                split_text_width_px: 50.0,
            },
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &BlockRegistry::new(),
            &mut rows,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].wrap_left, 1);
        assert_eq!(rows[0].wrap_right, 1);
        assert_eq!(rows[0].h_px, 20);
        assert_eq!(summary.content_height_px, 20);
    }

    #[test]
    fn split_layout_uses_taller_side_wrap_height() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Modified as u8,
                left_cols: 20,
                right_cols: 80,
                left_text: valid_range(),
                right_text: valid_range(),
                ..RenderLine::default()
            }],
        };
        let mut rows = Vec::new();

        rebuild_display_rows(
            &doc,
            DisplayLayoutConfig {
                split_mode: true,
                wrap_enabled: true,
                wrap_column: 0,
                show_file_headers: false,
                char_width_px: 1.0,
                unified_text_width_px: 100.0,
                split_text_width_px: 10.0,
            },
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &BlockRegistry::new(),
            &mut rows,
        );

        assert_eq!(rows[0].wrap_left, 2);
        assert_eq!(rows[0].wrap_right, 8);
        assert_eq!(rows[0].h_px, 160);
    }

    #[test]
    fn unified_modified_rows_stack_removed_and_added_wrap_heights() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Modified as u8,
                left_cols: 20,
                right_cols: 30,
                left_text: valid_range(),
                right_text: valid_range(),
                ..RenderLine::default()
            }],
        };
        let mut rows = Vec::new();

        rebuild_display_rows(
            &doc,
            DisplayLayoutConfig {
                split_mode: false,
                wrap_enabled: true,
                wrap_column: 0,
                show_file_headers: false,
                char_width_px: 1.0,
                unified_text_width_px: 10.0,
                split_text_width_px: 10.0,
            },
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &BlockRegistry::new(),
            &mut rows,
        );

        assert_eq!(rows[0].wrap_left, 2);
        assert_eq!(rows[0].wrap_right, 3);
        assert_eq!(rows[0].h_px, 100);
    }

    #[test]
    fn row_positions_stay_contiguous() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::FileHeader as u8,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    left_cols: 12,
                    right_cols: 12,
                    left_text: valid_range(),
                    right_text: valid_range(),
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::HunkSeparator as u8,
                    ..RenderLine::default()
                },
            ],
        };
        let mut rows = Vec::new();

        let summary = rebuild_display_rows(
            &doc,
            DisplayLayoutConfig {
                split_mode: false,
                wrap_enabled: true,
                wrap_column: 0,
                show_file_headers: false,
                char_width_px: 8.0,
                unified_text_width_px: 96.0,
                split_text_width_px: 48.0,
            },
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &BlockRegistry::new(),
            &mut rows,
        );

        // FileHeader lines are skipped, so only 2 rows are produced.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].y_px, 0);
        assert_eq!(rows[1].y_px, u32::from(rows[0].h_px));
        assert_eq!(summary.content_height_px, rows[1].bottom_px());
    }

    #[test]
    fn block_registry_injects_rows_above_and_below_anchor() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![
                RenderLine {
                    kind: RenderRowKind::HunkSeparator as u8,
                    ..RenderLine::default()
                },
                RenderLine {
                    kind: RenderRowKind::Context as u8,
                    left_cols: 4,
                    right_cols: 4,
                    left_text: valid_range(),
                    right_text: valid_range(),
                    ..RenderLine::default()
                },
            ],
        };
        let mut blocks = BlockRegistry::new();
        blocks.push(BlockPlacement::Above(1), Box::new(FixedHeightBlock(18)));
        blocks.push(BlockPlacement::Below(1), Box::new(FixedHeightBlock(9)));

        let mut rows = Vec::new();
        let summary = rebuild_display_rows(
            &doc,
            DisplayLayoutConfig {
                split_mode: false,
                wrap_enabled: false,
                wrap_column: 0,
                show_file_headers: false,
                char_width_px: 8.0,
                unified_text_width_px: 96.0,
                split_text_width_px: 48.0,
            },
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &blocks,
            &mut rows,
        );

        // hunk (24) + block-above (18) + context (20) + block-below (9) = 4 rows
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].kind, RenderRowKind::HunkSeparator as u8);
        assert_eq!(rows[1].kind, RenderRowKind::Block as u8);
        assert_eq!(rows[1].block_index, 0);
        assert_eq!(rows[1].h_px, 18);
        assert_eq!(rows[2].kind, RenderRowKind::Context as u8);
        assert_eq!(rows[3].kind, RenderRowKind::Block as u8);
        assert_eq!(rows[3].block_index, 1);
        assert_eq!(rows[3].h_px, 9);
        assert_eq!(rows[1].y_px, 24);
        assert_eq!(rows[2].y_px, 42);
        assert_eq!(rows[3].y_px, 62);
        assert_eq!(summary.content_height_px, 71);
    }

    #[test]
    fn block_registry_preserves_order_for_multiple_blocks_at_same_anchor() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::HunkSeparator as u8,
                ..RenderLine::default()
            }],
        };
        let mut blocks = BlockRegistry::new();
        blocks.push(BlockPlacement::Above(0), Box::new(FixedHeightBlock(5)));
        blocks.push(BlockPlacement::Above(0), Box::new(FixedHeightBlock(7)));

        let mut rows = Vec::new();
        rebuild_display_rows(
            &doc,
            DisplayLayoutConfig::default(),
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &blocks,
            &mut rows,
        );

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].block_index, 0);
        assert_eq!(rows[0].h_px, 5);
        assert_eq!(rows[1].block_index, 1);
        assert_eq!(rows[1].h_px, 7);
        assert_eq!(rows[2].kind, RenderRowKind::HunkSeparator as u8);
        assert_eq!(rows[2].y_px, 12);
    }

    #[test]
    fn block_registry_is_inert_when_empty() {
        let doc = RenderDoc {
            file_metadata: Vec::new(),
            text_bytes: Vec::new(),
            style_runs: Vec::new(),
            lines: vec![RenderLine {
                kind: RenderRowKind::Context as u8,
                left_cols: 4,
                right_cols: 4,
                left_text: valid_range(),
                right_text: valid_range(),
                ..RenderLine::default()
            }],
        };
        let mut rows = Vec::new();
        rebuild_display_rows(
            &doc,
            DisplayLayoutConfig::default(),
            DisplayLayoutMetrics {
                body_row_height_px: 20,
                file_header_height_px: 32,
                hunk_height_px: 24,
            },
            &BlockRegistry::new(),
            &mut rows,
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, RenderRowKind::Context as u8);
    }
}
