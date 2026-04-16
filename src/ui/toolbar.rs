use std::cell::Cell;
use std::rc::Rc;

use halogen::view;

use crate::actions::Action;
use crate::render::{Rect, TextMetrics};
use crate::ui::components::{self, Button, ButtonStyle, SegmentedControl, SegmentedItem};
use crate::ui::design::{Alpha, Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FocusTarget, WorkspaceMode, WorkspaceSource};
use crate::ui::status_bar::{compare_mode_label, display_ref};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

use crate::core::compare::LayoutMode;

pub(crate) fn main_surface(
    state: &AppState,
    theme: &Theme,
    _text_metrics: TextMetrics,
    viewport_bounds: Rc<Cell<Option<Rect>>>,
) -> AnyElement {
    let tc = &theme.colors;
    let has_overlay = state.active_overlay_name().is_some();

    let toolbar = if state.workspace_mode.get(&state.store) == WorkspaceMode::Ready {
        state
            .workspace
            .selected_file_path
            .as_deref()
            .map(|file_label| viewport_toolbar(state, theme, file_label))
    } else {
        None
    };

    let search = if state.workspace_mode.get(&state.store) == WorkspaceMode::Ready && state.editor.search.open {
        Some(search_bar(state, theme))
    } else {
        None
    };

    let vb = viewport_bounds.clone();
    let viewport_canvas =
        if state.workspace_mode.get(&state.store) == WorkspaceMode::Ready && state.workspace.active_file.is_some() {
            Some(
                canvas(move |bounds, _scene, _cx| {
                    vb.set(Some(bounds));
                })
                .flex_1()
                .into_any(),
            )
        } else {
            None
        };

    let content = match state.workspace_mode.get(&state.store) {
        WorkspaceMode::Loading => Some(loading_card(state, theme)),
        WorkspaceMode::Ready if state.workspace.active_file.is_none() && !has_overlay => {
            if state.workspace.source == WorkspaceSource::Status {
                Some(status_ready_hint(theme, state.workspace.files.is_empty()))
            } else if state.compare.repo_path.is_some() {
                Some(repo_ready_hint(theme))
            } else {
                None
            }
        }
        WorkspaceMode::Empty if !has_overlay => {
            if state.compare.repo_path.is_some() {
                Some(repo_ready_hint(theme))
            } else {
                Some(empty_state(state, theme))
            }
        }
        _ => None,
    };

    view! { scale,
        <div class="flex-1 flex-col h-full" min_h={0.0} bg={tc.editor_surface}>
            {?toolbar}
            {?search}
            {?viewport_canvas}
            {?content}
        </div>
    }
}

fn viewport_toolbar(state: &AppState, theme: &Theme, file_label: &str) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let has_active_diff = state.workspace.active_file.is_some();
    let selected_scope = state.workspace.selected_status_scope;
    let show_stage = matches!(
        selected_scope,
        Some(
            crate::core::vcs::git::StatusScope::Unstaged
                | crate::core::vcs::git::StatusScope::Untracked
        )
    );
    let show_unstage = matches!(
        selected_scope,
        Some(crate::core::vcs::git::StatusScope::Staged)
    );
    let show_discard = selected_scope.is_some();

    view! { scale,
        <div class="w-full flex-row items-center"
             h={theme.metrics.ui_row_height.round()}
             px={Sp::MD} border_b={tc.border_variant}>
            <div class="flex-row items-center flex-1" gap={Sp::SM} min_w={0.0}>
                {components::file_icon(file_label, Ico::SM)}
                <div class="flex-1" min_w={0.0}>
                    <text class="text-sm truncate" color={tc.text_muted}>{file_label}</text>
                </div>
            </div>
            <div class="flex-row items-center" gap={Sp::SM}>
                if has_active_diff {
                    {SegmentedControl::new(vec![
                        SegmentedItem::new(
                            "Split",
                            Action::SetLayoutMode(LayoutMode::Split),
                            state.compare.layout == LayoutMode::Split,
                        ).tooltip("Side-by-side view"),
                        SegmentedItem::new(
                            "Unified",
                            Action::SetLayoutMode(LayoutMode::Unified),
                            state.compare.layout == LayoutMode::Unified,
                        ).tooltip("Inline view"),
                    ])}
                }
                if has_active_diff {
                    <Button action={Action::ToggleWrap}
                            active={state.editor.wrap_enabled}
                            tooltip={"Toggle line wrapping (w)"}>
                        <Icon>{lucide::WRAP_TEXT}</Icon>
                        <Label>{"Wrap"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && state.editor.line_selection.is_empty()
                    && show_stage
                {
                    <Button action={Action::StageSelectedFile}
                            style={ButtonStyle::Subtle}
                            tooltip={"Stage selected file"}>
                        <Icon>{lucide::PLUS}</Icon>
                        <Label>{"Stage"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && state.editor.line_selection.is_empty()
                    && show_unstage
                {
                    <Button action={Action::UnstageSelectedFile}
                            style={ButtonStyle::Subtle}
                            tooltip={"Unstage selected file"}>
                        <Icon>{lucide::MINUS}</Icon>
                        <Label>{"Unstage"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && state.editor.line_selection.is_empty()
                    && show_discard
                {
                    <Button action={Action::DiscardSelectedFile}
                            style={ButtonStyle::Danger}
                            tooltip={"Discard selected file changes"}>
                        <Icon>{lucide::CORNER_UP_LEFT}</Icon>
                        <Label>{"Discard"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && !state.editor.line_selection.is_empty()
                    && show_stage
                {
                    <Button action={Action::StageSelectedLines}
                            style={ButtonStyle::Subtle}
                            tooltip={"Stage selected lines (s)"}>
                        <Icon>{lucide::PLUS}</Icon>
                        <Label>{"Stage Lines"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && !state.editor.line_selection.is_empty()
                    && show_unstage
                {
                    <Button action={Action::UnstageSelectedLines}
                            style={ButtonStyle::Subtle}
                            tooltip={"Unstage selected lines (S)"}>
                        <Icon>{lucide::MINUS}</Icon>
                        <Label>{"Unstage Lines"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && !state.editor.line_selection.is_empty()
                    && show_discard
                {
                    <Button action={Action::DiscardSelectedLines}
                            style={ButtonStyle::Danger}
                            tooltip={"Discard selected lines (x)"}>
                        <Icon>{lucide::CORNER_UP_LEFT}</Icon>
                        <Label>{"Discard Lines"}</Label>
                    </Button>
                }
                if state.workspace.source == WorkspaceSource::Status
                    && !state.editor.line_selection.is_empty()
                {
                    <Button action={Action::ClearLineSelection}
                            tooltip={"Clear line selection"}>
                        <Icon>{lucide::X}</Icon>
                    </Button>
                }
            </div>
        </div>
    }
}

fn search_bar(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let search = &state.editor.search;
    let search_focused = state.focus.get(&state.store) == Some(FocusTarget::SearchInput);

    let input = text_input("", &search.query)
        .placeholder("Find in diff\u{2026}")
        .focused(search_focused)
        .focus_target(FocusTarget::SearchInput)
        .cursor(state.text_edit.cursor)
        .anchor(state.text_edit.anchor)
        .cursor_moved_at(state.text_edit.cursor_moved_at_ms)
        .on_click(Action::SetFocus(Some(FocusTarget::SearchInput)))
        .bare()
        .w_full()
        .h(theme.metrics.ui_row_height.round());

    let match_count = search.matches.len();
    let count_label = if search.query.is_empty() {
        String::new()
    } else if match_count == 0 {
        "No results".to_string()
    } else {
        let idx = search.active_index.map(|i| i + 1).unwrap_or(0);
        format!("{}/{}", idx, match_count)
    };

    let search_icon_size = (Ico::SM * scale).round();

    let nav = view! { scale,
        <div class="flex-row items-center" gap={Sp::XXS}>
            <div class="shrink-0">
                <text class="text-xs font-mono" color={tc.text_muted}>{&count_label}</text>
            </div>
            <Button action={Action::SearchPrevious}
                    tooltip={"Previous match"}
                    fixed_size={Sz::ROW}>
                <Icon>{lucide::CHEVRON_UP}</Icon>
            </Button>
            <Button action={Action::SearchNext}
                    tooltip={"Next match"}
                    fixed_size={Sz::ROW}>
                <Icon>{lucide::CHEVRON_DOWN}</Icon>
            </Button>
            <Button action={Action::CloseSearch}
                    tooltip={"Close search (Esc)"}
                    fixed_size={Sz::ROW}>
                <Icon>{lucide::X}</Icon>
            </Button>
        </div>
    };

    view! { scale,
        <div class="w-full flex-row items-center"
             h={theme.metrics.ui_row_height.round()}
             gap={Sp::SM} px={Sp::MD}
             border_b={tc.border_variant}
             bg={tc.editor_surface}>
            {svg_icon(lucide::SEARCH, search_icon_size).color(tc.text_muted)}
            <div class="flex-1" min_w={0.0}>
                {input}
            </div>
            {nav}
        </div>
    }
}

fn repo_ready_hint(theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="flex-1 items-center justify-center">
            <div class="flex-col items-center" gap={Sp::MD}>
                <icon svg={lucide::GIT_COMPARE} size={Ico::XXL}
                      color={tc.text_muted.with_alpha(Alpha::SOFT)} />
                <text class="text-sm" color={tc.text_muted}>
                    {"Select refs to compare"}
                </text>
            </div>
        </div>
    }
}

fn loading_card(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let (title, detail) = if state.workspace.source == WorkspaceSource::Status {
        (
            "Loading diff\u{2026}",
            state
                .workspace
                .selected_file_path
                .clone()
                .unwrap_or_else(|| "Working tree".to_owned()),
        )
    } else {
        (
            "Comparing repository\u{2026}",
            format!(
                "{} \u{2022} {} \u{2192} {}",
                compare_mode_label(state.compare.mode),
                display_ref(&state.compare.left_ref),
                display_ref(&state.compare.right_ref)
            ),
        )
    };

    view! { scale,
        <div class="flex-1 items-center justify-center" p={Sp::XL}>
            <div class="w-full flex-col items-center rounded-xl"
                 max_w={Sz::CARD_SM} p={Sp::XL} gap={Sp::MD}
                bg={tc.elevated_surface}
                border_b={tc.border}
                shadow_preset={Shadow::PANEL}>
                <icon svg={lucide::LOADER} size={Ico::XXL} color={tc.text_muted} />
                <div class="w-full" min_w={0.0}>
                    <text class="font-semibold text-center truncate" color={tc.text_strong}>
                        {title}
                    </text>
                </div>
                <div class="w-full" min_w={0.0}>
                    <text class="text-sm text-center truncate" color={tc.text_muted}>
                        {detail}
                    </text>
                </div>
            </div>
        </div>
    }
}

fn status_ready_hint(theme: &Theme, is_clean: bool) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let (icon, message) = if is_clean {
        (lucide::CHECK, "No uncommitted changes")
    } else {
        (lucide::FILE_DIFF, "Select a file to inspect changes")
    };

    view! { scale,
        <div class="flex-1 items-center justify-center">
            <div class="flex-col items-center" gap={Sp::MD}>
                <icon svg={icon} size={Ico::XXL}
                      color={tc.text_muted.with_alpha(Alpha::SOFT)} />
                <text class="text-sm" color={tc.text_muted}>
                    {message}
                </text>
            </div>
        </div>
    }
}

fn empty_state(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let recent_repos = crate::core::frecency::recent_repo_paths(state.frecency.as_ref(), 8);
    let has_recent = !recent_repos.is_empty();

    let recent_section: Option<AnyElement> = if has_recent {
        Some(view! { scale,
            <div class="flex-col" gap={Sp::XXS}>
                <text class="text-xs font-semibold" color={tc.text_muted}>{"Recent"}</text>
                for repo in recent_repos.iter().take(8) {
                    {repo_row(repo, tc, scale)}
                }
            </div>
        })
    } else {
        None
    };

    view! { scale,
        <div class="flex-1 items-center justify-center" p={Sp::XL}>
            <div class="w-full flex-col" max_w={Sz::CARD_LG}
                 p={Sp::XXL} gap={Sp::LG}
                 bg={tc.elevated_surface}
                 rounded={Rad::XXXL}
                 border_b={tc.border}
                 shadow_preset={Shadow::FLOAT}>
                <div class="flex-row items-center" gap={Sp::SM}>
                    <icon svg={lucide::GIT_COMPARE} size={Ico::XL} color={tc.accent} />
                    <text class="font-semibold" color={tc.text_strong}>{"diffy"}</text>
                </div>
                {?recent_section}
                <div pt={Sp::XS}>
                    <Button action={Action::OpenRepoPicker}
                            tooltip={"Open a repository folder"}
                            style={ButtonStyle::Subtle}>
                        <Icon>{lucide::FOLDER_OPEN}</Icon>
                        <Label>{"Open Folder"}</Label>
                    </Button>
                </div>
                <text class="text-xs" color={tc.text_muted}>{"or drop a folder here"}</text>
            </div>
        </div>
    }
}

fn repo_row(repo: &std::path::Path, tc: &crate::ui::theme::ThemeColors, scale: f32) -> AnyElement {
    let repo_name = repo
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let repo_path = repo.display().to_string();
    view! { scale,
        <div class="w-full flex-row items-center"
             py={Sp::SM} px={Sp::SM}
             rounded={Rad::MD} gap={Sp::SM}
             hover_bg={tc.sidebar_row_hover}
             on_click={Action::OpenRepository(repo.to_path_buf())}
             cursor={CursorHint::Pointer}>
            <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
            <div class="flex-col flex-1" min_w={0.0}>
                <text class="text-sm medium truncate" color={tc.text}>{repo_name}</text>
                <text class="text-xs truncate" color={tc.text_muted}>{repo_path}</text>
            </div>
        </div>
    }
}
