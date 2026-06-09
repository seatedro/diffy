use super::layout::{
    editor_bottom_padding_px, render_cols_for_width, visible_segment_range_for_block,
};
use super::paint::{RowTone, build_wrapped_rich_text, wrapped_byte_slice};
use super::{CachedTextLayout, EditorDocument, EditorElement};
use crate::core::compare::LayoutMode;
use crate::editor::diff::render_doc::{ByteRange, RenderDoc, RenderLine, RenderRowKind, RunRange};
use crate::editor::diff::state::{
    EditorState, ViewportTextPoint, ViewportTextSelection, ViewportTextSide,
};
use crate::render::{FontStyle, FontWeight, Rect, TextMetrics};
use crate::ui::theme::Theme;

#[test]
fn wrapped_byte_slice_breaks_monospaced_text_by_columns() {
    let layout = CachedTextLayout::new("abcdefghij");
    assert_eq!(wrapped_byte_slice(&layout, 4, 0), Some((0, 4)));
    assert_eq!(wrapped_byte_slice(&layout, 4, 1), Some((4, 8)));
    assert_eq!(wrapped_byte_slice(&layout, 4, 2), Some((8, 10)));
    assert_eq!(wrapped_byte_slice(&layout, 4, 3), None);
}

#[test]
fn cached_text_layout_tracks_visual_columns_for_tabs() {
    let layout = CachedTextLayout::new("\ta\t");
    assert_eq!(layout.total_cols(), 16);
    assert_eq!(layout.col_for_byte(0), 0);
    assert_eq!(layout.col_for_byte(1), 8);
    assert_eq!(layout.col_for_byte(2), 9);
    assert_eq!(layout.col_for_byte(3), 16);
}

#[test]
fn rich_text_builder_returns_spans_for_requested_segment() {
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"keyword // comment".to_vec(),
        style_runs: vec![
            crate::editor::diff::render_doc::StyleRun {
                byte_start: 0,
                byte_len: 7,
                style_id: 1,
                flags: 0,
            },
            crate::editor::diff::render_doc::StyleRun {
                byte_start: 7,
                byte_len: 1,
                style_id: 0,
                flags: 0,
            },
            crate::editor::diff::render_doc::StyleRun {
                byte_start: 8,
                byte_len: 10,
                style_id: 3,
                flags: 0,
            },
        ],
        lines: vec![RenderLine {
            kind: RenderRowKind::Context as u8,
            right_text: ByteRange { start: 0, len: 18 },
            right_runs: RunRange { start: 0, len: 3 },
            right_cols: 18,
            ..RenderLine::default()
        }],
    };

    let text_layout = CachedTextLayout::new("keyword // comment");
    let spans = build_wrapped_rich_text(
        &doc,
        &text_layout,
        doc.lines[0].right_text,
        doc.lines[0].right_runs,
        0,
        u16::MAX,
        RowTone::Neutral,
        &Theme::default_dark(),
    )
    .expect("spans");

    assert!(!spans.is_empty());
    assert_eq!(
        spans
            .iter()
            .map(|span| span.text.as_ref())
            .collect::<String>(),
        "keyword // comment"
    );
    assert_eq!(spans[0].text.as_ref(), "keyword");
    assert_eq!(spans[0].font_weight, Some(FontWeight::Semibold));
    let comment = spans
        .iter()
        .find(|span| span.text.as_ref() == "// comment")
        .expect("comment span");
    assert_eq!(comment.font_style, Some(FontStyle::Italic));
}

#[test]
fn rich_text_builder_expands_tabs_across_wrapped_segments() {
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"\tabc".to_vec(),
        style_runs: vec![crate::editor::diff::render_doc::StyleRun {
            byte_start: 0,
            byte_len: 4,
            style_id: 0,
            flags: 0,
        }],
        lines: vec![RenderLine {
            kind: RenderRowKind::Context as u8,
            right_text: ByteRange { start: 0, len: 4 },
            right_runs: RunRange { start: 0, len: 1 },
            right_cols: 11,
            ..RenderLine::default()
        }],
    };

    let text_layout = CachedTextLayout::new("\tabc");
    let theme = Theme::default_dark();

    let seg0 = build_wrapped_rich_text(
        &doc,
        &text_layout,
        doc.lines[0].right_text,
        doc.lines[0].right_runs,
        0,
        4,
        RowTone::Neutral,
        &theme,
    )
    .expect("segment 0");
    let seg1 = build_wrapped_rich_text(
        &doc,
        &text_layout,
        doc.lines[0].right_text,
        doc.lines[0].right_runs,
        1,
        4,
        RowTone::Neutral,
        &theme,
    )
    .expect("segment 1");
    let seg2 = build_wrapped_rich_text(
        &doc,
        &text_layout,
        doc.lines[0].right_text,
        doc.lines[0].right_runs,
        2,
        4,
        RowTone::Neutral,
        &theme,
    )
    .expect("segment 2");

    assert_eq!(
        seg0.iter()
            .map(|span| span.text.as_ref())
            .collect::<String>(),
        "    "
    );
    assert_eq!(
        seg1.iter()
            .map(|span| span.text.as_ref())
            .collect::<String>(),
        "    "
    );
    assert_eq!(
        seg2.iter()
            .map(|span| span.text.as_ref())
            .collect::<String>(),
        "abc"
    );
}

#[test]
fn render_cols_cap_unwrapped_rows_to_viewport_budget() {
    assert_eq!(render_cols_for_width(false, 0, 8.0, 80.0), 26);
    assert_eq!(render_cols_for_width(true, 0, 8.0, 80.0), 10);
}

#[test]
fn visible_segment_range_limits_wrapped_blocks_to_viewport() {
    assert_eq!(
        visible_segment_range_for_block(100.0, 10, 20.0, 120.0, 170.0),
        1..4
    );
    assert_eq!(
        visible_segment_range_for_block(100.0, 10, 20.0, 0.0, 50.0),
        0..0
    );
}

#[test]
fn prepare_populates_visible_range_and_hit_testing() {
    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"demo.txt@@ -1 +1 @@line".to_vec(),
        style_runs: Vec::new(),
        lines: vec![
            RenderLine {
                kind: RenderRowKind::FileHeader as u8,
                left_text: ByteRange { start: 0, len: 8 },
                left_cols: 8,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::HunkSeparator as u8,
                left_text: ByteRange { start: 8, len: 11 },
                left_cols: 11,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 1,
                new_line_no: 1,
                right_text: ByteRange { start: 19, len: 4 },
                right_cols: 4,
                ..RenderLine::default()
            },
        ],
    };

    let mut runtime = EditorElement::default();
    runtime.prepare(
        &mut state,
        EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        TextMetrics::default(),
    );

    assert_eq!(state.visible_row_start, Some(0));
    // FileHeader lines are skipped in layout, so only 2 display rows exist.
    assert!(state.visible_row_end.expect("visible end") >= 2);
    let body = runtime.body_bounds();
    assert_eq!(
        runtime.hit_test_row(&state, body.x + 20.0, body.y + 5.0),
        Some(0)
    );
}

#[test]
fn prepare_adds_bottom_padding_to_keep_last_row_above_viewport_clip() {
    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"last".to_vec(),
        style_runs: Vec::new(),
        lines: vec![RenderLine {
            kind: RenderRowKind::Context as u8,
            old_line_no: 1,
            new_line_no: 1,
            right_text: ByteRange { start: 0, len: 4 },
            right_cols: 4,
            ..RenderLine::default()
        }],
    };
    let mut runtime = EditorElement::default();
    runtime.prepare(
        &mut state,
        EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 16.0,
        },
        TextMetrics::default(),
    );

    let bottom_padding = editor_bottom_padding_px(runtime.metrics);
    assert_eq!(
        state.content_height_px,
        runtime.summary.content_height_px + bottom_padding
    );
    let unpadded_max = runtime
        .summary
        .content_height_px
        .saturating_sub(state.viewport_height_px.max(1));
    assert_eq!(state.max_scroll_top_px(), unpadded_max + bottom_padding);
}

#[test]
fn preprepare_content_height_matches_prepared_viewport_height() {
    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"last".to_vec(),
        style_runs: Vec::new(),
        lines: vec![RenderLine {
            kind: RenderRowKind::Context as u8,
            old_line_no: 1,
            new_line_no: 1,
            right_text: ByteRange { start: 0, len: 4 },
            right_cols: 4,
            ..RenderLine::default()
        }],
    };
    let mut runtime = EditorElement::default();
    let bounds = Rect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    };
    let text_metrics = TextMetrics::default();
    let expected_height = runtime
        .content_height_for_bounds(bounds, text_metrics)
        .max(0.0)
        .round() as u32;

    runtime.prepare(
        &mut state,
        EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        },
        bounds,
        text_metrics,
    );

    assert_eq!(state.viewport_height_px, expected_height);
}

#[test]
fn hit_test_text_point_maps_viewport_columns_to_line_bytes() {
    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"hello".to_vec(),
        style_runs: Vec::new(),
        lines: vec![RenderLine {
            kind: RenderRowKind::Context as u8,
            old_line_no: 1,
            new_line_no: 1,
            right_text: ByteRange { start: 0, len: 5 },
            right_cols: 5,
            ..RenderLine::default()
        }],
    };
    let mut runtime = EditorElement::default();
    runtime.prepare(
        &mut state,
        EditorDocument::Text {
            compare_generation: 1,
            file_index: 0,
            path: "demo.txt",
            doc: &doc,
            show_file_headers: false,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        TextMetrics::default(),
    );

    let x = runtime.layout.unified_text_rect.x + TextMetrics::default().mono_char_width_px * 3.1;
    let y = runtime.body_bounds().y + runtime.layout.line_height * 0.5;
    let point = runtime
        .hit_test_text_point(&state, &doc, x, y)
        .expect("text point");

    assert_eq!(
        point,
        ViewportTextPoint {
            line_index: 0,
            side: ViewportTextSide::Right,
            byte_offset: 3,
        }
    );
}

#[test]
fn viewport_selection_text_copies_visible_line_segments() {
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"alphaBRAVO".to_vec(),
        style_runs: Vec::new(),
        lines: vec![
            RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 1,
                new_line_no: 1,
                right_text: ByteRange { start: 0, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 2,
                new_line_no: 2,
                right_text: ByteRange { start: 5, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            },
        ],
    };
    let selection = ViewportTextSelection {
        generation: 7,
        anchor: ViewportTextPoint {
            line_index: 0,
            side: ViewportTextSide::Right,
            byte_offset: 1,
        },
        focus: ViewportTextPoint {
            line_index: 1,
            side: ViewportTextSide::Right,
            byte_offset: 3,
        },
    };
    let runtime = EditorElement::default();

    assert_eq!(
        runtime.viewport_selection_text(&doc, &selection).as_deref(),
        Some("lpha\nBRA")
    );
}

#[test]
fn split_viewport_selection_text_stays_on_selected_side() {
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"old-aNEW-Aold-bNEW-B".to_vec(),
        style_runs: Vec::new(),
        lines: vec![
            RenderLine {
                kind: RenderRowKind::Modified as u8,
                old_line_no: 1,
                new_line_no: 1,
                left_text: ByteRange { start: 0, len: 5 },
                right_text: ByteRange { start: 5, len: 5 },
                left_cols: 5,
                right_cols: 5,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::Modified as u8,
                old_line_no: 2,
                new_line_no: 2,
                left_text: ByteRange { start: 10, len: 5 },
                right_text: ByteRange { start: 15, len: 5 },
                left_cols: 5,
                right_cols: 5,
                ..RenderLine::default()
            },
        ],
    };
    let selection = ViewportTextSelection {
        generation: 7,
        anchor: ViewportTextPoint {
            line_index: 0,
            side: ViewportTextSide::Left,
            byte_offset: 1,
        },
        focus: ViewportTextPoint {
            line_index: 1,
            side: ViewportTextSide::Left,
            byte_offset: 4,
        },
    };
    let mut runtime = EditorElement::default();
    runtime.layout.split_mode = true;

    assert_eq!(
        runtime.viewport_selection_text(&doc, &selection).as_deref(),
        Some("ld-a\nold-")
    );
}

#[test]
fn viewport_text_selection_paints_square_rectangles() {
    use crate::render::{Primitive, Scene};

    let mut state = EditorState {
        layout: LayoutMode::Unified,
        text_selection: Some(ViewportTextSelection {
            generation: 1,
            anchor: ViewportTextPoint {
                line_index: 0,
                side: ViewportTextSide::Right,
                byte_offset: 1,
            },
            focus: ViewportTextPoint {
                line_index: 1,
                side: ViewportTextSide::Right,
                byte_offset: 4,
            },
        }),
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"alphabravo".to_vec(),
        style_runs: Vec::new(),
        lines: vec![
            RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 1,
                new_line_no: 1,
                right_text: ByteRange { start: 0, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            },
            RenderLine {
                kind: RenderRowKind::Context as u8,
                old_line_no: 2,
                new_line_no: 2,
                right_text: ByteRange { start: 5, len: 5 },
                right_cols: 5,
                ..RenderLine::default()
            },
        ],
    };
    let mut runtime = EditorElement::default();
    let document = EditorDocument::Text {
        compare_generation: 1,
        file_index: 0,
        path: "demo.txt",
        doc: &doc,
        show_file_headers: false,
    };
    runtime.prepare(
        &mut state,
        document,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        TextMetrics::default(),
    );

    let theme = Theme::default_dark();
    let selection_bg = theme.colors.selection_bg;
    let mut scene = Scene::default();
    runtime.paint(&mut scene, &theme, &state, document);

    assert!(
        scene
            .primitives
            .iter()
            .any(|p| matches!(p, Primitive::Rect(r) if r.color == selection_bg))
    );
    assert!(
        !scene
            .primitives
            .iter()
            .any(|p| matches!(p, Primitive::RoundedRect(r) if r.color == selection_bg))
    );
}

#[test]
fn block_paint_emits_primitive_for_registered_decoration() {
    use super::super::decoration::{BlockDecoration, BlockPaintCtx, BlockPlacement};
    use crate::render::{Primitive, RectPrimitive, Scene};
    use crate::ui::theme::Color;

    #[derive(Debug)]
    struct StubBlock {
        color: Color,
    }

    impl BlockDecoration for StubBlock {
        fn height(&self, _metrics: &super::super::display_layout::DisplayLayoutMetrics) -> u16 {
            20
        }

        fn paint(&self, ctx: &mut BlockPaintCtx) {
            let _ = ctx.hovered;
            ctx.scene.rect(RectPrimitive {
                rect: ctx.row_rect,
                color: self.color,
            });
        }
    }

    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"@@ hdr @@".to_vec(),
        style_runs: Vec::new(),
        lines: vec![RenderLine {
            kind: RenderRowKind::HunkSeparator as u8,
            left_text: ByteRange { start: 0, len: 9 },
            left_cols: 9,
            ..RenderLine::default()
        }],
    };

    let marker = Color {
        r: 11,
        g: 22,
        b: 33,
        a: 255,
    };

    let mut runtime = EditorElement::default();
    runtime.blocks_mut().push(
        BlockPlacement::Above(0),
        Box::new(StubBlock { color: marker }),
    );

    let document = EditorDocument::Text {
        compare_generation: 7,
        file_index: 0,
        path: "demo.txt",
        doc: &doc,
        show_file_headers: false,
    };
    runtime.prepare(
        &mut state,
        document,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        TextMetrics::default(),
    );

    let theme = Theme::default_dark();
    let mut scene = Scene::default();
    runtime.paint(&mut scene, &theme, &state, document);

    let has_marker = scene.primitives.iter().any(|p| match p {
        Primitive::Rect(r) => r.color == marker,
        _ => false,
    });
    assert!(has_marker, "block paint() should emit its own primitives");
}

#[test]
fn hunk_separator_decoration_emits_background_rect() {
    use crate::render::{Primitive, Scene};

    let mut state = EditorState {
        layout: LayoutMode::Unified,
        ..EditorState::default()
    };
    let doc = RenderDoc {
        file_metadata: Vec::new(),
        text_bytes: b"@@ hdr @@".to_vec(),
        style_runs: Vec::new(),
        lines: vec![RenderLine {
            kind: RenderRowKind::HunkSeparator as u8,
            left_text: ByteRange { start: 0, len: 9 },
            left_cols: 9,
            ..RenderLine::default()
        }],
    };

    let mut runtime = EditorElement::default();
    let document = EditorDocument::Text {
        compare_generation: 1,
        file_index: 0,
        path: "demo.txt",
        doc: &doc,
        show_file_headers: false,
    };
    runtime.prepare(
        &mut state,
        document,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        },
        TextMetrics::default(),
    );

    let theme = Theme::default_dark();
    let mut scene = Scene::default();
    runtime.paint(&mut scene, &theme, &state, document);

    let hunk_bg = theme.colors.hunk_header_bg;
    let has_hunk_bg = scene.primitives.iter().any(|p| match p {
        Primitive::Rect(r) => r.color == hunk_bg,
        _ => false,
    });
    assert!(
        has_hunk_bg,
        "expected a rect with hunk_header_bg color to be emitted"
    );
}
