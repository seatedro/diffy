use std::cell::Cell;
use std::rc::Rc;

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

    let left = div()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_1()
        .min_w(0.0)
        .child(components::file_icon(file_label, Ico::SM))
        .child(div().w(Sp::XS))
        .child(
            div()
                .min_w(0.0)
                .flex_1()
                .child(text(file_label).text_sm().color(tc.text_muted).truncate()),
        );

    let right = div()
        .flex_row()
        .items_center()
        .gap_1()
        .child(SegmentedControl::new(vec![
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
        ]))
        .child(
            Button::new(Action::ToggleWrap)
                .icon(lucide::WRAP_TEXT)
                .label("Wrap")
                .active(state.editor.wrap_enabled),
        )
        .child(
            text(renderer_label(state.compare.renderer))
                .text_xs()
                .color(tc.text_muted),
        );

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

    let nav = div()
        .flex_row()
        .items_center()
        .gap(Sp::XXS * scale)
        .child(
            div()
                .flex_shrink_0()
                .child(
                    text(&count_label)
                        .text_xs()
                        .color(tc.text_muted)
                        .mono(),
                ),
        )
        .child(
            div()
                .w(nav_btn_size)
                .h(nav_btn_size)
                .items_center()
                .justify_center()
                .rounded(Rad::SM * scale)
                .hover_bg(tc.ghost_element_hover)
                .on_click(Action::SearchPrevious)
                .cursor(CursorHint::Pointer)
                .child(svg_icon(lucide::CHEVRON_UP, nav_icon_size).color(tc.text_muted)),
        )
        .child(
            div()
                .w(nav_btn_size)
                .h(nav_btn_size)
                .items_center()
                .justify_center()
                .rounded(Rad::SM * scale)
                .hover_bg(tc.ghost_element_hover)
                .on_click(Action::SearchNext)
                .cursor(CursorHint::Pointer)
                .child(svg_icon(lucide::CHEVRON_DOWN, nav_icon_size).color(tc.text_muted)),
        )
        .child(
            div()
                .w(nav_btn_size)
                .h(nav_btn_size)
                .items_center()
                .justify_center()
                .rounded(Rad::SM * scale)
                .hover_bg(tc.ghost_element_hover)
                .on_click(Action::CloseSearch)
                .cursor(CursorHint::Pointer)
                .child(svg_icon(lucide::X, nav_icon_size).color(tc.text_muted)),
        );

    let search_icon_size = (Ico::SM * scale).round();

    div()
        .w_full()
        .flex_row()
        .items_center()
        .gap(Sp::SM * scale)
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
        .child(
            div()
                .flex_col()
                .items_center()
                .gap(Sp::SM)
                .child(svg_icon(lucide::GIT_COMPARE, Ico::XXL).color(tc.text_muted.with_alpha(Alpha::SOFT)))
                .child(text("Select refs to compare").text_sm().color(tc.text_muted)),
        )
}

fn loading_card(state: &AppState, theme: &Theme) -> Div {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p(Sp::XL * scale)
        .child(
            div()
                .w_full()
                .max_w(Sz::CARD_SM * scale)
                .p(Sp::XL * scale)
                .flex_col()
                .gap(Sp::MD * scale)
                .items_center()
                .bg(tc.elevated_surface)
                .rounded_xl()
                .border_b(tc.border)
                .shadow_preset(Shadow::PANEL)
                .child(svg_icon(lucide::LOADER, Ico::XXL).color(tc.text_muted))
                .child(
                    div().w_full().min_w(0.0).child(
                        text("Comparing repository\u{2026}")
                            .semibold()
                            .text_center()
                            .color(tc.text_strong)
                            .truncate(),
                    ),
                )
                .child(
                    div().w_full().min_w(0.0).child(
                        text(format!(
                            "{} \u{2022} {} \u{2192} {}",
                            compare_mode_label(state.compare.mode),
                            display_ref(&state.compare.left_ref),
                            display_ref(&state.compare.right_ref)
                        ))
                        .text_sm()
                        .text_center()
                        .color(tc.text_muted)
                        .truncate(),
                    ),
                ),
        )
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
        .max_w(Sz::CARD_MD * scale)
        .p(Sp::XXL * scale)
        .flex_col()
        .gap(Sp::LG * scale)
        .bg(tc.elevated_surface)
        .rounded_xl()
        .border_b(tc.border)
        .shadow_preset(Shadow::FLOAT)
        .child(
            div()
                .flex_row()
                .items_center()
                .gap(Sp::SM * scale)
                .child(svg_icon(lucide::GIT_COMPARE, Ico::XL).color(tc.accent))
                .child(text("diffy").semibold().color(tc.text_strong)),
        );

    if has_recent {
        let mut recent_section = div()
            .flex_col()
            .gap(Sp::XXS);

        recent_section = recent_section.child(
            text("Recent")
                .text_xs()
                .semibold()
                .color(tc.text_muted),
        );

        for repo in recent_repos.iter().take(8) {
            let repo_name = repo
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let repo_path = repo.display().to_string();
            recent_section = recent_section.child(
                div()
                    .w_full()
                    .py(Sp::SM * scale)
                    .px(Sp::SM * scale)
                    .rounded(Rad::MD)
                    .flex_row()
                    .items_center()
                    .gap(Sp::SM * scale)
                    .hover_bg(tc.ghost_element_hover)
                    .on_click(Action::OpenRepository(repo.clone()))
                    .cursor(CursorHint::Pointer)
                    .child(svg_icon(lucide::FOLDER, Ico::SM).color(tc.text_muted))
                    .child(
                        div()
                            .flex_col()
                            .flex_1()
                            .min_w(0.0)
                            .child(text(repo_name).text_sm().medium().color(tc.text).truncate())
                            .child(text(repo_path).text_xs().color(tc.text_muted).truncate()),
                    ),
            );
        }

        card = card.child(recent_section);
    }

    card = card.child(
        div()
            .pt(Sp::XS * scale)
            .child(
                Button::new(Action::OpenRepositoryDialog)
                    .icon(lucide::FOLDER_OPEN)
                    .label("Open Folder")
                    .style(ButtonStyle::Subtle),
            ),
    );

    card = card.child(
        text("or drop a folder here")
            .text_xs()
            .color(tc.text_muted),
    );

    div()
        .flex_1()
        .items_center()
        .justify_center()
        .p(Sp::XL * scale)
        .child(card)
}
