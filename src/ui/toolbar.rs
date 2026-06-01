use std::cell::Cell;
use std::rc::Rc;

use halogen::{SemanticRole, view};

use crate::editor::input_element::text_editor_element;
use crate::render::{Rect, TextMetrics};
use crate::ui::components::{
    self, Button, ButtonSize, ButtonStyle, SegmentedControl, SegmentedItem,
};
use crate::ui::design::{Alpha, Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{
    AppState, AsyncStatus, FocusTarget, TextCompareLanguage, TextCompareSide, TextCompareView,
    WorkspaceMode, WorkspaceSource,
};
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
    if state.workspace.source.get(&state.store) == WorkspaceSource::TextCompare {
        return text_compare_surface(state, theme, viewport_bounds);
    }

    // Prefer the compare progress panel whenever a compare is in flight
    // AND the reveal delay has elapsed — sub-half-second diffs never
    // flash a loading state. Before reveal, fall through to whatever the
    // workspace was showing (old diff, empty state, etc.).
    let compare_progress_snapshot = state.compare_progress.with(&state.store, |p| p.clone());
    let progress_visible = compare_progress_snapshot
        .as_ref()
        .is_some_and(|p| state.clock_ms >= p.reveal_at_ms);

    let continuous_scroll = state.settings.continuous_scroll;
    let toolbar = if !progress_visible && state.is_workspace_ready() {
        if continuous_scroll {
            Some(viewport_toolbar(state, theme, None))
        } else {
            state
                .workspace
                .selected_file_path
                .get(&state.store)
                .map(|file_label| viewport_toolbar(state, theme, Some(&file_label)))
        }
    } else {
        None
    };

    let search = if !progress_visible
        && state.is_workspace_ready()
        && state.editor.search.open.get(&state.store)
    {
        Some(search_bar(state, theme))
    } else {
        None
    };

    let vb = viewport_bounds.clone();
    let viewport_canvas = if !progress_visible
        && state.is_workspace_ready()
        && (state
            .workspace
            .active_file
            .with(&state.store, |af| af.is_some())
            || (continuous_scroll && state.workspace_file_count() > 0))
    {
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

    let content = if progress_visible {
        let progress = compare_progress_snapshot.as_ref().unwrap();
        Some(crate::ui::components::compare_progress_panel(
            progress, state, theme,
        ))
    } else {
        match state.workspace_mode.get(&state.store) {
            // Loading mode is now always accompanied by a `compare_progress`
            // entry (either compare or repo-open). Reaching this arm means
            // the reveal delay hasn't elapsed — preserve the current view
            // instead of showing a placeholder.
            WorkspaceMode::Loading => None,
            WorkspaceMode::Ready if continuous_scroll && state.workspace_file_count() > 0 => None,
            WorkspaceMode::Ready
                if state
                    .workspace
                    .active_file
                    .with(&state.store, |af| af.is_none()) =>
            {
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status {
                    let no_files = state.workspace_file_count() == 0;
                    Some(status_ready_hint(theme, no_files))
                } else if state.compare.repo_path.with(&state.store, |p| p.is_some()) {
                    Some(repo_ready_hint(theme))
                } else {
                    None
                }
            }
            WorkspaceMode::Empty => {
                if state.compare.repo_path.with(&state.store, |p| p.is_some()) {
                    Some(repo_ready_hint(theme))
                } else {
                    Some(empty_state(state, theme))
                }
            }
            _ => None,
        }
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

fn viewport_toolbar(state: &AppState, theme: &Theme, file_label: Option<&str>) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let cx = &*state.store;
    let continuous_scroll = state.settings.continuous_scroll;
    let has_active_diff = state
        .workspace
        .active_file
        .with(&state.store, |af| af.is_some())
        || (continuous_scroll && state.workspace_file_count() > 0);
    let selected_bucket = state.workspace.selected_change_bucket.get(&state.store);
    let compare_layout = state.compare.layout.get(&state.store);
    let supports_staging = state
        .repository
        .capabilities
        .with(&state.store, |capabilities| {
            capabilities.is_some_and(|capabilities| capabilities.staging_area)
        });
    let supports_hunk_mutation = state
        .repository
        .capabilities
        .with(&state.store, |capabilities| {
            capabilities.is_some_and(|capabilities| capabilities.partial_hunk_mutation)
        });
    let show_stage = matches!(
        selected_bucket,
        Some(
            crate::core::vcs::model::ChangeBucket::Unstaged
                | crate::core::vcs::model::ChangeBucket::Untracked
        )
    ) && supports_staging;
    let show_unstage = matches!(
        selected_bucket,
        Some(crate::core::vcs::model::ChangeBucket::Staged)
    ) && supports_staging;
    let show_discard = selected_bucket.is_some() && supports_hunk_mutation;
    let file_label_view = file_label.map(|file_label| {
        view! { scale,
            <div class="flex-row items-center flex-1" gap={Sp::SM} min_w={0.0}>
                {components::file_icon(file_label, Ico::SM)}
                <div class="flex-1" min_w={0.0}>
                    <text class="text-sm truncate" color={tc.text_muted}>{file_label}</text>
                </div>
            </div>
        }
    });

    view! { scale,
        <div class="w-full flex-row items-center"
             h={theme.metrics.ui_row_height.round()}
             px={Sp::MD} border_b={tc.border_variant}>
            <div class="flex-row items-center flex-1" gap={Sp::SM} min_w={0.0}>
                {?file_label_view}
            </div>
            <div class="flex-row items-center" gap={Sp::SM}>
                if has_active_diff {
                    {SegmentedControl::new(vec![
                        SegmentedItem::new(
                            "Split",
                            crate::actions::CompareAction::SetLayoutMode(LayoutMode::Split).into(),
                            compare_layout == LayoutMode::Split,
                        ).tooltip("Side-by-side view"),
                        SegmentedItem::new(
                            "Unified",
                            crate::actions::CompareAction::SetLayoutMode(LayoutMode::Unified).into(),
                            compare_layout == LayoutMode::Unified,
                        ).tooltip("Inline view"),
                    ])}
                }
                if has_active_diff {
                    <Button action={crate::actions::SettingsAction::ToggleWrap.into()}
                            active={@state.editor.wrap_enabled}
                            tooltip={"Toggle line wrapping (w)"}>
                        <Icon>{lucide::WRAP_TEXT}</Icon>
                        <Label>{"Wrap"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_stage
                {
                    <Button action={crate::actions::RepositoryAction::StageSelectedFile.into()}
                            style={ButtonStyle::Subtle}
                            tooltip={"Stage selected file"}>
                        <Icon>{lucide::PLUS}</Icon>
                        <Label>{"Stage"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_unstage
                {
                    <Button action={crate::actions::RepositoryAction::UnstageSelectedFile.into()}
                            style={ButtonStyle::Subtle}
                            tooltip={"Unstage selected file"}>
                        <Icon>{lucide::MINUS}</Icon>
                        <Label>{"Unstage"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_discard
                {
                    <Button action={crate::actions::RepositoryAction::DiscardSelectedFile.into()}
                            style={ButtonStyle::Danger}
                            tooltip={"Discard selected file changes"}>
                        <Icon>{lucide::CORNER_UP_LEFT}</Icon>
                        <Label>{"Discard"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && !continuous_scroll
                    && !state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_stage
                {
                    <Button action={crate::actions::RepositoryAction::StageSelectedLines.into()}
                            style={ButtonStyle::Subtle}
                            tooltip={"Stage selected lines (s)"}>
                        <Icon>{lucide::PLUS}</Icon>
                        <Label>{"Stage Lines"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && !continuous_scroll
                    && !state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_unstage
                {
                    <Button action={crate::actions::RepositoryAction::UnstageSelectedLines.into()}
                            style={ButtonStyle::Subtle}
                            tooltip={"Unstage selected lines (S)"}>
                        <Icon>{lucide::MINUS}</Icon>
                        <Label>{"Unstage Lines"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && !continuous_scroll
                    && !state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                    && show_discard
                {
                    <Button action={crate::actions::RepositoryAction::DiscardSelectedLines.into()}
                            style={ButtonStyle::Danger}
                            tooltip={"Discard selected lines (x)"}>
                        <Icon>{lucide::CORNER_UP_LEFT}</Icon>
                        <Label>{"Discard Lines"}</Label>
                    </Button>
                }
                if state.workspace.source.get(&state.store) == WorkspaceSource::Status
                    && !continuous_scroll
                    && !state.editor.line_selection.with(&state.store, |ls| ls.is_empty())
                {
                    <Button action={crate::actions::RepositoryAction::ClearLineSelection.into()}
                            tooltip={"Clear line selection"}>
                        <Icon>{lucide::X}</Icon>
                    </Button>
                }
            </div>
        </div>
    }
}

fn text_compare_surface(
    state: &AppState,
    theme: &Theme,
    viewport_bounds: Rc<Cell<Option<Rect>>>,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let view = state.text_compare.view;
    let status = state.text_compare.status;
    let compare_disabled = status == AsyncStatus::Loading;
    let diff_file_label = state.workspace.selected_file_path.get(&state.store);

    let status_label = match status {
        AsyncStatus::Loading => "Comparing",
        AsyncStatus::Failed => "Failed",
        AsyncStatus::Ready if !state.text_compare_is_stale() => "Compared",
        _ => "Ready",
    };
    let status_color = match status {
        AsyncStatus::Failed => tc.status_error,
        AsyncStatus::Ready if !state.text_compare_is_stale() => tc.accent,
        _ => tc.text_muted,
    };

    let controls = if view == TextCompareView::Edit {
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                <Button action={crate::actions::TextCompareAction::SwapSides.into()}
                        size={ButtonSize::Compact}
                        tooltip={"Swap sides"}>
                    <Icon>{lucide::ARROW_LEFT_RIGHT}</Icon>
                    <Label>{"Swap"}</Label>
                </Button>
                <Button action={crate::actions::TextCompareAction::CompareNow.into()}
                        style={ButtonStyle::Filled}
                        size={ButtonSize::Compact}
                        disabled={compare_disabled}
                        tooltip={"Compare text"}>
                    <Icon>{lucide::GIT_COMPARE}</Icon>
                    <Label>{"Compare"}</Label>
                </Button>
            </div>
        }
    } else {
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                <Button action={crate::actions::TextCompareAction::SetView(TextCompareView::Edit).into()}
                        style={ButtonStyle::Filled}
                        size={ButtonSize::Compact}
                        tooltip={"Back to edit"}>
                    <Icon>{lucide::PENCIL}</Icon>
                    <Label>{"Edit"}</Label>
                </Button>
            </div>
        }
    };

    let top_bar = view! { scale,
        <div class="w-full flex-row items-center"
             h={theme.metrics.ui_row_height.round()}
             px={Sp::MD} border_b={tc.border_variant}
             bg={tc.editor_surface}>
            <div class="flex-row items-center flex-1" gap={Sp::SM} min_w={0.0}>
                <icon svg={lucide::FILE_DIFF} size={Ico::SM} color={status_color} />
                <text class="text-sm font-medium" color={status_color}>{status_label}</text>
            </div>
            {controls}
        </div>
    };

    let body = match view {
        TextCompareView::Edit => text_compare_edit_body(state, theme),
        TextCompareView::Diff => {
            let vb = viewport_bounds.clone();
            let viewport_canvas = state
                .workspace
                .active_file
                .with(&state.store, |af| af.is_some())
                .then(|| {
                    canvas(move |bounds, _scene, _cx| {
                        vb.set(Some(bounds));
                    })
                    .flex_1()
                    .into_any()
                });
            let hint = if viewport_canvas.is_none() {
                Some(text_compare_diff_hint(state, theme))
            } else {
                None
            };
            view! { scale,
                <div class="flex-1 flex-col" min_h={0.0}>
                    {viewport_toolbar(state, theme, diff_file_label.as_deref())}
                    {?viewport_canvas}
                    {?hint}
                </div>
            }
            .into_any()
        }
    };

    view! { scale,
        <div class="flex-1 flex-col h-full" min_h={0.0} bg={tc.editor_surface}>
            {top_bar}
            {body}
        </div>
    }
}

fn text_compare_edit_body(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let left_lines = state.text_compare.left_editor.line_count();
    let right_lines = state.text_compare.right_editor.line_count();
    let total_stats = state.workspace.compare_total_stats.get(&state.store);
    let stats_label = total_stats
        .map(|(additions, deletions)| format!("+{} -{}", additions, deletions))
        .unwrap_or_else(|| "\u{2014}".to_owned());

    view! { scale,
        <div class="flex-1 flex-row" min_h={0.0}>
            <div class="flex-col shrink-0"
                   w={(220.0 * scale).round()}
                   p={Sp::MD}
                   gap={Sp::LG}
                   bg={tc.sidebar_background}
                   border_r={tc.border_variant}>
                <div class="flex-col" gap={Sp::SM}>
                    {text_compare_stat_row("Original", &format!("{} lines", left_lines), tc, scale)}
                    {text_compare_stat_row("Changed", &format!("{} lines", right_lines), tc, scale)}
                    {text_compare_stat_row("Language", &text_compare_language_label(state), tc, scale)}
                    {text_compare_stat_row("Last diff", &stats_label, tc, scale)}
                </div>
                {text_compare_language_picker(state, theme)}
            </div>
            <div class="flex-1 flex-row" min_w={0.0} min_h={0.0} gap={1.0} bg={tc.border_variant}>
                {text_compare_editor_pane(
                    state,
                    theme,
                    TextCompareSide::Left,
                    FocusTarget::TextCompareLeft,
                    "Original",
                )}
                {text_compare_editor_pane(
                    state,
                    theme,
                    TextCompareSide::Right,
                    FocusTarget::TextCompareRight,
                    "Changed",
                )}
            </div>
        </div>
    }
}

fn text_compare_language_label(state: &AppState) -> String {
    match (
        state.text_compare.language,
        state.text_compare.detected_language,
    ) {
        (TextCompareLanguage::Auto, Some(detected)) => format!("Auto: {}", detected.label()),
        (language, _) => language.label().to_owned(),
    }
}

fn text_compare_language_picker(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    view! { scale,
        <div class="flex-col" gap={Sp::XS}>
            <text class="text-xs font-medium" color={tc.text_muted}>{"Language"}</text>
            <div class="flex-col" gap={Sp::XXS}>
                <div class="flex-row" gap={Sp::XXS}>
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Auto)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Rust)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::TypeScript)}
                </div>
                <div class="flex-row" gap={Sp::XXS}>
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Python)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::JavaScript)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Json)}
                </div>
                <div class="flex-row" gap={Sp::XXS}>
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Shell)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Toml)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Nix)}
                </div>
                <div class="flex-row" gap={Sp::XXS}>
                    {text_compare_language_chip(state, theme, TextCompareLanguage::C)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Cpp)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Go)}
                </div>
                <div class="flex-row" gap={Sp::XXS}>
                    {text_compare_language_chip(state, theme, TextCompareLanguage::Zig)}
                    {text_compare_language_chip(state, theme, TextCompareLanguage::PlainText)}
                </div>
            </div>
        </div>
    }
}

fn text_compare_language_chip(
    state: &AppState,
    theme: &Theme,
    language: TextCompareLanguage,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let selected = state.text_compare.language == language;
    view! { scale,
        <div class="flex-1 items-center justify-center"
             min_w={0.0}
             px={Sp::XS} py={Sp::XXS}
             rounded={Rad::SM}
             bg={if selected { tc.ghost_element_selected } else { tc.element_background }}
             border={if selected { tc.accent } else { tc.border_variant }}
             hover_bg={tc.ghost_element_hover}
             semantic_role={SemanticRole::RadioButton}
             accessibility_role={accesskit::Role::RadioButton}
             accessibility_id={format!("text-compare-language:{}", language.label())}
             accessibility_label={language.label()}
             accessibility_selected={selected}
             accessibility_toggled={selected}
             on_click={crate::actions::TextCompareAction::SetLanguage(language).into()}
             cursor={CursorHint::Pointer}>
            <text class="text-xs font-medium"
                  color={if selected { tc.text_strong } else { tc.text_muted }}>
                {language.short_label()}
            </text>
        </div>
    }
}

fn text_compare_stat_row(
    label: &str,
    value: &str,
    tc: &crate::ui::theme::ThemeColors,
    scale: f32,
) -> AnyElement {
    view! { scale,
        <div class="flex-row items-center" gap={Sp::SM}>
            <text class="text-xs" color={tc.text_muted}>{label}</text>
            <spacer />
            <text class="text-xs font-mono" color={tc.text}>{value}</text>
        </div>
    }
}

fn text_compare_editor_pane(
    state: &AppState,
    theme: &Theme,
    side: TextCompareSide,
    focus_target: FocusTarget,
    label: &'static str,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let focused = state.focus.get(&state.store) == Some(focus_target);
    let editor = match side {
        TextCompareSide::Left => &state.text_compare.left_editor,
        TextCompareSide::Right => &state.text_compare.right_editor,
    };
    let line_label = format!("{} lines", editor.line_count());
    let accent = match side {
        TextCompareSide::Left => tc.status_error,
        TextCompareSide::Right => tc.line_add_text,
    };
    let editor_el = text_editor_element()
        .placeholder(label)
        .editor_snapshot(editor)
        .focused(focused)
        .focus_target(focus_target)
        .font_size(theme.metrics.mono_font_size)
        .text_color(tc.text)
        .w_full()
        .flex_1();

    view! { scale,
        <div class="flex-1 flex-col" min_w={0.0} min_h={0.0} bg={tc.editor_surface}>
            <div class="flex-row items-center shrink-0"
                    h={theme.metrics.ui_row_height.round()}
                    px={Sp::MD}
                    gap={Sp::SM}
                    border_b={tc.border_variant}>
                <div w={(3.0 * scale).round()} h={(18.0 * scale).round()} rounded={Rad::SM} bg={accent} />
                <text class="text-sm font-medium" color={tc.text}>{label}</text>
                <text class="text-xs" color={tc.text_muted}>{line_label}</text>
                <spacer />
                <Button action={crate::actions::TextCompareAction::ClearSide(side).into()}
                        size={ButtonSize::Compact}
                        tooltip={format!("Clear {}", label.to_lowercase())}>
                    <Icon>{lucide::X}</Icon>
                </Button>
            </div>
            <div class="flex-1 w-full" min_h={0.0}
                 px={Sp::MD} py={Sp::SM}
                 border={if focused { tc.accent } else { crate::ui::theme::Color::TRANSPARENT }}
                 on_click={crate::actions::AppAction::SetFocus(Some(focus_target)).into()}
                 cursor={CursorHint::Text}>
                {editor_el}
            </div>
        </div>
    }
}

fn text_compare_diff_hint(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let label = if state.text_compare.status == AsyncStatus::Ready
        && !state.text_compare_is_stale()
        && state.workspace_file_count() == 0
    {
        "No differences"
    } else {
        "No diff yet"
    };
    view! { scale,
        <div class="flex-1 items-center justify-center" p={Sp::XL}>
            <div class="flex-col items-center" gap={Sp::SM}>
                <icon svg={lucide::FILE_DIFF} size={Ico::XL} color={tc.text_muted} />
                <text class="text-sm font-medium" color={tc.text_muted}>{label}</text>
            </div>
        </div>
    }
}

fn search_bar(state: &AppState, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let search_query = state.editor.search.query.with(&state.store, |s| s.clone());
    let match_count = state.editor.search.matches.with(&state.store, |m| m.len());
    let active_index = state.editor.search.active_index.get(&state.store);
    let search_focused = state.focus.get(&state.store) == Some(FocusTarget::SearchInput);

    let input = text_input("", &search_query)
        .placeholder("Find in diff\u{2026}")
        .focused(search_focused)
        .focus_target(FocusTarget::SearchInput)
        .cursor(state.text_edit.cursor.get(&state.store))
        .anchor(state.text_edit.anchor.get(&state.store))
        .cursor_moved_at(state.text_edit.cursor_moved_at_ms.get(&state.store))
        .on_click(crate::actions::Action::from(
            crate::actions::AppAction::SetFocus(Some(FocusTarget::SearchInput)),
        ))
        .bare()
        .w_full()
        .h(theme.metrics.ui_row_height.round());

    let count_label = if search_query.is_empty() {
        String::new()
    } else if match_count == 0 {
        "No results".to_string()
    } else {
        let idx = active_index.map(|i| i + 1).unwrap_or(0);
        format!("{}/{}", idx, match_count)
    };

    let search_icon_size = (Ico::SM * scale).round();

    let nav = view! { scale,
        <div class="flex-row items-center" gap={Sp::XXS}>
            <div class="shrink-0">
                <text class="text-xs font-mono" color={tc.text_muted}>{&count_label}</text>
            </div>
            <Button action={crate::actions::EditorAction::SearchPrevious.into()}
                    tooltip={"Previous match"}
                    fixed_size={Sz::ROW}>
                <Icon>{lucide::CHEVRON_UP}</Icon>
            </Button>
            <Button action={crate::actions::EditorAction::SearchNext.into()}
                    tooltip={"Next match"}
                    fixed_size={Sz::ROW}>
                <Icon>{lucide::CHEVRON_DOWN}</Icon>
            </Button>
            <Button action={crate::actions::EditorAction::CloseSearch.into()}
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

    let panel_max_w = (Sz::CARD_LG * scale).round();

    view! { scale,
        <div class="flex-1 items-center justify-center" p={Sp::XL}>
            <div class="w-full flex-col" max_w={panel_max_w}
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
                <div class="flex-row items-center" pt={Sp::XS} gap={Sp::SM}>
                    <Button action={crate::actions::OverlayAction::OpenRepoPicker.into()}
                            tooltip={"Open a repository folder"}
                            style={ButtonStyle::Subtle}>
                        <Icon>{lucide::FOLDER_OPEN}</Icon>
                        <Label>{"Open Folder"}</Label>
                    </Button>
                    <Button action={crate::actions::WorkspaceAction::NewTextCompare.into()}
                            tooltip={"Compare pasted text"}
                            style={ButtonStyle::Subtle}>
                        <Icon>{lucide::FILE_DIFF}</Icon>
                        <Label>{"Compare Text"}</Label>
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
    let accessibility_label = format!("{repo_name}, {repo_path}");
    view! { scale,
        <div class="w-full flex-row items-center"
             id={format!("recent-repo:{repo_path}")}
             key={repo_path.clone()}
             test_id={"recent-repo-row"}
             semantic_role={SemanticRole::Button}
             py={Sp::SM} px={Sp::SM}
             rounded={Rad::MD} gap={Sp::SM}
             hover_bg={tc.sidebar_row_hover}
             on_click={crate::actions::WorkspaceAction::OpenRepository(repo.to_path_buf()).into()}
             accessibility_role={accesskit::Role::Button}
             accessibility_id={format!("recent-repo:{repo_path}")}
             accessibility_label={accessibility_label}
             cursor={CursorHint::Pointer}>
            <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
            <div class="flex-col flex-1" min_w={0.0}>
                <text class="text-sm medium truncate" color={tc.text}>{repo_name}</text>
                <text class="text-xs truncate" color={tc.text_muted}>{repo_path}</text>
            </div>
        </div>
    }
}
