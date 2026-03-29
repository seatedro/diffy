use std::cell::Cell;
use std::rc::Rc;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::render::{Rect, Scene, TextMetrics};
use crate::ui::actions::Action;
use crate::ui::design::Sp;
use crate::ui::diff_viewport::runtime::{DiffViewportRuntime, ViewportDocument};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::state::{
    AppState, AsyncStatus, CompareField, FocusTarget, OverlaySurface, PickerItem, ToastKind,
    WorkspaceMode,
};
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};

// ---------------------------------------------------------------------------
// Public frame types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Default,
    Pointer,
    Text,
}

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: Scene,
    pub hits: Vec<HitRegion>,
    pub scroll_regions: Vec<ScrollRegion>,
    pub text_input_hit_areas: Vec<TextInputHitArea>,
    pub scrollbar_tracks: Vec<ScrollbarTrack>,
    pub file_list_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn build_ui_frame(
    state: &mut AppState,
    theme: &Theme,
    viewport_runtime: &mut DiffViewportRuntime,
    text_metrics: TextMetrics,
    width: f32,
    height: f32,
    cx: &mut ElementContext,
) -> UiFrame {
    let viewport_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
    let file_list_bounds: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));

    // Estimate the file list viewport height for scroll clamping.
    // Layout: title_bar + [sidebar | main] + status_bar.  Within sidebar: header (~40px) + list.
    let sidebar_list_height = (height
        - theme.metrics.title_bar_height
        - theme.metrics.status_bar_height
        - 40.0)
        .max(0.0);
    state.file_list.viewport_height = sidebar_list_height;
    state.file_list.clamp_scroll(state.workspace.files.len());

    let mut root = div()
        .w(width)
        .h(height)
        .flex_col()
        .bg(theme.colors.background)
        .child(title_bar(state, theme))
        .child(
            div()
                .flex_row()
                .flex_1()
                .child(sidebar(state, theme, file_list_bounds.clone()))
                .child(main_surface(state, theme, text_metrics, viewport_bounds.clone())),
        )
        .child(status_bar(state, theme));

    // --- Overlay (z-indexed above everything) ---
    if let Some(top) = state.overlays.stack.last().cloned() {
        let overlay = match top.surface {
            OverlaySurface::CompareSheet => compare_sheet(state, theme, width, height),
            OverlaySurface::RepoPicker => repo_picker(state, theme, width, height),
            OverlaySurface::RefPicker(field) => ref_picker(state, theme, field, width, height),
            OverlaySurface::CommandPalette => command_palette(state, theme, width, height),
            OverlaySurface::PullRequestModal => pull_request_modal(state, theme, width, height),
            OverlaySurface::GitHubAuthModal => auth_modal(state, theme, width, height),
        };
        root = root.child(overlay);
    }

    let mut root = root.into_any();

    // --- Render element tree ---
    let mut scene = Scene::default();
    render_element(&mut root, &mut scene, cx, width, height);
    let mut scrollbar_tracks = std::mem::take(&mut cx.scrollbar_tracks);

    // --- Viewport content (painted after element tree, clipped to bounds) ---
    if state.workspace_mode == WorkspaceMode::Ready {
        if let Some(vp_bounds) = viewport_bounds.get() {
            let document = match state.workspace.active_file.as_ref() {
                Some(active_file) if active_file.file.is_binary => ViewportDocument::Binary {
                    path: &active_file.path,
                },
                Some(active_file) => ViewportDocument::Text {
                    compare_generation: state.workspace.compare_generation,
                    file_index: active_file.index,
                    path: &active_file.path,
                    doc: &active_file.render_doc,
                },
                None => ViewportDocument::Empty,
            };
            viewport_runtime.prepare(
                &mut state.viewport,
                document,
                vp_bounds,
                text_metrics,
            );
            scene.clip(vp_bounds);
            viewport_runtime.paint(&mut scene, theme, &state.viewport, document);
            scene.pop_clip();

            // Register viewport scrollbar for drag support
            if state.viewport.content_height_px > state.viewport.viewport_height_px
                && state.viewport.viewport_height_px > 0
            {
                let sb = viewport_runtime.scrollbar_rect();
                let ratio = state.viewport.viewport_height_px as f32
                    / state.viewport.content_height_px as f32;
                let thumb_h = (sb.height * ratio).max(32.0).min(sb.height);
                let scroll_range = state.viewport.max_scroll_top_px().max(1) as f32;
                let top_ratio = state.viewport.scroll_top_px as f32 / scroll_range;
                let thumb_y = sb.y + (sb.height - thumb_h) * top_ratio;
                scrollbar_tracks.push(ScrollbarTrack {
                    track_rect: Rect {
                        x: sb.x - 6.0,
                        y: sb.y,
                        width: sb.width + 12.0,
                        height: sb.height,
                    },
                    thumb_top: thumb_y,
                    thumb_height: thumb_h,
                    content_height: state.viewport.content_height_px as f32,
                    viewport_height: state.viewport.viewport_height_px as f32,
                    action_builder: ScrollActionBuilder::ViewportLines,
                });
            }
        }
    }

    // --- Toasts (painted last so they appear above viewport content) ---
    if !state.toasts.is_empty() {
        let toast_width = 360.0_f32.min((width - 32.0).max(220.0));
        let toast_height = 52.0;
        let tc = ToastColors {
            surface: theme.colors.elevated_surface,
            text: theme.colors.text,
            text_muted: theme.colors.text_muted,
            error_accent: theme.colors.status_error,
            info_accent: theme.colors.status_info,
            border: theme.colors.border,
            icon_color: theme.colors.text_muted,
            font_size: theme.metrics.ui_font_size,
        };
        for (offset, (index, toast)) in state.toasts.iter().enumerate().rev().enumerate() {
            let (message, kind) = (&toast.message, toast.kind);
            let rect = Rect {
                x: width - toast_width - Sp::XL,
                y: height - 28.0 - Sp::LG - toast_height
                    - offset as f32 * (toast_height + Sp::SM),
                width: toast_width,
                height: toast_height,
            };
            paint_toast(&mut scene, cx, rect, message, kind, index, &tc);
        }
    }

    UiFrame {
        scene,
        hits: std::mem::take(&mut cx.hits),
        scroll_regions: std::mem::take(&mut cx.scroll_regions),
        text_input_hit_areas: std::mem::take(&mut cx.text_input_hit_areas),
        scrollbar_tracks,
        file_list_rect: file_list_bounds.get(),
        viewport_rect: viewport_bounds.get(),
    }
}

// ---------------------------------------------------------------------------
// Title bar
// ---------------------------------------------------------------------------

fn title_bar(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;

    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("diffy");

    // Left cluster: app icon + name
    let left = div()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(Sp::SM)
        .child(svg_icon(lucide::GIT_COMPARE, 18.0).color(tc.accent))
        .child(text(repo_label).semibold().color(tc.text_strong));

    // Center: summary when ready
    let center = if state.workspace_mode == WorkspaceMode::Ready {
        let summary = format!(
            "{} files  \u{00b7}  {} \u{2192} {}",
            state.workspace.files.len(),
            display_ref(
                state
                    .compare
                    .resolved_left
                    .as_deref()
                    .unwrap_or(&state.compare.left_ref)
            ),
            display_ref(
                state
                    .compare
                    .resolved_right
                    .as_deref()
                    .unwrap_or(&state.compare.right_ref)
            ),
        );
        div().child(text(summary).text_sm().color(tc.text_muted))
    } else if state.workspace_mode == WorkspaceMode::Loading {
        div().child(
            text("Comparing\u{2026}")
                .text_sm()
                .color(tc.text_muted),
        )
    } else {
        div()
    };

    // Right cluster: toolbar buttons
    let compare_active = state.overlays.top() == Some(OverlaySurface::CompareSheet);
    let pr_active = state.overlays.top() == Some(OverlaySurface::PullRequestModal);

    let right = div()
        .flex_row()
        .items_center()
        .gap_1()
        .child(icon_ghost_btn(
            lucide::GIT_COMPARE,
            "Compare",
            Action::OpenCompareSheet,
            compare_active,
            theme,
        ))
        .child(icon_ghost_btn(
            lucide::GIT_PULL_REQUEST,
            "PR",
            Action::OpenPullRequestModal,
            pr_active,
            theme,
        ))
        .child(toolbar_separator(tc))
        .child(segmented_control(
            &[
                (
                    "Split",
                    Action::SetLayoutMode(LayoutMode::Split),
                    state.compare.layout == LayoutMode::Split,
                ),
                (
                    "Unified",
                    Action::SetLayoutMode(LayoutMode::Unified),
                    state.compare.layout == LayoutMode::Unified,
                ),
            ],
            theme,
        ))
        .child(icon_ghost_btn(
            lucide::WRAP_TEXT,
            "Wrap",
            Action::ToggleWrap,
            state.viewport.wrap_enabled,
            theme,
        ))
        .child(
            div()
                .px_2()
                .py_1()
                .rounded_md()
                .hover_bg(tc.ghost_element_hover)
                .on_click(Action::ToggleThemeMode)
                .cursor(CursorHint::Pointer)
                .child(svg_icon(
                    if theme.mode == crate::ui::theme::ThemeMode::Dark {
                        lucide::MOON
                    } else {
                        lucide::SUN
                    },
                    15.0,
                ).color(tc.text_muted)),
        );

    div()
        .flex_row()
        .items_center()
        .h(theme.metrics.title_bar_height)
        .w_full()
        .px(Sp::XL)
        .bg(tc.title_bar_background)
        .border_b(tc.border_variant)
        .child(left)
        .child(div().px_4().child(center))
        .child(spacer())
        .child(right)
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn sidebar(state: &AppState, theme: &Theme, _bounds_cell: Rc<Cell<Option<Rect>>>) -> Div {
    let tc = &theme.colors;
    let file_count = state.workspace.files.len();

    // Header with count badge
    let header = div()
        .px_4()
        .py_3()
        .flex_row()
        .items_center()
        .child(
            text("FILES")
                .text_xs()
                .semibold()
                .color(tc.text_muted),
        )
        .optional_child(if file_count > 0 {
            Some(
                div()
                    .px(Sp::SM)
                    .child(
                        div()
                            .px(6.0)
                            .py(2.0)
                            .rounded_sm()
                            .bg(Color::rgba(255, 255, 255, 10))
                            .child(
                                text(file_count.to_string())
                                    .text_xs()
                                    .color(tc.text_muted),
                            ),
                    ),
            )
        } else {
            None
        });

    let mut sidebar = div()
        .flex_col()
        .w(theme.metrics.sidebar_width)
        .flex_shrink_0()
        .h_full()
        .bg(tc.sidebar_background)
        .border_r(tc.border_variant)
        .child(header);

    if state.workspace.files.is_empty() {
        let (icon, msg) = if state.compare.repo_path.is_some() {
            (lucide::GIT_COMPARE, "Run a compare to see changes.")
        } else {
            (lucide::FOLDER_OPEN, "Open a repository to start.")
        };
        sidebar = sidebar.child(
            div()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .child(svg_icon(icon, 20.0).color(tc.text_muted))
                        .child(text(msg).text_sm().color(tc.text_muted)),
                ),
        );
    } else {
        let file_count = state.workspace.files.len();
        let row_height = state.file_list.row_height;
        let total_height = state.file_list.total_content_height(file_count);
        let scroll_px = state.file_list.scroll_offset_px;

        let mut list = div()
            .flex_1()
            .flex_col()
            .px(6.0)
            .gap(Sp::XS)
            .clip()
            .scroll_y(scroll_px)
            .scroll_total(total_height)
            .on_scroll(ScrollActionBuilder::FileList);

        for (index, file) in state.workspace.files.iter().enumerate() {
            let selected = state.workspace.selected_file_index == Some(index);
            let icon_color = if selected {
                tc.text_accent
            } else {
                tc.text_muted
            };
            let text_color = if selected {
                tc.text_strong
            } else {
                tc.text
            };

            let mut row = div()
                .w_full()
                .h(row_height)
                .flex_row()
                .items_center()
                .px(Sp::SM)
                .gap_2()
                .on_click(Action::SelectFile(index))
                .cursor(CursorHint::Pointer);

            // Selected: left accent border + selected bg
            if selected {
                row = row.bg(tc.sidebar_row_selected).border_l(tc.accent);
            } else {
                row = row.hover_bg(tc.sidebar_row_hover);
            }

            // File icon
            row = row.child(svg_icon(lucide::FILE_CODE, 15.0).color(icon_color));

            // File path (truncated)
            row = row.child(
                div()
                    .flex_1()
                    .flex_col()
                    .gap(1.0)
                    .child(text(&file.path).text_sm().color(text_color).truncate()),
            );

            // +/- stats with semantic colors
            if file.additions > 0 || file.deletions > 0 {
                row = row.child(
                    div()
                        .flex_row()
                        .gap(Sp::XS)
                        .flex_shrink_0()
                        .child(
                            text(format!("+{}", file.additions))
                                .text_xs()
                                .color(tc.line_add_text),
                        )
                        .child(
                            text(format!("\u{2212}{}", file.deletions))
                                .text_xs()
                                .color(tc.line_del_text),
                        ),
                );
            }

            list = list.child(row);
        }

        sidebar = sidebar.child(list);
    }

    sidebar
}

// ---------------------------------------------------------------------------
// Main surface
// ---------------------------------------------------------------------------

fn main_surface(
    state: &AppState,
    theme: &Theme,
    _text_metrics: TextMetrics,
    viewport_bounds: Rc<Cell<Option<Rect>>>,
) -> Div {
    let tc = &theme.colors;
    let mut main = div()
        .flex_1()
        .flex_col()
        .h_full()
        .bg(tc.editor_surface);

    let has_overlay = state.active_overlay_name().is_some();
    match state.workspace_mode {
        WorkspaceMode::Ready => {
            let file_label = state
                .workspace
                .selected_file_path
                .as_deref()
                .unwrap_or("No file selected");

            // File header bar
            main = main.child(
                div()
                    .h(36.0)
                    .px_4()
                    .flex_row()
                    .items_center()
                    .border_b(tc.border_variant)
                    .child(svg_icon(lucide::FILE_CODE, 14.0).color(tc.text_muted))
                    .child(div().w(Sp::SM))
                    .child(
                        text(file_label)
                            .text_sm()
                            .color(tc.text_muted)
                            .truncate(),
                    ),
            );

            // Viewport canvas
            let vb = viewport_bounds.clone();
            main = main.child(
                canvas(move |bounds, _scene, _cx| {
                    vb.set(Some(bounds));
                })
                .flex_1(),
            );
        }
        WorkspaceMode::Loading => {
            main = main.child(loading_card(state, theme));
        }
        WorkspaceMode::Empty if !has_overlay => {
            main = main.child(empty_state(state, theme));
        }
        WorkspaceMode::Empty => {}
    }

    main
}

fn loading_card(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(440.0)
                .p_6()
                .flex_col()
                .gap_3()
                .items_center()
                .bg(tc.elevated_surface)
                .rounded_xl()
                .border_b(tc.border)
                .shadow(16.0, 6.0, Color::rgba(0, 0, 0, 80))
                .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 40))
                .child(svg_icon(lucide::LOADER, 24.0).color(tc.text_muted))
                .child(
                    text("Comparing repository\u{2026}")
                        .semibold()
                        .color(tc.text_strong),
                )
                .child(
                    text(format!(
                        "{} \u{2022} {} \u{2192} {}",
                        compare_mode_label(state.compare.mode),
                        display_ref(&state.compare.left_ref),
                        display_ref(&state.compare.right_ref)
                    ))
                    .text_sm()
                    .color(tc.text_muted),
                ),
        )
}

fn empty_state(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let has_repo = state.compare.repo_path.is_some();

    let (title, subtitle) = if has_repo {
        (
            "Ready to compare",
            "Use the compare sheet, PR modal, or command palette to build a diff.",
        )
    } else {
        (
            "Start a new compare",
            "Choose a repository, select refs, then open the native diff workspace.",
        )
    };

    let hero_icon = if has_repo {
        lucide::GIT_COMPARE
    } else {
        lucide::GIT_BRANCH
    };

    let mut card = div()
        .w(520.0)
        .p(Sp::XXL)
        .flex_col()
        .gap(Sp::LG)
        .bg(tc.elevated_surface)
        .rounded_xl()
        .border_b(tc.border)
        .shadow(20.0, 8.0, Color::rgba(0, 0, 0, 80))
        .shadow(4.0, 2.0, Color::rgba(0, 0, 0, 40))
        // Hero icon
        .child(svg_icon(hero_icon, 32.0).color(tc.accent))
        // Heading
        .child(text(title).text_lg().semibold().color(tc.text_strong))
        // Subtitle
        .child(text(subtitle).text_sm().color(tc.text_muted))
        // Action buttons
        .child(
            div()
                .flex_row()
                .gap_3()
                .pt(Sp::XS)
                .child(filled_icon_button(
                    lucide::PLAY,
                    "Open Compare",
                    Action::OpenCompareSheet,
                    theme,
                ))
                .child(subtle_icon_button(
                    lucide::FOLDER_OPEN,
                    "Open Folder",
                    Action::OpenRepositoryDialog,
                    theme,
                )),
        );

    // Recent repositories section
    if !state.settings.recent_repos.is_empty() {
        let mut recent_section = div()
            .pt(Sp::SM)
            .flex_col()
            .gap(Sp::XS)
            .child(
                text("Recent repositories")
                    .text_xs()
                    .semibold()
                    .color(tc.text_muted),
            );

        for repo in state.settings.recent_repos.iter().take(5) {
            let label = repo.display().to_string();
            recent_section = recent_section.child(
                div()
                    .w_full()
                    .py(Sp::XS)
                    .px_2()
                    .rounded_sm()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .hover_bg(tc.ghost_element_hover)
                    .on_click(Action::OpenRepository(repo.clone()))
                    .cursor(CursorHint::Pointer)
                    .child(svg_icon(lucide::FOLDER, 13.0).color(tc.text_muted))
                    .child(text(label).text_sm().color(tc.text).truncate()),
            );
        }

        card = card.child(recent_section);
    }

    div()
        .flex_1()
        .items_center()
        .justify_center()
        .child(card)
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn status_bar(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let (status_icon, status_color, status_text) = match state.repository.status {
        AsyncStatus::Ready => (lucide::CHECK, tc.line_add_text, "ready"),
        AsyncStatus::Loading => (lucide::LOADER, tc.text_muted, "loading"),
        AsyncStatus::Failed => (lucide::ALERT_CIRCLE, tc.status_error, "error"),
        AsyncStatus::Idle => (lucide::INFO, tc.text_muted, "idle"),
    };

    let right_text = format!(
        "{}  \u{00b7}  {}",
        compare_mode_label(state.compare.mode),
        renderer_label(state.compare.renderer),
    );

    div()
        .flex_row()
        .items_center()
        .h(theme.metrics.status_bar_height)
        .w_full()
        .px_4()
        .bg(tc.status_bar_background)
        .border_t(tc.border_variant)
        .child(svg_icon(status_icon, 12.0).color(status_color))
        .child(div().w(6.0))
        .child(text(status_text).text_xs().color(tc.text_muted))
        .child(spacer())
        .child(text(right_text).text_xs().color(tc.text_muted))
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

struct ToastColors {
    surface: Color,
    text: Color,
    text_muted: Color,
    error_accent: Color,
    info_accent: Color,
    border: Color,
    icon_color: Color,
    font_size: f32,
}

fn paint_toast(
    scene: &mut Scene,
    cx: &mut ElementContext,
    rect: Rect,
    message: &str,
    kind: ToastKind,
    index: usize,
    colors: &ToastColors,
) {
    use crate::render::{
        BorderPrimitive, FontKind, RoundedRectPrimitive, ShadowPrimitive, TextPrimitive,
    };

    let radius = 12.0;

    // Shadow
    scene.shadow(ShadowPrimitive {
        rect,
        blur_radius: 16.0,
        corner_radius: radius,
        offset: [0.0, 4.0],
        color: Color::rgba(0, 0, 0, 60),
    });
    scene.shadow(ShadowPrimitive {
        rect,
        blur_radius: 4.0,
        corner_radius: radius,
        offset: [0.0, 2.0],
        color: Color::rgba(0, 0, 0, 30),
    });

    // Background
    scene.rounded_rect(RoundedRectPrimitive::uniform(rect, radius, colors.surface));

    // Left accent stripe based on kind
    let accent = match kind {
        ToastKind::Info => colors.info_accent,
        ToastKind::Error => colors.error_accent,
    };
    let stripe = Rect {
        x: rect.x,
        y: rect.y,
        width: 3.0,
        height: rect.height,
    };
    scene.rounded_rect(RoundedRectPrimitive::uniform(stripe, radius, accent));

    // Border
    scene.border(BorderPrimitive::uniform(rect, 1.0, radius, colors.border));

    // Message text
    scene.text(TextPrimitive {
        rect: rect.pad(Sp::XL, 0.0, Sp::XL, 0.0),
        text: message.to_string(),
        color: colors.text,
        font_size: colors.font_size,
        font_kind: FontKind::Ui,
        font_weight: crate::render::FontWeight::Normal,
    });

    // Dismiss X text
    scene.text(TextPrimitive {
        rect: Rect {
            x: rect.x + rect.width - 32.0,
            y: rect.y,
            width: 20.0,
            height: rect.height,
        },
        text: "\u{00d7}".to_string(),
        color: colors.text_muted,
        font_size: colors.font_size,
        font_kind: FontKind::Ui,
        font_weight: crate::render::FontWeight::Normal,
    });

    // Hit region for dismiss
    cx.hits.push(HitRegion {
        rect,
        action: Action::DismissToast(index),
        cursor: CursorHint::Pointer,
    });
}

// ---------------------------------------------------------------------------
// Overlays
// ---------------------------------------------------------------------------

fn modal_backdrop(theme: &Theme, width: f32, height: f32) -> Div {
    div()
        .w(width)
        .h(height)
        .z_index(100)
        .bg(theme.colors.overlay_scrim)
        .on_click(Action::CloseOverlay)
        .items_center()
        .justify_center()
}

fn modal_panel(width: f32, theme: &Theme) -> Div {
    let tc = &theme.colors;
    div()
        .w(width)
        .flex_col()
        .p(Sp::XXL)
        .gap(Sp::LG)
        .bg(tc.elevated_surface)
        .rounded_xl()
        .border_b(tc.border)
        .shadow(24.0, 8.0, Color::rgba(0, 0, 0, 100))
        .shadow(8.0, 4.0, Color::rgba(0, 0, 0, 50))
        .shadow(2.0, 1.0, Color::rgba(0, 0, 0, 30))
        // Consume clicks so they don't propagate to the backdrop's CloseOverlay.
        .on_click(Action::Noop)
}

fn modal_header(title: &str, subtitle: &str, icon: &'static str, theme: &Theme) -> Div {
    let tc = &theme.colors;
    div()
        .flex_col()
        .gap(Sp::SM)
        .child(
            div()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(Sp::SM)
                .child(svg_icon(icon, 18.0).color(tc.accent))
                .child(text(title).text_lg().semibold().color(tc.text_strong)),
        )
        .child(text(subtitle).text_sm().color(tc.text_muted))
}

fn compare_sheet(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    let tc = &theme.colors;
    modal_backdrop(theme, width, height).child(
        modal_panel(560.0, theme)
            .gap(Sp::XL)
            .child(modal_header(
                "Compare Setup",
                "Pick a repository, refs, compare mode, and renderer.",
                lucide::GIT_COMPARE,
                theme,
            ))
            // Repo picker button
            .child(
                subtle_icon_button(
                    lucide::FOLDER,
                    &state
                        .compare
                        .repo_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "Choose repository\u{2026}".into()),
                    Action::OpenRepoPicker,
                    theme,
                )
                .w_full(),
            )
            // Ref fields
            .child(
                div()
                    .flex_row()
                    .gap(Sp::MD)
                    .child(
                        text_input("Left ref", &state.compare.left_ref)
                            .placeholder("main")
                            .focused(state.focus.current == Some(FocusTarget::CompareLeftRef))
                            .on_click(Action::SetFocus(Some(FocusTarget::CompareLeftRef)))
                            .cursor(state.text_edit.cursor)
                            .anchor(state.text_edit.anchor)
                            .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                            .focus_target(FocusTarget::CompareLeftRef)
                            .w_full()
                            .h(64.0)
                            .flex_1(),
                    )
                    .child(
                        text_input("Right ref", &state.compare.right_ref)
                            .placeholder("feature")
                            .focused(state.focus.current == Some(FocusTarget::CompareRightRef))
                            .on_click(Action::SetFocus(Some(FocusTarget::CompareRightRef)))
                            .cursor(state.text_edit.cursor)
                            .anchor(state.text_edit.anchor)
                            .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                            .focus_target(FocusTarget::CompareRightRef)
                            .w_full()
                            .h(64.0)
                            .flex_1(),
                    ),
            )
            // Compare mode + layout/renderer controls
            .child(
                div()
                    .flex_col()
                    .gap(Sp::MD)
                    .child(
                        div()
                            .flex_row()
                            .items_center()
                            .gap(Sp::MD)
                            .child(text("Mode").text_sm().medium().color(tc.text_muted))
                            .child(segmented_control(
                                &[
                                    (
                                        "Single",
                                        Action::SetCompareMode(CompareMode::SingleCommit),
                                        state.compare.mode == CompareMode::SingleCommit,
                                    ),
                                    (
                                        "Two Dot",
                                        Action::SetCompareMode(CompareMode::TwoDot),
                                        state.compare.mode == CompareMode::TwoDot,
                                    ),
                                    (
                                        "Three Dot",
                                        Action::SetCompareMode(CompareMode::ThreeDot),
                                        state.compare.mode == CompareMode::ThreeDot,
                                    ),
                                ],
                                theme,
                            )),
                    )
                    .child(
                        div()
                            .flex_row()
                            .gap(Sp::XL)
                            .child(
                                div()
                                    .flex_row()
                                    .items_center()
                                    .gap(Sp::MD)
                                    .child(text("Layout").text_sm().medium().color(tc.text_muted))
                                    .child(segmented_control(
                                        &[
                                            (
                                                "Unified",
                                                Action::SetLayoutMode(LayoutMode::Unified),
                                                state.compare.layout == LayoutMode::Unified,
                                            ),
                                            (
                                                "Split",
                                                Action::SetLayoutMode(LayoutMode::Split),
                                                state.compare.layout == LayoutMode::Split,
                                            ),
                                        ],
                                        theme,
                                    )),
                            )
                            .child(
                                div()
                                    .flex_row()
                                    .items_center()
                                    .gap(Sp::MD)
                                    .child(text("Engine").text_sm().medium().color(tc.text_muted))
                                    .child(segmented_control(
                                        &[
                                            (
                                                "Built-in",
                                                Action::SetRenderer(RendererKind::Builtin),
                                                state.compare.renderer == RendererKind::Builtin,
                                            ),
                                            (
                                                "Difftastic",
                                                Action::SetRenderer(RendererKind::Difftastic),
                                                state.compare.renderer == RendererKind::Difftastic,
                                            ),
                                        ],
                                        theme,
                                    )),
                            ),
                    ),
            )
            // Validation message
            .optional_child(
                state
                    .overlays
                    .compare_sheet
                    .validation_message
                    .as_deref()
                    .map(|msg| {
                        div()
                            .flex_row()
                            .flex_shrink_0()
                            .items_center()
                            .gap(Sp::SM)
                            .child(
                                svg_icon(lucide::ALERT_CIRCLE, 14.0).color(tc.status_error),
                            )
                            .child(text(msg).text_sm().color(tc.status_error))
                    }),
            )
            .child(spacer())
            // Footer
            .child(
                div()
                    .flex_row()
                    .justify_end()
                    .child(filled_icon_button(
                        lucide::PLAY,
                        if state.workspace.status == AsyncStatus::Loading {
                            "Comparing\u{2026}"
                        } else {
                            "Start Compare"
                        },
                        Action::StartCompare,
                        theme,
                    )),
            ),
    )
}

fn repo_picker(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    modal_backdrop(theme, width, height).child(
        modal_panel(680.0, theme)
            .h(420.0)
            .child(modal_header(
                "Repository Picker",
                "Search or type a path to a git repository.",
                lucide::FOLDER_OPEN,
                theme,
            ))
            .child(
                text_input("Search or type a path", &state.overlays.picker.query)
                    .placeholder("C:\\work\\repo")
                    .focused(state.focus.current == Some(FocusTarget::PickerInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::PickerInput)
                    .w_full()
                    .h(44.0),
            )
            .child(picker_list(
                &state.overlays.picker.entries,
                state.overlays.picker.selected_index,
                theme,
            ))
            .child(subtle_icon_button(
                lucide::FOLDER_OPEN,
                "Browse Folders",
                Action::OpenRepositoryDialog,
                theme,
            )),
    )
}

fn ref_picker(
    state: &AppState,
    theme: &Theme,
    field: CompareField,
    width: f32,
    height: f32,
) -> Div {
    let (title, icon) = match field {
        CompareField::Left => ("Pick Left Ref", lucide::GIT_BRANCH),
        CompareField::Right => ("Pick Right Ref", lucide::GIT_BRANCH),
    };
    let current_value = match field {
        CompareField::Left => &state.compare.left_ref,
        CompareField::Right => &state.compare.right_ref,
    };

    modal_backdrop(theme, width, height).child(
        modal_panel(480.0, theme)
            .h(380.0)
            .child(modal_header(
                title,
                "Search branches, tags, or commits.",
                icon,
                theme,
            ))
            .child(
                text_input("Filter refs", current_value)
                    .placeholder("Search branches, tags, commits")
                    .focused(state.focus.current == Some(FocusTarget::PickerInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::PickerInput)
                    .w_full()
                    .h(44.0),
            )
            .child(picker_list(
                &state.overlays.picker.entries,
                state.overlays.picker.selected_index,
                theme,
            )),
    )
}

fn command_palette(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    div()
        .w(width)
        .h(height)
        .z_index(100)
        .bg(theme.colors.overlay_scrim)
        .on_click(Action::CloseOverlay)
        .items_center()
        .child(
            div()
                .w(720.0)
                .h(420.0)
                .flex_col()
                .p(Sp::XL)
                .gap(Sp::LG)
                .bg(theme.colors.elevated_surface)
                .rounded_xl()
                .border_b(theme.colors.border)
                .shadow(24.0, 8.0, Color::rgba(0, 0, 0, 100))
                .shadow(8.0, 4.0, Color::rgba(0, 0, 0, 50))
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap(Sp::SM)
                        .child(svg_icon(lucide::COMMAND, 16.0).color(theme.colors.accent))
                        .child(
                            text("Command Palette")
                                .semibold()
                                .color(theme.colors.text_strong),
                        ),
                )
                .child(
                    text_input(
                        "Command palette",
                        &state.overlays.command_palette.query,
                    )
                    .placeholder("Type a command, file, repo, or ref")
                    .focused(
                        state.focus.current == Some(FocusTarget::CommandPaletteInput),
                    )
                    .on_click(Action::SetFocus(Some(
                        FocusTarget::CommandPaletteInput,
                    )))
                    .cursor(state.text_edit.cursor)
                    .anchor(state.text_edit.anchor)
                    .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                    .focus_target(FocusTarget::CommandPaletteInput)
                    .w_full()
                    .h(44.0),
                )
                .child(picker_list(
                    &state.overlays.command_palette.entries,
                    state.overlays.command_palette.selected_index,
                    theme,
                )),
        )
}

fn pull_request_modal(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    let tc = &theme.colors;

    let mut panel = modal_panel(640.0, theme)
        .h(340.0)
        .child(modal_header(
            "GitHub Pull Request",
            "Paste a PR URL to load its base and head refs for diffing.",
            lucide::GIT_PULL_REQUEST,
            theme,
        ))
        .child(
            text_input("Pull request URL", &state.github.pull_request.url_input)
                .placeholder("https://github.com/owner/repo/pull/42")
                .focused(state.focus.current == Some(FocusTarget::PullRequestInput))
                .on_click(Action::SetFocus(Some(FocusTarget::PullRequestInput)))
                .cursor(state.text_edit.cursor)
                .anchor(state.text_edit.anchor)
                .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
                .focus_target(FocusTarget::PullRequestInput)
                .w_full()
                .h(44.0),
        );

    if let Some(info) = state.github.pull_request.info.as_ref() {
        panel = panel.child(
            div()
                .flex_col()
                .gap(Sp::SM)
                .p(Sp::MD)
                .rounded_md()
                .bg(tc.surface)
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap(Sp::SM)
                        .child(svg_icon(lucide::GIT_PULL_REQUEST, 14.0).color(tc.accent))
                        .child(
                            text(format!("#{} {}", info.number, info.title))
                                .medium()
                                .color(tc.text_strong),
                        ),
                )
                .child(
                    text("Use this compare to apply the PR base/head refs and start diffing.")
                        .text_sm()
                        .color(tc.text_muted),
                ),
        );
    }

    panel = panel.child(spacer()).child(
        div()
            .flex_row()
            .gap(Sp::LG)
            .child(filled_icon_button(
                lucide::PLAY,
                if state.github.pull_request.status == AsyncStatus::Loading {
                    "Loading\u{2026}"
                } else {
                    "Load PR"
                },
                Action::SubmitPullRequest,
                theme,
            ))
            .optional_child(state.github.pull_request.info.as_ref().map(|_| {
                subtle_icon_button(
                    lucide::GIT_COMPARE,
                    "Use Compare",
                    Action::UsePullRequestCompare,
                    theme,
                )
            })),
    );

    modal_backdrop(theme, width, height).child(panel)
}

fn auth_modal(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    let tc = &theme.colors;
    let (status_icon, status_text) = if state.github.auth.token_present {
        (lucide::CHECK, "Token stored")
    } else if state.github.auth.device_flow.is_some() {
        (lucide::LOADER, "Waiting for authorization")
    } else {
        (lucide::SHIELD, "Not authenticated")
    };

    let (action_icon, action_label, action) = if state.github.auth.device_flow.is_some() {
        (
            lucide::EXTERNAL_LINK,
            "Open Browser",
            Action::OpenDeviceFlowBrowser,
        )
    } else {
        (
            lucide::KEY,
            "Start Device Flow",
            Action::StartGitHubDeviceFlow,
        )
    };

    let mut panel = modal_panel(580.0, theme)
        .h(320.0)
        .child(modal_header(
            "GitHub Device Flow",
            "Authenticate with GitHub to access private repositories and PRs.",
            lucide::SHIELD,
            theme,
        ))
        .child(
            div()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(Sp::SM)
                .child(svg_icon(status_icon, 14.0).color(tc.text_muted))
                .child(text(status_text).text_sm().color(tc.text_muted)),
        );

    if let Some(flow) = state.github.auth.device_flow.as_ref() {
        panel = panel.child(
            div()
                .flex_col()
                .gap(Sp::MD)
                .p(Sp::MD)
                .rounded_md()
                .bg(tc.surface)
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap(Sp::SM)
                        .child(svg_icon(lucide::COPY, 14.0).color(tc.text_muted))
                        .child(
                            text(format!("User code: {}", flow.user_code))
                                .mono()
                                .medium()
                                .color(tc.text_strong),
                        ),
                )
                .child(
                    div()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap(Sp::SM)
                        .child(
                            svg_icon(lucide::EXTERNAL_LINK, 14.0).color(tc.text_accent),
                        )
                        .child(
                            text(&flow.verification_uri)
                                .text_sm()
                                .color(tc.text_accent),
                        ),
                ),
        );
    }

    panel = panel
        .child(spacer())
        .child(filled_icon_button(action_icon, action_label, action, theme));

    modal_backdrop(theme, width, height).child(panel)
}

// ---------------------------------------------------------------------------
// Shared components
// ---------------------------------------------------------------------------

fn filled_icon_button(
    icon: &'static str,
    label: &str,
    action: Action,
    theme: &Theme,
) -> Div {
    div()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(Sp::SM)
        .px(Sp::LG)
        .py(Sp::SM)
        .rounded_md()
        .bg(theme.colors.accent)
        .hover_bg(theme.colors.accent.with_alpha(230))
        .on_click(action)
        .cursor(CursorHint::Pointer)
        .child(svg_icon(icon, 15.0).color(theme.colors.text_strong))
        .child(text(label).medium().color(theme.colors.text_strong))
}

fn subtle_icon_button(
    icon: &'static str,
    label: &str,
    action: Action,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    div()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(Sp::SM)
        .px(Sp::MD)
        .py(Sp::SM)
        .rounded_md()
        .bg(tc.element_background)
        .hover_bg(tc.element_hover)
        .on_click(action)
        .cursor(CursorHint::Pointer)
        .child(svg_icon(icon, 15.0).color(tc.text_muted))
        .child(text(label).text_sm().medium().color(tc.text))
}

fn icon_ghost_btn(
    icon: &'static str,
    label: &str,
    action: Action,
    active: bool,
    theme: &Theme,
) -> Div {
    let tc = &theme.colors;
    let icon_color = if active { tc.text } else { tc.text_muted };
    let text_color = if active { tc.text } else { tc.text_muted };

    div()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(6.0)
        .px_3()
        .py_1()
        .rounded_md()
        .when(active, |d| d.bg(tc.element_background))
        .when(!active, |d| d.hover_bg(tc.ghost_element_hover))
        .on_click(action)
        .cursor(CursorHint::Pointer)
        .child(svg_icon(icon, 14.0).color(icon_color))
        .child(text(label).text_sm().medium().color(text_color))
}

fn toolbar_separator(tc: &crate::ui::theme::ThemeColors) -> Div {
    div().w(1.0).h(20.0).bg(tc.border_variant)
}

fn segmented_control(items: &[(&str, Action, bool)], theme: &Theme) -> Div {
    let tc = &theme.colors;
    let mut row = div()
        .flex_row()
        .flex_shrink_0()
        .rounded_md()
        .bg(tc.element_background)
        .p(3.0)
        .gap(2.0);

    for &(label, ref action, selected) in items {
        row = row.child(
            div()
                .flex_shrink_0()
                .px(Sp::MD)
                .py(5.0)
                .rounded(6.0)
                .when(selected, |d| d.bg(tc.surface).shadow(2.0, 1.0, Color::rgba(0, 0, 0, 40)))
                .when(!selected, |d| d.hover_bg(tc.ghost_element_hover))
                .on_click(action.clone())
                .cursor(CursorHint::Pointer)
                .child(
                    text(label)
                        .text_sm()
                        .medium()
                        .color(if selected { tc.text } else { tc.text_muted }),
                ),
        );
    }

    row
}

fn picker_list<T: PickerItem>(entries: &[T], selected_index: usize, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let mut list = div().flex_1().flex_col().clip().overflow_y_scroll();

    for (i, entry) in entries.iter().enumerate() {
        let selected = i == selected_index;
        list = list.child(
            div()
                .w_full()
                .h(36.0)
                .flex_row()
                .items_center()
                .px(Sp::MD)
                .rounded(5.0)
                .when(selected, |d| d.bg(tc.sidebar_row_selected))
                .when(!selected, |d| d.hover_bg(tc.ghost_element_hover))
                .on_click(Action::SelectOverlayEntry(i))
                .cursor(CursorHint::Pointer)
                .child(
                    div()
                        .flex_1()
                        .flex_col()
                        .child(
                            text(entry.label())
                                .text_sm()
                                .color(if selected { tc.text_strong } else { tc.text }),
                        )
                        .optional_child(entry.detail().map(|d| {
                            text(d).text_xs().color(tc.text_muted)
                        })),
                ),
        );
    }

    list
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compare_mode_label(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::SingleCommit => "single-commit",
        CompareMode::TwoDot => "two-dot",
        CompareMode::ThreeDot => "three-dot",
    }
}

fn renderer_label(renderer: RendererKind) -> &'static str {
    match renderer {
        RendererKind::Builtin => "built-in",
        RendererKind::Difftastic => "difftastic",
    }
}

fn display_ref(value: &str) -> &str {
    if value.is_empty() {
        "?"
    } else {
        value
    }
}
