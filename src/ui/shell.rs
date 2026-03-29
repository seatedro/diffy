use taffy::prelude::{AvailableSpace, TaffyTree, length};

use crate::core::compare::{CompareMode, LayoutMode, RendererKind};
use crate::render::{Rect, RectPrimitive, TextMetrics};
use crate::ui::actions::Action;
use crate::ui::animation::AnimationKey;
use crate::ui::components::{
    Button, ButtonStyle, Label, ListItem, Modal, PickerList, SegmentedControl, Surface, TextInput,
    Toast,
};
use crate::ui::design::{Sp, TextStyle};
use crate::ui::diff_viewport::runtime::{DiffViewportRuntime, ViewportDocument};
use crate::ui::layout::{Fl, Fx, hstack, right_align, vstack};
use crate::ui::state::{
    AppState, AsyncStatus, CompareField, FocusTarget, OverlaySurface, WorkspaceMode,
};
use crate::ui::theme::Theme;

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

#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: Action,
    pub hover_file_index: Option<usize>,
    pub hover_toast_index: Option<usize>,
    pub cursor: CursorHint,
}

#[derive(Debug, Clone, Default)]
pub struct UiFrame {
    pub scene: crate::render::Scene,
    pub hits: Vec<HitRegion>,
    pub file_list_rect: Option<Rect>,
    pub viewport_rect: Option<Rect>,
}

// ---------------------------------------------------------------------------
// Internal layout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
struct WorkspaceLayout {
    title_bar: Rect,
    sidebar: Rect,
    main: Rect,
    status_bar: Rect,
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
) -> UiFrame {
    let mut frame = UiFrame::default();
    let layout = layout_workspace(width, height, theme);

    frame.scene.rect(RectPrimitive {
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        },
        color: theme.colors.background,
    });

    draw_title_bar(&mut frame, state, theme, layout.title_bar);
    draw_sidebar(&mut frame, state, theme, layout.sidebar);
    draw_main_surface(
        &mut frame,
        state,
        theme,
        viewport_runtime,
        text_metrics,
        layout.main,
    );
    draw_status_bar(&mut frame, state, theme, layout.status_bar);
    draw_toasts(&mut frame, state, theme, width, height);

    // Only render the topmost overlay. Text renders in a separate GPU pass
    // after all quads, so stacked overlays would bleed through each other.
    if let Some(top) = state.overlays.stack.last().cloned() {
        match top.surface {
            OverlaySurface::CompareSheet => {
                draw_compare_sheet(&mut frame, state, theme, width, height);
            }
            OverlaySurface::RepoPicker => {
                draw_repo_picker(&mut frame, state, theme, width, height);
            }
            OverlaySurface::RefPicker(field) => {
                draw_ref_picker(&mut frame, state, theme, field, width, height);
            }
            OverlaySurface::CommandPalette => {
                draw_command_palette(&mut frame, state, theme, width, height);
            }
            OverlaySurface::PullRequestModal => {
                draw_pull_request_modal(&mut frame, state, theme, width, height);
            }
            OverlaySurface::GitHubAuthModal => {
                draw_auth_modal(&mut frame, state, theme, width, height);
            }
        }
    }

    frame
}

// ---------------------------------------------------------------------------
// Workspace layout (Taffy — the one place we keep it)
// ---------------------------------------------------------------------------

fn layout_workspace(width: f32, height: f32, theme: &Theme) -> WorkspaceLayout {
    let mut tree = TaffyTree::<()>::new();
    let title_bar = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: length(theme.metrics.title_bar_height),
            },
            ..Default::default()
        })
        .unwrap();
    let sidebar = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: length(theme.metrics.sidebar_width),
                height: taffy::prelude::auto(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .unwrap();
    let main = tree
        .new_leaf(taffy::Style {
            flex_grow: 1.0,
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: taffy::prelude::auto(),
            },
            ..Default::default()
        })
        .unwrap();
    let body = tree
        .new_with_children(
            taffy::Style {
                flex_grow: 1.0,
                flex_direction: taffy::FlexDirection::Row,
                gap: taffy::Size {
                    width: length(theme.metrics.spacing_sm),
                    height: length(0.0),
                },
                ..Default::default()
            },
            &[sidebar, main],
        )
        .unwrap();
    let status_bar = tree
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::auto(),
                height: length(theme.metrics.status_bar_height),
            },
            ..Default::default()
        })
        .unwrap();
    let root = tree
        .new_with_children(
            taffy::Style {
                size: taffy::Size {
                    width: length(width),
                    height: length(height),
                },
                padding: taffy::Rect {
                    left: length(theme.metrics.spacing_sm),
                    right: length(theme.metrics.spacing_sm),
                    top: length(theme.metrics.spacing_sm),
                    bottom: length(theme.metrics.spacing_sm),
                },
                flex_direction: taffy::FlexDirection::Column,
                gap: taffy::Size {
                    width: length(0.0),
                    height: length(theme.metrics.spacing_sm),
                },
                ..Default::default()
            },
            &[title_bar, body, status_bar],
        )
        .unwrap();

    tree.compute_layout(
        root,
        taffy::Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        },
    )
    .unwrap();

    let body_layout = tree.layout(body).unwrap();
    let body_x = body_layout.location.x;
    let body_y = body_layout.location.y;

    let mut sidebar_rect = rect_from_layout(tree.layout(sidebar).unwrap());
    sidebar_rect.x += body_x;
    sidebar_rect.y += body_y;

    let mut main_rect = rect_from_layout(tree.layout(main).unwrap());
    main_rect.x += body_x;
    main_rect.y += body_y;

    WorkspaceLayout {
        title_bar: rect_from_layout(tree.layout(title_bar).unwrap()),
        sidebar: sidebar_rect,
        main: main_rect,
        status_bar: rect_from_layout(tree.layout(status_bar).unwrap()),
    }
}

// ---------------------------------------------------------------------------
// Title bar
// ---------------------------------------------------------------------------

fn draw_title_bar(frame: &mut UiFrame, state: &AppState, theme: &Theme, rect: Rect) {
    Surface::panel()
        .fill(theme.colors.title_bar_background)
        .paint(frame, rect, theme);

    let content = rect.pad(Sp::XL, 0.0, Sp::XL, 0.0);

    // Left: repo name only — vertically centered, clean
    let repo_label = state
        .compare
        .repo_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("diffy");
    let label_h = theme.metrics.heading_font_size * 1.35;
    Label::new(repo_label)
        .style(TextStyle::Heading)
        .paint(
            frame,
            Rect {
                x: content.x,
                y: content.y + (content.height - label_h) * 0.5,
                width: content.width * 0.25,
                height: label_h,
            },
            theme,
        );

    // Center: compare summary — subtle, secondary
    if state.workspace_mode == WorkspaceMode::Ready {
        let summary = format!(
            "{} files  \u{00b7}  {} \u{2192} {}",
            state.workspace.files.len(),
            state.compare.resolved_left.as_deref().unwrap_or("?"),
            state.compare.resolved_right.as_deref().unwrap_or("?")
        );
        let summary_h = theme.metrics.ui_small_font_size * 1.35;
        Label::new(&summary)
            .style(TextStyle::BodySmall)
            .paint(
                frame,
                Rect {
                    x: content.x + content.width * 0.25,
                    y: content.y + (content.height - summary_h) * 0.5,
                    width: content.width * 0.3,
                    height: summary_h,
                },
                theme,
            );
    }

    // Right: toolbar — fewer buttons, more space between them
    let btn_h = 30.0;
    let btn_y = rect.y + (rect.height - btn_h) * 0.5;
    let gap = Sp::SM;
    let mut x = rect.right() - Sp::XL;

    // Primary actions only in the title bar
    x -= 76.0;
    Button::new("Compare", Action::OpenCompareSheet)
        .style(ButtonStyle::Subtle)
        .selected(state.overlays.top() == Some(OverlaySurface::CompareSheet))
        .paint(frame, Rect { x, y: btn_y, width: 76.0, height: btn_h }, theme);

    x -= 52.0 + gap;
    Button::new("PR", Action::OpenPullRequestModal)
        .selected(state.overlays.top() == Some(OverlaySurface::PullRequestModal))
        .paint(frame, Rect { x, y: btn_y, width: 52.0, height: btn_h }, theme);

    // Separator gap before view controls
    x -= Sp::LG;

    // Layout toggle as segmented control
    let seg_w = 130.0;
    x -= seg_w;
    SegmentedControl::new([
        ("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
        ("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
    ])
    .paint(frame, Rect { x, y: btn_y, width: seg_w, height: btn_h }, theme);

    x -= 56.0 + gap;
    Button::new("Wrap", Action::ToggleWrap)
        .selected(state.viewport.wrap_enabled)
        .paint(frame, Rect { x, y: btn_y, width: 56.0, height: btn_h }, theme);

    x -= 50.0 + gap;
    Button::new(
        if theme.mode == crate::ui::theme::ThemeMode::Dark { "\u{263e}" } else { "\u{2600}" },
        Action::ToggleThemeMode,
    )
    .focused(state.focus.current == Some(FocusTarget::ThemeToggle))
    .paint(frame, Rect { x, y: btn_y, width: 32.0, height: btn_h }, theme);
}

// ---------------------------------------------------------------------------
// Sidebar (file list)
// ---------------------------------------------------------------------------

fn draw_sidebar(frame: &mut UiFrame, state: &AppState, theme: &Theme, rect: Rect) {
    frame.file_list_rect = Some(rect);
    Surface::panel()
        .fill(theme.colors.sidebar_background)
        .paint(frame, rect, theme);

    let content = rect.pad(Sp::MD, Sp::LG, Sp::MD, Sp::SM);
    let [header_row, list_area] = vstack(content, Sp::MD, [Fx(22.0), Fl(1.0)]);

    let file_count = state.workspace.files.len();
    let header_text = if file_count > 0 {
        format!("Files  \u{00b7}  {file_count}")
    } else {
        "Files".to_owned()
    };
    Label::new(&header_text)
        .style(TextStyle::BodySmall)
        .color(theme.colors.text_muted)
        .paint(frame, header_row, theme);

    if state.workspace.files.is_empty() {
        Label::new(if state.compare.repo_path.is_some() {
            "Run a compare to see changes."
        } else {
            "Open a repository to start."
        })
        .style(TextStyle::BodySmall)
        .paint(
            frame,
            Rect {
                y: header_row.bottom() + Sp::MD,
                height: 16.0,
                ..header_row
            },
            theme,
        );
        return;
    }

    frame.scene.clip(list_area);
    let visible = (list_area.height / state.file_list.row_height).ceil().max(1.0) as usize;
    let max_start = state.workspace.files.len().saturating_sub(visible);
    let start = state.file_list.scroll_offset.min(max_start);
    let end = (start + visible + 1).min(state.workspace.files.len());

    for index in start..end {
        let file = &state.workspace.files[index];
        let row = Rect {
            x: list_area.x,
            y: list_area.y + (index - start) as f32 * state.file_list.row_height,
            width: list_area.width,
            height: state.file_list.row_height - 2.0,
        };
        let hover_progress = state
            .animation
            .progress(AnimationKey::FileListHover(index))
            .unwrap_or(0.0);
        ListItem::new(&file.path)
            .detail(format!(
                "+{} \u{2212}{}", file.additions, file.deletions
            ))
            .selected(state.workspace.selected_file_index == Some(index))
            .hover_progress(hover_progress)
            .on_click(Action::SelectFile(index))
            .hover_file_index(index)
            .paint(frame, row, theme);
    }
    frame.scene.pop_clip();
}

// ---------------------------------------------------------------------------
// Main surface
// ---------------------------------------------------------------------------

fn draw_main_surface(
    frame: &mut UiFrame,
    state: &mut AppState,
    theme: &Theme,
    viewport_runtime: &mut DiffViewportRuntime,
    text_metrics: TextMetrics,
    rect: Rect,
) {
    Surface::panel()
        .fill(theme.colors.editor_surface)
        .paint(frame, rect, theme);

    let has_overlay = state.active_overlay_name().is_some();
    match state.workspace_mode {
        WorkspaceMode::Ready => {
            draw_viewport_surface(frame, state, theme, viewport_runtime, text_metrics, rect)
        }
        WorkspaceMode::Loading => draw_loading_state(frame, state, theme, rect),
        WorkspaceMode::Empty if !has_overlay => draw_empty_state(frame, state, theme, rect),
        WorkspaceMode::Empty => {}
    }
}

fn draw_loading_state(frame: &mut UiFrame, state: &AppState, theme: &Theme, rect: Rect) {
    let card = rect.center(420.0, 120.0);
    Surface::raised()
        .fill(theme.colors.elevated_surface)
        .paint(frame, card, theme);

    let content = card.pad(Sp::XXL, Sp::XL, Sp::XXL, Sp::XL);
    let [title_row, detail_row] = vstack(content, Sp::MD, [Fx(22.0), Fx(18.0)]);

    Label::new("Comparing repository\u{2026}")
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);
    Label::new(&format!(
        "{} \u{2022} {} -> {}",
        compare_mode_label(state.compare.mode),
        display_ref(state.compare.left_ref.as_str()),
        display_ref(state.compare.right_ref.as_str())
    ))
    .style(TextStyle::BodySmall)
    .paint(frame, detail_row, theme);
}

fn draw_empty_state(frame: &mut UiFrame, state: &AppState, theme: &Theme, rect: Rect) {
    let card = rect.center(540.0, 300.0);
    Surface::raised()
        .fill(theme.colors.empty_state_background)
        .border(theme.colors.empty_state_border)
        .paint(frame, card, theme);

    let content = card.pad(Sp::XXL, Sp::XXL, Sp::XXL, Sp::XXL);
    let [title_row, subtitle_row, buttons_row, _spacer, recent_header, recent_list] =
        vstack(content, Sp::MD, [
            Fx(22.0),
            Fx(20.0),
            Fx(34.0),
            Fx(Sp::LG),
            Fx(16.0),
            Fl(1.0),
        ]);

    Label::new(if state.compare.repo_path.is_some() {
        "Open compare setup"
    } else {
        "Start a new compare"
    })
    .style(TextStyle::HeadingLg)
    .paint(frame, title_row, theme);

    Label::new(if state.compare.repo_path.is_some() {
        "Use the compare sheet, PR modal, or command palette to build a diff."
    } else {
        "Choose a repository, select refs, then open the native diff workspace."
    })
    .style(TextStyle::Body)
    .color(theme.colors.text_muted)
    .paint(frame, subtitle_row, theme);

    let [primary_btn, dialog_btn] =
        hstack(buttons_row, Sp::LG, [Fx(138.0), Fx(160.0)]);
    Button::new("Open Compare", Action::OpenCompareSheet)
        .style(ButtonStyle::Filled)
        .focused(state.focus.current == Some(FocusTarget::WorkspacePrimaryButton))
        .paint(frame, primary_btn, theme);
    Button::new("Folder Dialog", Action::OpenRepositoryDialog)
        .style(ButtonStyle::Subtle)
        .paint(frame, dialog_btn, theme);

    Label::new("Recent repositories")
        .style(TextStyle::BodySmall)
        .paint(frame, recent_header, theme);

    let repo_rows = vstack(
        recent_list,
        Sp::XS,
        [Fx(26.0), Fx(26.0), Fx(26.0), Fx(26.0)],
    );
    for (i, repo) in state.settings.recent_repos.iter().take(4).enumerate() {
        Label::new(&repo.display().to_string())
            .style(TextStyle::BodySmall)
            .color(theme.colors.text)
            .paint(frame, repo_rows[i], theme);
        frame.hits.push(HitRegion {
            rect: repo_rows[i],
            action: Action::OpenRepository(repo.clone()),
            hover_file_index: None,
            hover_toast_index: None,
            cursor: CursorHint::Pointer,
        });
    }
}

// ---------------------------------------------------------------------------
// Viewport surface (file toolbar + diff)
// ---------------------------------------------------------------------------

fn draw_viewport_surface(
    frame: &mut UiFrame,
    state: &mut AppState,
    theme: &Theme,
    viewport_runtime: &mut DiffViewportRuntime,
    text_metrics: TextMetrics,
    rect: Rect,
) {
    let content = rect.pad(0.0, Sp::SM, 0.0, 0.0);
    let [toolbar, viewport_bounds] = vstack(content, Sp::SM, [Fx(32.0), Fl(1.0)]);

    let tb_content = toolbar.pad(Sp::LG, 0.0, Sp::LG, 0.0);
    let label_h = theme.metrics.ui_font_size * 1.35;
    Label::new(
        state
            .workspace
            .selected_file_path
            .as_deref()
            .unwrap_or("No file selected"),
    )
    .style(TextStyle::BodySmall)
    .paint(
        frame,
        Rect {
            y: tb_content.y + (tb_content.height - label_h) * 0.5,
            height: label_h,
            ..tb_content
        },
        theme,
    );

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
    viewport_runtime.prepare(&mut state.viewport, document, viewport_bounds, text_metrics);
    frame.viewport_rect = Some(viewport_runtime.body_bounds());
    viewport_runtime.paint(&mut frame.scene, theme, &state.viewport, document);
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn draw_status_bar(frame: &mut UiFrame, state: &AppState, theme: &Theme, rect: Rect) {
    Surface::panel()
        .fill(theme.colors.status_bar_background)
        .paint(frame, rect, theme);

    let content = rect.pad(Sp::LG, 0.0, Sp::LG, 0.0);
    let label_h = theme.metrics.ui_small_font_size * 1.35;

    // Left: status
    let status_text = async_status_label(state.repository.status);
    Label::new(status_text)
        .style(TextStyle::Caption)
        .paint(
            frame,
            Rect {
                y: content.y + (content.height - label_h) * 0.5,
                height: label_h,
                ..content
            },
            theme,
        );

    // Right: mode
    let right_text = format!(
        "{}  \u{00b7}  {}",
        compare_mode_label(state.compare.mode),
        renderer_label(state.compare.renderer),
    );
    let right_w = right_text.len() as f32 * 6.5;
    Label::new(&right_text)
        .style(TextStyle::Caption)
        .paint(
            frame,
            Rect {
                x: content.right() - right_w,
                y: content.y + (content.height - label_h) * 0.5,
                width: right_w,
                height: label_h,
            },
            theme,
        );
}

// ---------------------------------------------------------------------------
// Toasts
// ---------------------------------------------------------------------------

fn draw_toasts(frame: &mut UiFrame, state: &AppState, theme: &Theme, width: f32, height: f32) {
    let toast_width = 360.0_f32.min((width - 32.0).max(220.0));
    let toast_height = 52.0;
    let gap = Sp::LG;
    for (offset, (index, toast)) in state.toasts.iter().enumerate().rev().enumerate() {
        let rect = Rect {
            x: width - toast_width - Sp::XL,
            y: height
                - theme.metrics.status_bar_height
                - Sp::XXL
                - toast_height
                - offset as f32 * (toast_height + gap),
            width: toast_width,
            height: toast_height,
        };
        Toast::new(&toast.message, toast.kind, index).paint(frame, rect, theme);
    }
}

// ---------------------------------------------------------------------------
// Compare sheet
// ---------------------------------------------------------------------------

fn draw_compare_sheet(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let bounds = Rect { x: 0.0, y: 0.0, width, height };
    let panel = bounds.center(760.0, 380.0);
    Modal::backdrop(frame, theme, width, height);
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XXL, 20.0, Sp::XXL, Sp::XL);
    let [title_row, subtitle_row, repo_row, fields_row, mode_row, controls_row, _spacer, footer_row] =
        vstack(content, Sp::LG, [
            Fx(22.0),
            Fx(16.0),
            Fx(34.0),
            Fx(58.0),
            Fx(32.0),
            Fx(32.0),
            Fl(1.0),
            Fx(32.0),
        ]);

    Label::new("Compare Setup")
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);
    Label::new("Pick a repository, refs, compare mode, and renderer.")
        .style(TextStyle::BodySmall)
        .paint(frame, subtitle_row, theme);

    Button::new(
        state
            .compare
            .repo_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Choose repository".to_owned()),
        Action::OpenRepoPicker,
    )
    .style(ButtonStyle::Subtle)
    .focused(state.focus.current == Some(FocusTarget::CompareRepoButton))
    .paint(frame, repo_row, theme);

    // Two ref fields side by side
    let [left_field, right_field] = hstack(fields_row, Sp::LG, [Fl(1.0), Fl(1.0)]);

    TextInput::new("Left ref", &state.compare.left_ref)
        .placeholder("main")
        .focused(state.focus.current == Some(FocusTarget::CompareLeftRef))
        .on_click(Action::SetFocus(Some(FocusTarget::CompareLeftRef)))
        .paint(frame, left_field, theme);
    TextInput::new("Right ref", &state.compare.right_ref)
        .placeholder("feature")
        .focused(state.focus.current == Some(FocusTarget::CompareRightRef))
        .on_click(Action::SetFocus(Some(FocusTarget::CompareRightRef)))
        .paint(frame, right_field, theme);

    // Pick buttons overlaid on the fields
    let pick_btn = Rect { width: 62.0, height: 26.0, ..Rect::default() };
    Button::new("Pick", Action::OpenRefPicker(CompareField::Left))
        .style(ButtonStyle::Subtle)
        .paint(frame, Rect {
            x: left_field.right() - pick_btn.width - Sp::MD,
            y: left_field.y + (left_field.height - pick_btn.height) * 0.5,
            ..pick_btn
        },
        theme,
    );
    Button::new("Pick", Action::OpenRefPicker(CompareField::Right))
        .style(ButtonStyle::Subtle)
        .paint(frame, Rect {
            x: right_field.right() - pick_btn.width - Sp::MD,
            y: right_field.y + (right_field.height - pick_btn.height) * 0.5,
            ..pick_btn
        },
        theme,
    );

    SegmentedControl::new([
        ("Single", Action::SetCompareMode(CompareMode::SingleCommit), state.compare.mode == CompareMode::SingleCommit),
        ("Two Dot", Action::SetCompareMode(CompareMode::TwoDot), state.compare.mode == CompareMode::TwoDot),
        ("Three Dot", Action::SetCompareMode(CompareMode::ThreeDot), state.compare.mode == CompareMode::ThreeDot),
    ])
    .paint(frame, mode_row, theme);

    let [layout_seg, renderer_seg] = hstack(controls_row, Sp::XL, [Fx(220.0), Fl(1.0)]);
    SegmentedControl::new([
        ("Unified", Action::SetLayoutMode(LayoutMode::Unified), state.compare.layout == LayoutMode::Unified),
        ("Split", Action::SetLayoutMode(LayoutMode::Split), state.compare.layout == LayoutMode::Split),
    ])
    .paint(frame, layout_seg, theme);
    SegmentedControl::new([
        ("Built-in", Action::SetRenderer(RendererKind::Builtin), state.compare.renderer == RendererKind::Builtin),
        ("Difftastic", Action::SetRenderer(RendererKind::Difftastic), state.compare.renderer == RendererKind::Difftastic),
    ])
    .paint(frame, renderer_seg, theme);

    if let Some(message) = state.overlays.compare_sheet.validation_message.as_deref() {
        Label::new(message)
            .style(TextStyle::BodySmall)
            .color(theme.colors.status_error)
            .paint(
                frame,
                Rect { y: controls_row.bottom() + Sp::SM, height: 16.0, ..controls_row },
                theme,
            );
    }

    let start_btn = right_align(footer_row, 126.0, 32.0);
    Button::new(
        if state.workspace.status == AsyncStatus::Loading { "Comparing\u{2026}" } else { "Start Compare" },
        Action::StartCompare,
    )
    .style(ButtonStyle::Filled)
    .focused(state.focus.current == Some(FocusTarget::CompareStartButton))
    .paint(frame, start_btn, theme);
}

// ---------------------------------------------------------------------------
// Repository picker
// ---------------------------------------------------------------------------

fn draw_repo_picker(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let bounds = Rect { x: 0.0, y: 0.0, width, height };
    let panel = bounds.center(680.0, 420.0);
    Modal::backdrop(frame, theme, width, height);
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XXL, 20.0, Sp::XXL, Sp::XL);
    let [title_row, input_row, list_area, footer_row] =
        vstack(content, Sp::LG, [Fx(22.0), Fx(40.0), Fl(1.0), Fx(30.0)]);

    Label::new("Repository Picker")
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);

    TextInput::new("Search or type a path", &state.overlays.picker.query)
        .placeholder("C:\\work\\repo")
        .focused(state.focus.current == Some(FocusTarget::PickerInput))
        .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
        .paint(frame, input_row, theme);

    PickerList::new(&state.overlays.picker.entries, state.overlays.picker.selected_index)
        .paint(frame, list_area, theme);

    Button::new("Folder Dialog", Action::OpenRepositoryDialog)
        .style(ButtonStyle::Subtle)
        .paint(frame, Rect { width: 160.0, ..footer_row }, theme);
}

// ---------------------------------------------------------------------------
// Ref picker
// ---------------------------------------------------------------------------

fn draw_ref_picker(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    field: CompareField,
    width: f32,
    height: f32,
) {
    let bounds = Rect { x: 0.0, y: 0.0, width, height };
    let panel = bounds.center(480.0, 380.0);
    Modal::backdrop(frame, theme, width, height);
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XXL, 20.0, Sp::XXL, Sp::XL);
    let [title_row, input_row, list_area] =
        vstack(content, Sp::LG, [Fx(22.0), Fx(40.0), Fl(1.0)]);

    let title = match field {
        CompareField::Left => "Pick Left Ref",
        CompareField::Right => "Pick Right Ref",
    };
    Label::new(title)
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);

    TextInput::new(
        "Filter refs",
        match field {
            CompareField::Left => &state.compare.left_ref,
            CompareField::Right => &state.compare.right_ref,
        },
    )
    .placeholder("Search branches, tags, commits")
    .focused(state.focus.current == Some(FocusTarget::PickerInput))
    .on_click(Action::SetFocus(Some(FocusTarget::PickerInput)))
    .paint(frame, input_row, theme);

    PickerList::new(&state.overlays.picker.entries, state.overlays.picker.selected_index)
        .paint(frame, list_area, theme);
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

fn draw_command_palette(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    Modal::backdrop(frame, theme, width, height);

    let panel = Rect {
        x: (width - 720.0) * 0.5,
        y: 56.0,
        width: 720.0,
        height: 420.0,
    };
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XL, Sp::XL, Sp::XL, Sp::XL);
    let [input_row, list_area] = vstack(content, Sp::LG, [Fx(42.0), Fl(1.0)]);

    TextInput::new("Command palette", &state.overlays.command_palette.query)
        .placeholder("Type a command, file, repo, or ref")
        .focused(state.focus.current == Some(FocusTarget::CommandPaletteInput))
        .on_click(Action::SetFocus(Some(FocusTarget::CommandPaletteInput)))
        .paint(frame, input_row, theme);

    PickerList::new(
        &state.overlays.command_palette.entries,
        state.overlays.command_palette.selected_index,
    )
    .paint(frame, list_area, theme);
}

// ---------------------------------------------------------------------------
// Pull request modal
// ---------------------------------------------------------------------------

fn draw_pull_request_modal(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let bounds = Rect { x: 0.0, y: 0.0, width, height };
    let panel = bounds.center(640.0, 320.0);
    Modal::backdrop(frame, theme, width, height);
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XXL, 20.0, Sp::XXL, Sp::XL);
    let [title_row, input_row, info_area, _spacer, footer_row] =
        vstack(content, Sp::LG, [Fx(22.0), Fx(40.0), Fx(40.0), Fl(1.0), Fx(32.0)]);

    Label::new("GitHub Pull Request")
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);

    TextInput::new("Pull request URL", &state.github.pull_request.url_input)
        .placeholder("https://github.com/owner/repo/pull/42")
        .focused(state.focus.current == Some(FocusTarget::PullRequestInput))
        .on_click(Action::SetFocus(Some(FocusTarget::PullRequestInput)))
        .paint(frame, input_row, theme);

    if let Some(info) = state.github.pull_request.info.as_ref() {
        let [pr_title, pr_desc] = vstack(info_area, Sp::SM, [Fx(18.0), Fx(16.0)]);
        Label::new(&format!("#{} {}", info.number, info.title))
            .style(TextStyle::Body)
            .paint(frame, pr_title, theme);
        Label::new("Use this compare to apply the PR base/head refs and start diffing.")
            .style(TextStyle::BodySmall)
            .paint(frame, pr_desc, theme);
    }

    let [load_btn, use_btn] = hstack(footer_row, Sp::LG, [Fx(120.0), Fx(134.0)]);
    Button::new(
        if state.github.pull_request.status == AsyncStatus::Loading { "Loading\u{2026}" } else { "Load PR" },
        Action::SubmitPullRequest,
    )
    .style(ButtonStyle::Filled)
    .focused(state.focus.current == Some(FocusTarget::PullRequestConfirm))
    .paint(frame, load_btn, theme);

    if state.github.pull_request.info.is_some() {
        Button::new("Use Compare", Action::UsePullRequestCompare)
            .style(ButtonStyle::Subtle)
            .paint(frame, use_btn, theme);
    }
}

// ---------------------------------------------------------------------------
// GitHub auth modal
// ---------------------------------------------------------------------------

fn draw_auth_modal(
    frame: &mut UiFrame,
    state: &AppState,
    theme: &Theme,
    width: f32,
    height: f32,
) {
    let bounds = Rect { x: 0.0, y: 0.0, width, height };
    let panel = bounds.center(580.0, 300.0);
    Modal::backdrop(frame, theme, width, height);
    Modal::panel(frame, panel, theme);

    let content = panel.pad(Sp::XXL, 20.0, Sp::XXL, Sp::XL);
    let [title_row, status_row, flow_area, _spacer, action_row] =
        vstack(content, Sp::LG, [Fx(22.0), Fx(18.0), Fx(50.0), Fl(1.0), Fx(32.0)]);

    Label::new("GitHub Device Flow")
        .style(TextStyle::Heading)
        .paint(frame, title_row, theme);

    let status = if state.github.auth.token_present {
        "Token stored"
    } else if state.github.auth.device_flow.is_some() {
        "Waiting for authorization"
    } else {
        "Not authenticated"
    };
    Label::new(status)
        .style(TextStyle::Body)
        .color(theme.colors.text_muted)
        .paint(frame, status_row, theme);

    if let Some(flow) = state.github.auth.device_flow.as_ref() {
        let [code_row, uri_row] = vstack(flow_area, Sp::MD, [Fx(20.0), Fx(18.0)]);
        Label::new(&format!("User code: {}", flow.user_code))
            .style(TextStyle::Mono)
            .paint(frame, code_row, theme);
        Label::new(&flow.verification_uri)
            .style(TextStyle::BodySmall)
            .color(theme.colors.text_accent)
            .paint(frame, uri_row, theme);
    }

    Button::new(
        if state.github.auth.device_flow.is_some() { "Open Browser" } else { "Start Device Flow" },
        if state.github.auth.device_flow.is_some() { Action::OpenDeviceFlowBrowser } else { Action::StartGitHubDeviceFlow },
    )
    .style(ButtonStyle::Filled)
    .focused(state.focus.current == Some(FocusTarget::AuthPrimaryAction))
    .paint(frame, Rect { width: 160.0, ..action_row }, theme);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn rect_from_layout(layout: &taffy::Layout) -> Rect {
    Rect {
        x: layout.location.x,
        y: layout.location.y,
        width: layout.size.width,
        height: layout.size.height,
    }
}

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
