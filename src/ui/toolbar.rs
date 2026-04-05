use std::cell::Cell;
use std::rc::Rc;

use halogen::view;

use crate::render::{Rect, TextMetrics};
use crate::ui::actions::Action;
use crate::ui::components::{self, Button, ButtonStyle, SegmentedControl, SegmentedItem};
use crate::ui::design::{Alpha, Ico, Rad, Shadow, Sp, Sz};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::state::{AppState, FocusTarget, WorkspaceMode};
use crate::ui::status_bar::{compare_mode_label, display_ref, renderer_label};
use crate::ui::style::Styled;
use crate::ui::theme::Theme;

use crate::core::compare::LayoutMode;

pub(crate) fn main_surface(
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
        .min_h(0.0)
        .bg(tc.editor_surface);

    let has_overlay = state.active_overlay_name().is_some();
    match state.workspace_mode {
        WorkspaceMode::Ready => {
            let file_label = state
                .workspace
                .selected_file_path
                .as_deref()
                .unwrap_or("No file selected");

            main = main.child(viewport_toolbar(state, theme, file_label));

            if state.editor.search.open {
                main = main.child(search_bar(state, theme));
            }

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
            if state.compare.repo_path.is_some() {
                main = main.child(repo_ready_hint(theme));
            } else {
                main = main.child(empty_state(state, theme));
            }
        }
        WorkspaceMode::Empty => {}
    }

    main
}

fn viewport_toolbar(state: &AppState, theme: &Theme, file_label: &str) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let left = view! { scale,
        <div class="flex-row items-center gap-1 flex-1" min_w={0.0}>
            {components::file_icon(file_label, Ico::SM)}
            <div w={Sp::XS} />
            <div class="flex-1" min_w={0.0}>
                <text class="text-sm truncate" color={tc.text_muted}>{file_label}</text>
            </div>
        </div>
    };

    let right = view! {
        <div class="flex-row items-center gap-1">
            {SegmentedControl::new(vec![
                SegmentedItem::new(
                    "Split",
                    Action::SetLayoutMode(LayoutMode::Split),
                    state.compare.layout == LayoutMode::Split,
                ),
                SegmentedItem::new(
                    "Unified",
                    Action::SetLayoutMode(LayoutMode::Unified),
                    state.compare.layout == LayoutMode::Unified,
                ),
            ])}
            {Button::new(Action::ToggleWrap)
                .icon(lucide::WRAP_TEXT)
                .label("Wrap")
                .active(state.editor.wrap_enabled)}
            <text class="text-xs" color={tc.text_muted}>
                {renderer_label(state.compare.renderer)}
            </text>
        </div>
    };

    div()
        .h((Sz::ROW * scale).round())
        .w_full()
        .flex_row()
        .items_center()
        .px((Sp::MD * scale).round())
        .border_b(tc.border_variant)
        .child(left)
        .child(right)
}

fn search_bar(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let search = &state.editor.search;
    let search_focused = state.focus.current == Some(FocusTarget::SearchInput);

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
        .h((Sz::SEARCH_INPUT * scale).round());

    let match_count = search.matches.len();
    let count_label = if search.query.is_empty() {
        String::new()
    } else if match_count == 0 {
        "No results".to_string()
    } else {
        let idx = search.active_index.map(|i| i + 1).unwrap_or(0);
        format!("{}/{}", idx, match_count)
    };

    let nav_icon_size = (Ico::SM * scale).round();
    let nav_btn_size = (Sz::SEARCH_INPUT * scale).round();
    let search_icon_size = (Ico::SM * scale).round();

    let nav = view! { scale,
        <div class="flex-row items-center" gap={Sp::XXS}>
            <div class="shrink-0">
                <text class="text-xs font-mono" color={tc.text_muted}>{&count_label}</text>
            </div>
            <div w={nav_btn_size} h={nav_btn_size}
                 class="items-center justify-center"
                 rounded={Rad::SM}
                 hover_bg={tc.ghost_element_hover}
                 on_click={Action::SearchPrevious}
                 cursor={CursorHint::Pointer}>
                <icon svg={lucide::CHEVRON_UP} size={nav_icon_size} color={tc.text_muted} />
            </div>
            <div w={nav_btn_size} h={nav_btn_size}
                 class="items-center justify-center"
                 rounded={Rad::SM}
                 hover_bg={tc.ghost_element_hover}
                 on_click={Action::SearchNext}
                 cursor={CursorHint::Pointer}>
                <icon svg={lucide::CHEVRON_DOWN} size={nav_icon_size} color={tc.text_muted} />
            </div>
            <div w={nav_btn_size} h={nav_btn_size}
                 class="items-center justify-center"
                 rounded={Rad::SM}
                 hover_bg={tc.ghost_element_hover}
                 on_click={Action::CloseSearch}
                 cursor={CursorHint::Pointer}>
                <icon svg={lucide::X} size={nav_icon_size} color={tc.text_muted} />
            </div>
        </div>
    };

    div()
        .w_full()
        .flex_row()
        .items_center()
        .gap((Sp::SM * scale).round())
        .px((Sp::MD * scale).round())
        .py((Sp::XS * scale).round())
        .border_b(tc.border_variant)
        .bg(tc.editor_surface)
        .child(svg_icon(lucide::SEARCH, search_icon_size).color(tc.text_muted))
        .child(div().flex_1().min_w(0.0).child(input))
        .child(nav)
}

fn repo_ready_hint(theme: &Theme) -> Div {
    let tc = &theme.colors;
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .child(view! {
            <div class="flex-col items-center gap-2">
                <icon svg={lucide::GIT_COMPARE} size={Ico::XXL}
                      color={tc.text_muted.with_alpha(Alpha::SOFT)} />
                <text class="text-sm" color={tc.text_muted}>
                    {"Select refs to compare"}
                </text>
            </div>
        })
}

fn loading_card(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();

    let refs_label = format!(
        "{} \u{2022} {} \u{2192} {}",
        compare_mode_label(state.compare.mode),
        display_ref(&state.compare.left_ref),
        display_ref(&state.compare.right_ref)
    );

    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p((Sp::XL * scale).round())
        .child(view! { scale,
            <div class="w-full flex-col items-center rounded-xl"
                 max_w={Sz::CARD_SM} p={Sp::XL} gap={Sp::MD}
                 bg={tc.elevated_surface}
                 border_b={tc.border}
                 shadow_preset={Shadow::PANEL}>
                <icon svg={lucide::LOADER} size={Ico::XXL} color={tc.text_muted} />
                <div class="w-full" min_w={0.0}>
                    <text class="font-semibold text-center truncate" color={tc.text_strong}>
                        {"Comparing repository\u{2026}"}
                    </text>
                </div>
                <div class="w-full" min_w={0.0}>
                    <text class="text-sm text-center truncate" color={tc.text_muted}>
                        {refs_label}
                    </text>
                </div>
            </div>
        })
}

fn empty_state(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let recent_repos = crate::core::frecency::recent_repo_paths(
        state.frecency.as_ref(),
        8,
    );
    let has_recent = !recent_repos.is_empty();

    let mut card = div()
        .w_full()
        .max_w((Sz::CARD_MD * scale).round())
        .p((Sp::XXL * scale).round())
        .flex_col()
        .gap((Sp::LG * scale).round())
        .bg(tc.elevated_surface)
        .rounded_xl()
        .border_b(tc.border)
        .shadow_preset(Shadow::FLOAT)
        .child(view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                <icon svg={lucide::GIT_COMPARE} size={Ico::XL} color={tc.accent} />
                <text class="font-semibold" color={tc.text_strong}>{"diffy"}</text>
            </div>
        });

    if has_recent {
        let mut recent_section = div()
            .flex_col()
            .gap(Sp::XXS);

        recent_section = recent_section.child(
            view! {
                <text class="text-xs font-semibold" color={tc.text_muted}>{"Recent"}</text>
            }
        );

        for repo in recent_repos.iter().take(8) {
            let repo_name = repo
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let repo_path = repo.display().to_string();
            recent_section = recent_section.child(view! { scale,
                <div class="w-full flex-row items-center"
                     py={Sp::SM} px={Sp::SM}
                     rounded={Rad::MD} gap={Sp::SM}
                     hover_bg={tc.ghost_element_hover}
                     on_click={Action::OpenRepository(repo.clone())}
                     cursor={CursorHint::Pointer}>
                    <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.text_muted} />
                    <div class="flex-col flex-1" min_w={0.0}>
                        <text class="text-sm medium truncate" color={tc.text}>{repo_name}</text>
                        <text class="text-xs truncate" color={tc.text_muted}>{repo_path}</text>
                    </div>
                </div>
            });
        }

        card = card.child(recent_section);
    }

    card = card.child(view! { scale,
        <div pt={Sp::XS}>
            {Button::new(Action::OpenRepoPicker)
                .icon(lucide::FOLDER_OPEN)
                .label("Open Folder")
                .style(ButtonStyle::Subtle)}
        </div>
    });

    card = card.child(view! {
        <text class="text-xs" color={tc.text_muted}>{"or drop a folder here"}</text>
    });

    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p((Sp::XL * scale).round())
        .child(card)
}
