use std::cell::Cell;
use std::rc::Rc;

use crate::render::{Rect, Scene, TextMetrics};
use crate::ui::components::ToastStack;
use crate::ui::design::{Bp, Rad, Sp, Sz};
use crate::ui::editor::element::{EditorDocument, EditorElement};
use crate::ui::element::*;
use crate::ui::overlays;
use crate::ui::sidebar as sidebar_mod;
use crate::ui::state::{AppState, WorkspaceMode};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;
use crate::ui::title_bar;
use crate::ui::toolbar as toolbar_mod;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeCol,
}

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: Scene,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    pub file_list_rect: Option<Rect>,
    pub sidebar_resize_handle_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
}

pub fn build_ui_frame(
    state: &mut AppState,
    theme: &Theme,
    editor: &mut EditorElement,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
    cx: &mut ElementContext,
) -> UiFrame {
    let viewport_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let file_list_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let sidebar_resize_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let ui_scale = theme.metrics.ui_scale();

    let sidebar_list_height =
        (height - theme.metrics.title_bar_height - theme.metrics.status_bar_height - Sz::SIDEBAR_LIST_OFFSET * ui_scale).max(0.0);
    state.file_list.row_height = (Sz::ROW * ui_scale).round();
    state.file_list.gap = (Sp::XS * ui_scale).round();
    let overlay_row_height = (Sz::ROW * ui_scale).round().max(24.0) as u32;
    let overlay_gap = (Sp::XS * ui_scale).round() as u32;
    state.overlays.picker.list.row_height_px = overlay_row_height;
    state.overlays.picker.list.gap_px = overlay_gap;
    state.overlays.picker.list.viewport_height_px =
        state.overlays.picker.list.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, state.overlays.picker.entries.len());
    state.overlays.command_palette.list.row_height_px = overlay_row_height;
    state.overlays.command_palette.list.gap_px = overlay_gap;
    state.overlays.command_palette.list.viewport_height_px =
        state.overlays.command_palette.list.viewport_for_max_rows(Sz::PICKER_MAX_ROWS, state.overlays.command_palette.entries.len());
    state.file_list.viewport_height = sidebar_list_height;
    state.file_list.clamp_scroll(state.workspace.files.len());
    let sidebar_width_factor = cx
        .ui_signals
        .map(|s| cx.read(s.sidebar_width_factor))
        .unwrap_or(1.0);
    let sidebar_width = sidebar_mod::preferred_sidebar_width(state, theme, cx, width) * sidebar_width_factor;

    let mut root = div()
        .w(width)
        .h(height)
        .flex_col()
        .bg(theme.colors.background)
        .child(title_bar::title_bar(state, theme, sidebar_width_factor))
        .child(
            div()
                .flex_row()
                .flex_1()
                .min_h(0.0)
                .when(state.workspace_mode == WorkspaceMode::Ready && sidebar_width_factor > 0.001 && width >= Bp::COMPACT * ui_scale, |d| {
                    d.child(sidebar_mod::sidebar(
                        state,
                        theme,
                        sidebar_width,
                        file_list_bounds.clone(),
                        cx,
                    ))
                    .child(sidebar_mod::sidebar_resizer(theme, sidebar_resize_bounds.clone(), sidebar_width))
                })
                .child(toolbar_mod::main_surface(
                    state,
                    theme,
                    text_metrics,
                    viewport_bounds.clone(),
                )),
        )
        .child(crate::ui::status_bar::status_bar(state, theme));

    if let Some(overlay) = overlays::render_active_overlay(state, theme, width, height) {
        root = root.child(overlay);
    }

    if !state.toasts.is_empty() {
        root = root.child(ToastStack::new(&state.toasts, width, height).build());
    }

    let mut root = root.into_any();

    let mut scene = Scene::default();
    render_element(&mut root, &mut scene, cx, width, height);
    let mut scrollbar_tracks = std::mem::take(&mut cx.scrollbar_tracks);

    if state.workspace_mode == WorkspaceMode::Ready {
        if let Some(vp_bounds) = viewport_bounds.get() {
            let document = match state.workspace.active_file.as_ref() {
                Some(active_file) if active_file.file.is_binary => EditorDocument::Binary {
                    path: &active_file.path,
                },
                Some(active_file) => EditorDocument::Text {
                    compare_generation: state.workspace.compare_generation,
                    file_index: active_file.index,
                    path: &active_file.path,
                    doc: &active_file.render_doc,
                },
                None => EditorDocument::Empty,
            };
            editor.prepare(&mut state.editor, document, vp_bounds, text_metrics);
            scene.clip(vp_bounds);
            editor.paint(&mut scene, theme, &state.editor, document);
            scene.pop_clip();

            if state.editor.content_height_px > state.editor.viewport_height_px
                && state.editor.viewport_height_px > 0
            {
                let sb = editor.scrollbar_rect();
                let ratio = state.editor.viewport_height_px as f32
                    / state.editor.content_height_px as f32;
                let thumb_h = (sb.height * ratio).max(Sp::XXL * ui_scale).min(sb.height);
                let scroll_range = state.editor.max_scroll_top_px().max(1) as f32;
                let top_ratio = state.editor.scroll_top_px as f32 / scroll_range;
                let thumb_y = sb.y + (sb.height - thumb_h) * top_ratio;
                scrollbar_tracks.push(ScrollbarTrack {
                    track_rect: Rect {
                        x: sb.x - Rad::LG * ui_scale,
                        y: sb.y,
                        width: sb.width + Sp::MD * ui_scale,
                        height: sb.height,
                    },
                    thumb_top: thumb_y,
                    thumb_height: thumb_h,
                    content_height: state.editor.content_height_px as f32,
                    viewport_height: state.editor.viewport_height_px as f32,
                    action_builder: ScrollActionBuilder::ViewportLines,
                });
            }
        }
    }

    let hits = std::mem::take(&mut cx.hits);
    let scroll_regions = std::mem::take(&mut cx.scroll_regions);
    let text_input_hit_areas = std::mem::take(&mut cx.text_input_hit_areas);
    let file_list_rect = scroll_regions.iter().find_map(|region| {
        matches!(region.action_builder, ScrollActionBuilder::FileList).then_some(region.bounds)
    });

    UiFrame {
        scene,
        hits,
        scroll_regions,
        text_input_hit_areas,
        scrollbar_tracks,
        file_list_rect: file_list_rect.or_else(|| file_list_bounds.get()),
        sidebar_resize_handle_rect: sidebar_resize_bounds.get(),
        viewport_rect: viewport_bounds.get(),
    }
}
