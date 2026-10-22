use std::rc::Rc;
use std::time::Duration;

use diffy_ui::DiffEvent;
use diffy_ui::diff_view::decorator::{DiffDecorator, DiffViewOptions, LinePointer};
use quark::view;
use quark_app::ViewContext;
use quark_components::{Button, ButtonSize, ButtonStyle, CollectionEnv, DropdownItem, dropdown};
use quark_components::{kbd, search_field};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::text_input::{EditorMode, text_editor_element};
use quark_ui::theme::Theme;

use super::keys;
use super::state::{InteractionController, Phase};
use crate::app::commands::keymap::{self, workspace_context};
use crate::app::shell::{Slot, empty_state};
use crate::app::text_targets::{SEARCH, TEXT_COMPARE_LEFT, TEXT_COMPARE_RIGHT};
use crate::app::tokens::{self, Sp, TextStyle};
use crate::contracts::commands::CommandsAction;
use crate::contracts::shell::Action;
use crate::contracts::viewport::interaction::{
    DiffInput, InteractionAction, LineInput, TextCompareLanguage, TextCompareSide, TextCompareView,
};
use crate::core::keymap::ShortcutCommand;

const SYNTAX_POLL: Duration = Duration::from_millis(50);

pub fn view(
    interaction: &mut InteractionController,
    slot: &Slot,
    _decorator: &dyn DiffDecorator,
    cx: &mut ViewContext,
) -> AnyElement {
    render(interaction, slot, None, cx)
}

pub fn view_shared(
    interaction: &mut InteractionController,
    slot: &Slot,
    decorator: Rc<dyn DiffDecorator>,
    cx: &mut ViewContext,
) -> AnyElement {
    render(interaction, slot, Some(decorator), cx)
}

fn act(action: InteractionAction) -> quark_ui::Action {
    Action::from(action).into()
}

fn diff_event(event: DiffEvent) -> quark_ui::Action {
    act(InteractionAction::Diff(DiffInput(event)))
}

fn line_event(pointer: LinePointer) -> quark_ui::Action {
    act(InteractionAction::Line(LineInput(pointer)))
}

fn row_height(theme: &Theme) -> f32 {
    theme.metrics.ui_row_height
}

fn render(
    interaction: &mut InteractionController,
    slot: &Slot,
    decorator: Option<Rc<dyn DiffDecorator>>,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let (width, height) = (slot.width, slot.height);
    let text_compare = interaction.text_compare.active;
    let toolbar_h = if text_compare {
        (row_height(theme) + tokens::scaled(Sp::XS, tokens::ui_scale(theme)) * 2.0).min(height)
    } else {
        0.0
    };
    let body_h = (height - toolbar_h).max(0.0);
    let toolbar = text_compare.then(|| toolbar(interaction, width, toolbar_h, theme));
    let editing = text_compare && interaction.text_compare.view == TextCompareView::Edit;
    let decorator = interaction.decorations(decorator, theme);
    let body = if editing {
        text_compare_editors(interaction, width, body_h, cx)
    } else if slot.shared.repository.is_none()
        && !text_compare
        && matches!(interaction.phase, Phase::Idle)
        && interaction
            .diff
            .as_ref()
            .is_none_or(|diff| diff.session().is_empty())
    {
        welcome(slot, width, body_h, theme)
    } else {
        let reviewing = slot.shared.review.is_some();
        diff_area(interaction, width, body_h, Some(decorator), reviewing, cx)
    };
    let theme = cx.theme;
    view! {
        <div
            w={width}
            h={height}
            class="flex-col overflow-hidden"
            bg={theme.colors.editor_surface}
            accessibility_role={AccessibilityRole::Main}
            aria-label="Diff"
            test_id="diff"
        >
            {?toolbar}
            {body}
        </div>
    }
}

fn toolbar(
    interaction: &InteractionController,
    width: f32,
    height: f32,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let compact = ButtonSize::Compact;
    let toggle = |label: &'static str, active: bool, action: InteractionAction| {
        Button::new(act(action))
            .label(label)
            .size(compact)
            .active(active)
            .into_any()
    };
    let controls: Vec<AnyElement> = {
        let text = &interaction.text_compare;
        let editing = text.view == TextCompareView::Edit;
        let languages = TextCompareLanguage::OPTIONS
            .iter()
            .map(|&language| {
                let mut item = DropdownItem::new(
                    language.label(),
                    act(InteractionAction::SetTextCompareLanguage(language)),
                );
                item.selected = language == text.language;
                item
            })
            .collect();
        vec![
            toggle(
                "Edit",
                editing,
                InteractionAction::SetTextCompareView(TextCompareView::Edit),
            ),
            toggle(
                "Diff",
                !editing,
                InteractionAction::SetTextCompareView(TextCompareView::Diff),
            ),
            toggle("Swap sides", false, InteractionAction::SwapTextCompareSides),
            toggle(
                "Clear original",
                false,
                InteractionAction::ClearTextCompareSide(TextCompareSide::Left),
            ),
            toggle(
                "Clear changed",
                false,
                InteractionAction::ClearTextCompareSide(TextCompareSide::Right),
            ),
            dropdown(text.language.label(), languages)
                .open(text.language_menu)
                .on_toggle(act(InteractionAction::ToggleTextCompareLanguageMenu))
                .into_any(),
            Button::new(act(InteractionAction::CompareText))
                .label("Compare")
                .size(compact)
                .style(ButtonStyle::Filled)
                .into_any(),
        ]
    };
    view! {
        <div
            w={width}
            h={height}
            class="flex-row items-center shrink-0 overflow-hidden"
            px={tokens::scaled(Sp::SM, scale)}
            gap={tokens::scaled(Sp::XS, scale)}
            border_b={tc.border_variant}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Diff toolbar"
            test_id="diff.toolbar"
        >
            {...controls}
        </div>
    }
}

fn welcome(slot: &Slot, width: f32, height: f32, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &slot.shared.settings.settings.keymap_overrides;
    let hint = |command: ShortcutCommand, label: &'static str| {
        keymap::display_binding(saved, command).map(|binding| {
            view! {
                <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)}>
                    {kbd(binding, theme)}
                    <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{label}</text>
                </div>
            }
        })
    };
    view! {
        <div
            w={width}
            h={height}
            class="flex-col items-center justify-center"
            gap={tokens::scaled(Sp::LG, scale)}
            p={tokens::scaled(Sp::LG, scale)}
            test_id="diff.welcome"
        >
            <div class="flex-col items-center" gap={tokens::scaled(Sp::XS, scale)}>
                <text size={TextStyle::Heading.font_size(theme)} class="font-semibold" color={tc.text_strong}>
                    "Open a repository"
                </text>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>
                    "Review branches, commits and uncommitted changes."
                </text>
            </div>
            <Button
                on:click={Action::from(CommandsAction::OpenRepoPicker { anchored: false })}
                label="Open Repository…"
                icon={lucide::FOLDER_OPEN}
                variant={ButtonStyle::Filled}
            />
            <div class="flex-row items-center" gap={tokens::scaled(Sp::LG, scale)}>
                {?hint(ShortcutCommand::OpenRepository, "Open repository")}
                {?hint(ShortcutCommand::OpenCommandPalette, "All commands")}
            </div>
        </div>
    }
}

fn diff_area(
    interaction: &mut InteractionController,
    width: f32,
    height: f32,
    decorator: Option<Rc<dyn DiffDecorator>>,
    reviewing: bool,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let has_files = interaction
        .diff
        .as_ref()
        .is_some_and(|diff| !diff.session().is_empty());
    if !has_files {
        let (title, detail) = match &interaction.phase {
            Phase::Idle => (
                "Nothing to compare",
                "Open a repository to review its changes",
            ),
            Phase::Loading => ("Loading diff", "Comparing revisions"),
            Phase::Ready => ("No changes", "Nothing differs between these revisions"),
            Phase::Failed(message) => ("Couldn't load the diff", message.as_str()),
        };
        let state = empty_state(title, detail, theme);
        return view! { <div w={width} h={height} test_id="diff.empty">{state}</div> };
    }
    let scale = tokens::ui_scale(theme);
    let search_h = if interaction.search.open {
        (row_height(theme) + tokens::scaled(Sp::XS, scale) * 2.0).min(height)
    } else {
        0.0
    };
    let search = search_bar(interaction, width, search_h, cx);
    let banner = decorator
        .as_ref()
        .and_then(|d| d.banner(width))
        .map(|(h, element)| {
            let h = h.clamp(0.0, (height - search_h).max(0.0));
            let banner = div()
                .w(width)
                .h(h)
                .flex_shrink_0()
                .overflow_hidden()
                .test_id("diff.banner")
                .child(element);
            (h, banner)
        });
    let banner_h = banner.as_ref().map_or(0.0, |(h, _)| *h);
    let banner = banner.map(|(_, banner)| banner);
    let list_h = (height - search_h - banner_h).max(0.0);
    interaction.set_viewport(width, list_h);
    let style = interaction.style(cx.theme.metrics.mono_font_size);
    let options = DiffViewOptions {
        decorator,
        sources: Some(interaction.sources.clone()),
        keys: Some(keys::DIFF),
        on_line: Some(line_event),
    };
    let poll = !interaction.syntax.has_wake();
    let Some(diff) = interaction.diff.as_mut() else {
        return view! { <div w={width} h={height} /> };
    };
    diff.set_viewport(width, list_h);
    diff.set_style(style);
    let scale_factor = cx.frame.scale_factor();
    let now_ms = cx.frame.elapsed().as_millis() as u64;
    let text = cx.frame.text();
    diff.prepare(&mut text.system, &mut text.layouts, scale_factor, now_ms);
    if poll && diff.syntax_pending() {
        cx.frame.request_frame_in(SYNTAX_POLL);
    }
    let env = CollectionEnv {
        focused: cx.is_focused(crate::app::text_targets::VIEWPORT),
        accessible: cx.frame.accessibility_active(),
    };
    let list = diff.view_with_options(cx.theme, env, diff_event, options);
    let list_box = div()
        .w(width)
        .h(list_h)
        .key_context(workspace_context(reviewing))
        .on_key("escape", act(InteractionAction::ClearSelection))
        .test_id("diff.lines")
        .child(list);
    view! {
        <div w={width} h={height} class="flex-col">
            {search}
            {?banner}
            {list_box}
        </div>
    }
}

fn search_bar(
    interaction: &InteractionController,
    width: f32,
    height: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let focused = cx.is_focused(SEARCH);
    let open = interaction.search.open;
    let field = &interaction.search.field;
    let field_h = if open {
        (theme.metrics.ui_small_font_size * 1.5).round()
    } else {
        0.0
    };
    let input = text_input("Find in diff", "")
        .field(field)
        .placeholder("Find in diff")
        .focus_target(SEARCH)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h(field_h);
    let mut bar = div()
        .w(width)
        .h(height)
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .overflow_hidden()
        .test_id("diff.search");
    for (binding, action) in keys::search() {
        bar = bar.on_key(binding, act(action));
    }
    if !open {
        return bar.child(input).into_any();
    }
    let has_query = !field.text().is_empty();
    let summary = interaction.search_summary();
    let count = match (has_query, summary.matches, summary.active) {
        (false, ..) => String::new(),
        (true, 0, _) => "No matches".to_owned(),
        (true, n, active) => {
            let more = if summary.at_least { "+" } else { "" };
            match active {
                Some(i) => format!("{} of {n}{more}", i + 1),
                None => format!("{n}{more} matches"),
            }
        }
    };
    let compact = ButtonSize::Compact;
    let field = search_field(input, has_query, None, None, theme);
    let field = div()
        .flex_1()
        .min_w(0.0)
        .h_full()
        .flex_row()
        .items_center()
        .child(field);
    let count = view! {
        <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{count}</text>
    };
    let case = Button::new(act(InteractionAction::ToggleSearchCaseSensitive))
        .label("Match case")
        .size(compact)
        .active(interaction.search.case_sensitive);
    let previous = Button::new(act(InteractionAction::SearchPrevious))
        .label("Previous match")
        .size(compact);
    let next = Button::new(act(InteractionAction::SearchNext))
        .label("Next match")
        .size(compact);
    let close = Button::new(act(InteractionAction::CloseSearch))
        .label("Close search")
        .size(compact);
    bar.px(tokens::scaled(Sp::SM, scale))
        .gap(tokens::scaled(Sp::XS, scale))
        .border_b(tc.border_variant)
        .child(field)
        .child(count)
        .child(case)
        .child(previous)
        .child(next)
        .child(close)
        .into_any()
}

fn text_compare_editors(
    interaction: &mut InteractionController,
    width: f32,
    height: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let scale = tokens::ui_scale(theme);
    let pad = tokens::scaled(Sp::SM, scale);
    let label_h = row_height(theme);
    let pane_w = ((width - pad * 3.0) / 2.0).floor().max(0.0);
    let editor_h = (height - pad * 2.0 - label_h).max(0.0);
    let font_size = theme.metrics.mono_font_size;
    let now_ms = cx.frame.elapsed().as_millis() as u64;
    let text = cx.frame.text();
    for editor in [
        &mut interaction.text_compare.left,
        &mut interaction.text_compare.right,
    ] {
        editor.set_clock(now_ms);
        editor.set_font_size(font_size);
        editor.sync_size(pane_w, editor_h);
        editor.flush(&mut text.system);
    }
    let left = editor_pane(
        interaction,
        TextCompareSide::Left,
        pane_w,
        editor_h,
        label_h,
        cx,
    );
    let right = editor_pane(
        interaction,
        TextCompareSide::Right,
        pane_w,
        editor_h,
        label_h,
        cx,
    );
    view! {
        <div w={width} h={height} class="flex-row" p={pad} gap={pad} test_id="text-compare">
            {left}
            {right}
        </div>
    }
}

fn editor_pane(
    interaction: &InteractionController,
    side: TextCompareSide,
    width: f32,
    height: f32,
    label_h: f32,
    cx: &mut ViewContext,
) -> AnyElement {
    let theme = cx.theme;
    let tc = &theme.colors;
    let (focus, editor, title, placeholder, id) = match side {
        TextCompareSide::Left => (
            TEXT_COMPARE_LEFT,
            &interaction.text_compare.left,
            "Original",
            "Paste the original text",
            "text-compare.left",
        ),
        TextCompareSide::Right => (
            TEXT_COMPARE_RIGHT,
            &interaction.text_compare.right,
            "Changed",
            "Paste the changed text",
            "text-compare.right",
        ),
    };
    let scroll = match side {
        TextCompareSide::Left => ScrollActionBuilder::new(|lines| {
            act(InteractionAction::ScrollTextCompare(
                TextCompareSide::Left,
                lines,
            ))
        }),
        TextCompareSide::Right => ScrollActionBuilder::new(|lines| {
            act(InteractionAction::ScrollTextCompare(
                TextCompareSide::Right,
                lines,
            ))
        }),
    };
    let focused = cx.is_focused(focus);
    view! {
        <div w={width} class="flex-col" test_id={id}>
            <div h={label_h} class="flex-row items-center">
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>{title}</text>
            </div>
            <div
                w={width}
                h={height}
                class="overflow-hidden"
                bg={tc.background}
                border={if focused { tc.focus_border } else { tc.border_variant }}
                rounded={theme.metrics.control_radius}
            >
                <text_editor_element(focus, scroll)
                    editor_snapshot={editor}
                    mode={EditorMode::CodeInput}
                    label={format!("{title} text")}
                    placeholder={placeholder}
                    focused={focused}
                    font_size={theme.metrics.mono_font_size}
                    text_color={tc.text}
                    w={width}
                    h={height}
                />
            </div>
        </div>
    }
}
