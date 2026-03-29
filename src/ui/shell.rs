use std::cell::Cell;
use std::rc::Rc;

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::render::{Rect, RectPrimitive, Scene, TextMetrics};
use crate::ui::actions::Action;
use crate::ui::design::{Sp, TextStyle};
use crate::ui::diff_viewport::runtime::{DiffViewportRuntime, ViewportDocument};
use crate::ui::element::*;
use crate::ui::state::{
    AppState, AsyncStatus, CompareField, FocusTarget, OverlaySurface, PickerItem, WorkspaceMode,
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

    let gap = theme.metrics.spacing_sm;

    // --- Main content tree ---
    let mut root = div()
        .w(width)
        .h(height)
        .flex_col()
        .bg(theme.colors.background)
        .p(gap)
        .gap(gap)
        .child(title_bar(state, theme))
        .child(
            div()
                .flex_row()
                .flex_1()
                .gap(gap)
                .child(sidebar(state, theme, file_list_bounds.clone()))
                .child(main_surface(state, theme, text_metrics, viewport_bounds.clone())),
        )
        .child(status_bar(state, theme));

    // --- Toasts (z-indexed above main content) ---
    let toast_width = 360.0_f32.min((width - 32.0).max(220.0));
    let toast_height = 52.0;
    if !state.toasts.is_empty() {
        let toasts_state = state
            .toasts
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.message.clone(), t.kind))
            .collect::<Vec<_>>();

        root = root.child(
            canvas({
                let theme_colors = ToastColors {
                    surface: theme.colors.elevated_surface,
                    text: theme.colors.text,
                    text_muted: theme.colors.text_muted,
                    error_surface: theme.colors.status_error,
                    border: theme.colors.border,
                    radius: theme.metrics.panel_radius,
                    font_size: theme.metrics.ui_font_size,
                };
                move |_bounds, scene, cx| {
                    for (offset, &(index, ref message, kind)) in
                        toasts_state.iter().rev().enumerate()
                    {
                        let rect = Rect {
                            x: width - toast_width - Sp::XL,
                            y: height
                                - 30.0 // status bar height
                                - Sp::XXL
                                - toast_height
                                - offset as f32 * (toast_height + Sp::LG),
                            width: toast_width,
                            height: toast_height,
                        };
                        paint_toast(scene, cx, rect, message, kind, index, &theme_colors);
                    }
                }
            })
            .w(0.0)
            .h(0.0),
        );
    }

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
        }
    }

    UiFrame {
        scene,
        hits: std::mem::take(&mut cx.hits),
        scroll_regions: std::mem::take(&mut cx.scroll_regions),
        file_list_rect: file_list_bounds.get(),
        viewport_rect: viewport_bounds.get(),
    }
}

// ---------------------------------------------------------------------------
// Title bar
// ---------------------------------------------------------------------------

fn title_bar(state: &AppState, theme: &Theme) -> Div {
    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("diffy");

    let mut bar = div()
        .flex_row()
        .items_center()
        .h(theme.metrics.title_bar_height)
        .w_full()
        .px(Sp::XL)
        .bg(theme.colors.title_bar_background)
        .rounded(theme.metrics.panel_radius)
        .child(text(repo_label).text_lg().color(theme.colors.text_strong));

    // Center: compare summary
    if state.workspace_mode == WorkspaceMode::Ready {
        let summary = format!(
            "{} files  \u{00b7}  {} \u{2192} {}",
            state.workspace.files.len(),
            state.compare.resolved_left.as_deref().unwrap_or("?"),
            state.compare.resolved_right.as_deref().unwrap_or("?")
        );
        bar = bar.child(
            text(summary)
                .text_sm()
                .color(theme.colors.text_muted),
        );
    }

    bar = bar.child(spacer());

    // Right: toolbar buttons
    let btn_style = |label: &str, action: Action, selected: bool| -> Div {
        div()
            .px(14.0)
            .py(6.0)
            .rounded(7.0)
            .items_center()
            .justify_center()
            .when(selected, |d| d.bg(theme.colors.element_background))
            .when(!selected, |d| d.hover_bg(theme.colors.ghost_element_hover))
            .on_click(action)
            .child(text(label).text_sm().color(
                if selected { theme.colors.text } else { theme.colors.text_muted },
            ))
    };

    bar = bar.child(
        div()
            .flex_row()
            .items_center()
            .gap(Sp::SM)
            .child(btn_style(
                "Compare",
                Action::OpenCompareSheet,
                state.overlays.top() == Some(OverlaySurface::CompareSheet),
            ))
            .child(btn_style(
                "PR",
                Action::OpenPullRequestModal,
                state.overlays.top() == Some(OverlaySurface::PullRequestModal),
            ))
            .child(div().w(Sp::SM)) // separator
            .child(segmented_control(
                &[
                    ("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
                    ("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
                ],
                theme,
            ))
            .child(btn_style("Wrap", Action::ToggleWrap, state.viewport.wrap_enabled))
            .child(btn_style(
                if theme.mode == crate::ui::theme::ThemeMode::Dark { "\u{263e}" } else { "\u{2600}" },
                Action::ToggleThemeMode,
                false,
            )),
    );

    bar
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

fn sidebar(state: &AppState, theme: &Theme, bounds_cell: Rc<Cell<Option<Rect>>>) -> Div {
    let file_count = state.workspace.files.len();
    let header_text = if file_count > 0 {
        format!("Files  \u{00b7}  {file_count}")
    } else {
        "Files".to_owned()
    };

    let mut sidebar = div()
        .flex_col()
        .w(theme.metrics.sidebar_width)
        .flex_shrink_0()
        .h_full()
        .bg(theme.colors.sidebar_background)
        .rounded(theme.metrics.panel_radius)
        .p(Sp::MD)
        .gap(Sp::MD)
        .child(text(header_text).text_sm().color(theme.colors.text_muted));

    if state.workspace.files.is_empty() {
        let msg = if state.compare.repo_path.is_some() {
            "Run a compare to see changes."
        } else {
            "Open a repository to start."
        };
        sidebar = sidebar.child(text(msg).text_sm().color(theme.colors.text_muted));
    } else {
        let row_height = state.file_list.row_height;
        let scroll_offset = state.file_list.scroll_offset as f32 * row_height;

        let mut list = div()
            .flex_1()
            .flex_col()
            .clip()
            .scroll_y(scroll_offset)
            .on_scroll(ScrollActionBuilder::FileList);

        for (index, file) in state.workspace.files.iter().enumerate() {
            let selected = state.workspace.selected_file_index == Some(index);
            let detail = format!("+{} \u{2212}{}", file.additions, file.deletions);

            list = list.child(
                div()
                    .w_full()
                    .h(row_height - 2.0)
                    .flex_row()
                    .items_center()
                    .px(Sp::SM)
                    .rounded(7.0)
                    .when(selected, |d| d.bg(theme.colors.sidebar_row_selected))
                    .when(!selected, |d| d.hover_bg(theme.colors.sidebar_row_hover))
                    .on_click(Action::SelectFile(index))
                    .child(
                        div()
                            .flex_1()
                            .flex_col()
                            .child(text(&file.path).text_sm().color(theme.colors.text))
                            .child(
                                text(detail)
                                    .text_xs()
                                    .color(theme.colors.text_muted),
                            ),
                    ),
            );
        }

        sidebar = sidebar.child(list);
    }

    // Record sidebar bounds via canvas
    sidebar = sidebar.child(
        canvas(move |bounds, _scene, _cx| {
            // Walk up to get the sidebar's outer bounds from this child's parent.
            // Actually, the bounds here are this canvas's bounds (0x0).
            // Use a different approach: record in prepaint.
        })
        .w(0.0)
        .h(0.0),
    );

    // Better approach: wrap in a canvas that records bounds
    // Actually, let's use the simpler approach and compute it from layout
    // The file_list_rect is used for scroll hit testing in app.rs.
    // We'll set it from the sidebar's known width/position.

    sidebar
}

// ---------------------------------------------------------------------------
// Main surface
// ---------------------------------------------------------------------------

fn main_surface(
    state: &AppState,
    theme: &Theme,
    text_metrics: TextMetrics,
    viewport_bounds: Rc<Cell<Option<Rect>>>,
) -> Div {
    let mut main = div()
        .flex_1()
        .flex_col()
        .h_full()
        .bg(theme.colors.editor_surface)
        .rounded(theme.metrics.panel_radius);

    let has_overlay = state.active_overlay_name().is_some();
    match state.workspace_mode {
        WorkspaceMode::Ready => {
            // Toolbar
            let file_label = state
                .workspace
                .selected_file_path
                .as_deref()
                .unwrap_or("No file selected");
            main = main.child(
                div()
                    .h(32.0)
                    .px(Sp::LG)
                    .flex_row()
                    .items_center()
                    .child(text(file_label).text_sm().color(theme.colors.text_muted)),
            );

            // Viewport placeholder — captures bounds, painted after element tree
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
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(420.0)
                .p(Sp::XL)
                .flex_col()
                .gap(Sp::MD)
                .bg(theme.colors.elevated_surface)
                .rounded(theme.metrics.panel_radius)
                .shadow(12.0, 4.0, Color::rgba(0, 0, 0, 60))
                .child(text("Comparing repository\u{2026}").text_lg())
                .child(
                    text(format!(
                        "{} \u{2022} {} -> {}",
                        compare_mode_label(state.compare.mode),
                        display_ref(&state.compare.left_ref),
                        display_ref(&state.compare.right_ref)
                    ))
                    .text_sm()
                    .color(theme.colors.text_muted),
                ),
        )
}

fn empty_state(state: &AppState, theme: &Theme) -> Div {
    let title = if state.compare.repo_path.is_some() {
        "Open compare setup"
    } else {
        "Start a new compare"
    };
    let subtitle = if state.compare.repo_path.is_some() {
        "Use the compare sheet, PR modal, or command palette to build a diff."
    } else {
        "Choose a repository, select refs, then open the native diff workspace."
    };

    let mut card = div()
        .flex_1()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(540.0)
                .p(Sp::XXL)
                .flex_col()
                .gap(Sp::MD)
                .bg(theme.colors.empty_state_background)
                .border_b(theme.colors.empty_state_border)
                .rounded(theme.metrics.panel_radius)
                .child(text(title).text_lg())
                .child(text(subtitle).text_sm().color(theme.colors.text_muted))
                .child(
                    div()
                        .flex_row()
                        .gap(Sp::LG)
                        .child(filled_button("Open Compare", Action::OpenCompareSheet, theme))
                        .child(subtle_button("Folder Dialog", Action::OpenRepositoryDialog, theme)),
                )
                .child(text("Recent repositories").text_sm().color(theme.colors.text_muted))
                .children_from(
                    state
                        .settings
                        .recent_repos
                        .iter()
                        .take(4)
                        .map(|repo| {
                            div()
                                .w_full()
                                .py(4.0)
                                .hover_bg(theme.colors.ghost_element_hover)
                                .rounded(4.0)
                                .on_click(Action::OpenRepository(repo.clone()))
                                .cursor(CursorHint::Pointer)
                                .child(
                                    text(repo.display().to_string())
                                        .text_sm()
                                        .color(theme.colors.text),
                                )
                                .into_any()
                        }),
                ),
        );

    card
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn status_bar(state: &AppState, theme: &Theme) -> Div {
    let status_text = async_status_label(state.repository.status);
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
        .px(Sp::LG)
        .bg(theme.colors.status_bar_background)
        .rounded(theme.metrics.panel_radius)
        .child(text(status_text).text_xs().color(theme.colors.text_muted))
        .child(spacer())
        .child(text(right_text).text_xs().color(theme.colors.text_muted))
}

// ---------------------------------------------------------------------------
// Overlays — modals with backdrop
// ---------------------------------------------------------------------------

fn modal_backdrop(theme: &Theme, width: f32, height: f32) -> Div {
    div()
        .w(width)
        .h(height)
        .z_index(100)
        .bg(Color::rgba(0, 0, 0, 140))
        .on_click(Action::CloseOverlay)
        .items_center()
        .justify_center()
}

fn modal_panel(width: f32, theme: &Theme) -> Div {
    div()
        .w(width)
        .flex_col()
        .p(Sp::XXL)
        .gap(Sp::LG)
        .bg(theme.colors.elevated_surface)
        .rounded(theme.metrics.modal_radius)
        .shadow(24.0, 8.0, Color::rgba(0, 0, 0, 100))
}

fn compare_sheet(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    modal_backdrop(theme, width, height).child(
        modal_panel(760.0, theme)
            .h(380.0)
            .child(text("Compare Setup").text_lg())
            .child(
                text("Pick a repository, refs, compare mode, and renderer.")
                    .text_sm()
                    .color(theme.colors.text_muted),
            )
            // Repo picker button
            .child(
                subtle_button(
                    &state
                        .compare
                        .repo_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "Choose repository".into()),
                    Action::OpenRepoPicker,
                    theme,
                )
                .w_full(),
            )
            // Ref fields
            .child(
                div()
                    .flex_row()
                    .gap(Sp::LG)
                    .child(
                        text_input("Left ref", &state.compare.left_ref)
                            .placeholder("main")
                            .focused(state.focus.current == Some(FocusTarget::CompareLeftRef))
                            .on_click(Action::SetFocus(Some(FocusTarget::CompareLeftRef)))
                            .w_full()
                            .h(56.0)
                            .flex_1(),
                    )
                    .child(
                        text_input("Right ref", &state.compare.right_ref)
                            .placeholder("feature")
                            .focused(state.focus.current == Some(FocusTarget::CompareRightRef))
                            .on_click(Action::SetFocus(Some(FocusTarget::CompareRightRef)))
                            .w_full()
                            .h(56.0)
                            .flex_1(),
                    ),
            )
            // Compare mode
            .child(segmented_control(
                &[
                    ("Single", Action::SetCompareMode(CompareMode::SingleCommit), state.compare.mode == CompareMode::SingleCommit),
                    ("Two Dot", Action::SetCompareMode(CompareMode::TwoDot), state.compare.mode == CompareMode::TwoDot),
                    ("Three Dot", Action::SetCompareMode(CompareMode::ThreeDot), state.compare.mode == CompareMode::ThreeDot),
                ],
                theme,
            ))
            // Layout + renderer controls
            .child(
                div()
                    .flex_row()
                    .gap(Sp::XL)
                    .child(segmented_control(
                        &[
                            ("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
                            ("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
                        ],
                        theme,
                    ))
                    .child(segmented_control(
                        &[
                            ("Built-in", Action::SetRenderer(RendererKind::Builtin), state.compare.renderer == RendererKind::Builtin),
                            ("Difftastic", Action::SetRenderer(RendererKind::Difftastic), state.compare.renderer == RendererKind::Difftastic),
                        ],
                        theme,
                    )),
            )
            // Validation message
            .optional_child(
                state.overlays.compare_sheet.validation_message.as_deref().map(|msg| {
                    text(msg).text_sm().color(theme.colors.status_error)
                }),
            )
            .child(spacer())
            // Footer
            .child(
                div()
                    .flex_row()
                    .justify_end()
                    .child(filled_button(
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
            .child(text("Repository Picker").text_lg())
            .child(
                text_input("Search or type a path", &state.overlays.picker.query)
                    .placeholder("C:\\work\\repo")
                    .focused(state.focus.current == Some(FocusTarget::PickerInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
                    .w_full()
                    .h(40.0),
            )
            .child(picker_list(&state.overlays.picker.entries, state.overlays.picker.selected_index, theme))
            .child(
                subtle_button("Folder Dialog", Action::OpenRepositoryDialog, theme),
            ),
    )
}

fn ref_picker(
    state: &AppState,
    theme: &Theme,
    field: CompareField,
    width: f32,
    height: f32,
) -> Div {
    let title = match field {
        CompareField::Left => "Pick Left Ref",
        CompareField::Right => "Pick Right Ref",
    };
    let current_value = match field {
        CompareField::Left => &state.compare.left_ref,
        CompareField::Right => &state.compare.right_ref,
    };

    modal_backdrop(theme, width, height).child(
        modal_panel(480.0, theme)
            .h(380.0)
            .child(text(title).text_lg())
            .child(
                text_input("Filter refs", current_value)
                    .placeholder("Search branches, tags, commits")
                    .focused(state.focus.current == Some(FocusTarget::PickerInput))
                    .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
                    .w_full()
                    .h(40.0),
            )
            .child(picker_list(&state.overlays.picker.entries, state.overlays.picker.selected_index, theme)),
    )
}

fn command_palette(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    // Command palette is positioned at top, not centered
    div()
        .w(width)
        .h(height)
        .z_index(100)
        .bg(Color::rgba(0, 0, 0, 140))
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
                .rounded(theme.metrics.modal_radius)
                .shadow(24.0, 8.0, Color::rgba(0, 0, 0, 100))
                .child(
                    text_input("Command palette", &state.overlays.command_palette.query)
                        .placeholder("Type a command, file, repo, or ref")
                        .focused(state.focus.current == Some(FocusTarget::CommandPaletteInput))
                        .on_click(Action::SetFocus(Some(FocusTarget::CommandPaletteInput)))
                        .w_full()
                        .h(42.0),
                )
                .child(picker_list(
                    &state.overlays.command_palette.entries,
                    state.overlays.command_palette.selected_index,
                    theme,
                )),
        )
}

fn pull_request_modal(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    let mut panel = modal_panel(640.0, theme)
        .h(320.0)
        .child(text("GitHub Pull Request").text_lg())
        .child(
            text_input("Pull request URL", &state.github.pull_request.url_input)
                .placeholder("https://github.com/owner/repo/pull/42")
                .focused(state.focus.current == Some(FocusTarget::PullRequestInput))
                .on_click(Action::SetFocus(Some(FocusTarget::PullRequestInput)))
                .w_full()
                .h(40.0),
        );

    if let Some(info) = state.github.pull_request.info.as_ref() {
        panel = panel.child(
            div()
                .flex_col()
                .gap(Sp::SM)
                .child(text(format!("#{} {}", info.number, info.title)))
                .child(
                    text("Use this compare to apply the PR base/head refs and start diffing.")
                        .text_sm()
                        .color(theme.colors.text_muted),
                ),
        );
    }

    panel = panel.child(spacer()).child(
        div()
            .flex_row()
            .gap(Sp::LG)
            .child(filled_button(
                if state.github.pull_request.status == AsyncStatus::Loading {
                    "Loading\u{2026}"
                } else {
                    "Load PR"
                },
                Action::SubmitPullRequest,
                theme,
            ))
            .optional_child(state.github.pull_request.info.as_ref().map(|_| {
                subtle_button("Use Compare", Action::UsePullRequestCompare, theme)
            })),
    );

    modal_backdrop(theme, width, height).child(panel)
}

fn auth_modal(state: &AppState, theme: &Theme, width: f32, height: f32) -> Div {
    let status = if state.github.auth.token_present {
        "Token stored"
    } else if state.github.auth.device_flow.is_some() {
        "Waiting for authorization"
    } else {
        "Not authenticated"
    };

    let (action_label, action) = if state.github.auth.device_flow.is_some() {
        ("Open Browser", Action::OpenDeviceFlowBrowser)
    } else {
        ("Start Device Flow", Action::StartGitHubDeviceFlow)
    };

    let mut panel = modal_panel(580.0, theme)
        .h(300.0)
        .child(text("GitHub Device Flow").text_lg())
        .child(text(status).color(theme.colors.text_muted));

    if let Some(flow) = state.github.auth.device_flow.as_ref() {
        panel = panel.child(
            div()
                .flex_col()
                .gap(Sp::MD)
                .child(text(format!("User code: {}", flow.user_code)).mono())
                .child(text(&flow.verification_uri).text_sm().color(theme.colors.text_accent)),
        );
    }

    panel = panel
        .child(spacer())
        .child(filled_button(action_label, action, theme));

    modal_backdrop(theme, width, height).child(panel)
}

// ---------------------------------------------------------------------------
// Shared components
// ---------------------------------------------------------------------------

fn filled_button(label: &str, action: Action, theme: &Theme) -> Div {
    div()
        .px(16.0)
        .py(8.0)
        .rounded(7.0)
        .bg(theme.colors.accent)
        .on_click(action)
        .child(text(label).text_sm().color(theme.colors.text_strong))
}

fn subtle_button(label: &str, action: Action, theme: &Theme) -> Div {
    div()
        .px(16.0)
        .py(8.0)
        .rounded(7.0)
        .bg(theme.colors.element_background)
        .hover_bg(theme.colors.element_hover)
        .on_click(action)
        .child(text(label).text_sm().color(theme.colors.text))
}

fn segmented_control(items: &[(&str, Action, bool)], theme: &Theme) -> Div {
    let mut row = div()
        .flex_row()
        .rounded(7.0)
        .bg(theme.colors.element_background)
        .p(2.0)
        .gap(2.0);

    for &(label, ref action, selected) in items {
        row = row.child(
            div()
                .px(12.0)
                .py(4.0)
                .rounded(5.0)
                .when(selected, |d| d.bg(theme.colors.accent))
                .on_click(action.clone())
                .child(text(label).text_xs().color(
                    if selected { theme.colors.text_strong } else { theme.colors.text_muted },
                )),
        );
    }

    row
}

fn picker_list<T: PickerItem>(
    entries: &[T],
    selected_index: usize,
    theme: &Theme,
) -> Div {
    let mut list = div()
        .flex_1()
        .flex_col()
        .clip()
        .overflow_y_scroll();

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
                .when(selected, |d| d.bg(theme.colors.sidebar_row_selected))
                .when(!selected, |d| d.hover_bg(theme.colors.ghost_element_hover))
                .on_click(Action::SelectOverlayEntry(i))
                .child(
                    div()
                        .flex_1()
                        .flex_col()
                        .child(text(entry.label()).text_sm().color(theme.colors.text))
                        .optional_child(entry.detail().map(|d| {
                            text(d).text_xs().color(theme.colors.text_muted)
                        })),
                ),
        );
    }

    list
}

// ---------------------------------------------------------------------------
// Toast painting (via canvas, for absolute positioning)
// ---------------------------------------------------------------------------

struct ToastColors {
    surface: Color,
    text: Color,
    text_muted: Color,
    error_surface: Color,
    border: Color,
    radius: f32,
    font_size: f32,
}

fn paint_toast(
    scene: &mut Scene,
    cx: &mut ElementContext,
    rect: Rect,
    message: &str,
    kind: crate::ui::state::ToastKind,
    index: usize,
    colors: &ToastColors,
) {
    use crate::render::{
        BorderPrimitive, FontKind, RoundedRectPrimitive, ShadowPrimitive, TextPrimitive,
    };

    let fill = match kind {
        crate::ui::state::ToastKind::Info => colors.surface,
        crate::ui::state::ToastKind::Error => colors.error_surface,
    };

    scene.shadow(ShadowPrimitive {
        rect,
        blur_radius: 16.0,
        corner_radius: colors.radius,
        offset: [0.0, 4.0],
        color: Color::rgba(0, 0, 0, 60),
    });
    scene.rounded_rect(RoundedRectPrimitive::uniform(rect, colors.radius, fill));
    scene.border(BorderPrimitive::uniform(rect, 1.0, colors.radius, colors.border));
    scene.text(TextPrimitive {
        rect: rect.pad(Sp::LG, 0.0, 40.0, 0.0),
        text: message.to_string(),
        color: colors.text,
        font_size: colors.font_size,
        font_kind: FontKind::Ui,
    });

    cx.hits.push(HitRegion {
        rect,
        action: Action::DismissToast(index),
        cursor: CursorHint::Pointer,
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn async_status_label(status: AsyncStatus) -> &'static str {
    match status {
        AsyncStatus::Idle => "idle",
        AsyncStatus::Loading => "loading",
        AsyncStatus::Ready => "ready",
        AsyncStatus::Failed => "failed",
    }
}

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
    if value.is_empty() { "?" } else { value }
}
