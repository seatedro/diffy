use quark::view;
use quark_app::ViewContext;
use quark_components::{SegmentedControl, SegmentedItem, kbd};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::CommandsController;
use super::list;
use super::ref_picker::{REF_PICKER_VISIBLE_ROWS, RefPicker};
use crate::app::icons;
use crate::app::text_targets::PICKER;
use crate::app::tokens::{self, Ico, Layout, Rad, Sp, Sz, TextStyle};
use crate::contracts::commands::{CommandsAction, RefPickerAction};
use crate::contracts::navigation::CompareField;
use crate::contracts::shell::Action;
use crate::core::compare::{CompareMode, RendererKind};
use crate::core::ref_catalog::{RefEntry, RefEntryKind, RefRow, relative_age};

fn act(action: RefPickerAction) -> quark_ui::Action {
    Action::from(CommandsAction::RefPicker(action)).into()
}

pub fn kind_icon(kind: RefEntryKind) -> &'static str {
    match kind {
        RefEntryKind::WorkingTree => lucide::PENCIL,
        RefEntryKind::Branch | RefEntryKind::RemoteBranch => lucide::GIT_BRANCH,
        RefEntryKind::Tag => icons::TAG,
        RefEntryKind::Commit => icons::COMMIT,
    }
}

pub fn ref_picker_panel(
    commands: &mut CommandsController,
    size: (f32, f32),
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let focused = cx.is_focused(PICKER);
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let stride = list::row_stride(theme);
    let picker = commands.ref_picker_mut()?;
    picker.reveal_pending(stride);
    let picker = &*picker;
    let width = tokens::scaled(Layout::REF_PICKER_W, scale)
        .min(size.0 - tokens::scaled(Sp::LG, scale) * 2.0)
        .max(1.0);
    let top = theme.metrics.title_bar_height + tokens::scaled(Layout::POPOVER_GAP, scale);
    let left = ((size.0 - width) / 2.0).max(0.0);
    let side = picker.side();
    let sides = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Base",
            act(RefPickerAction::SetSide(CompareField::Left)),
            side == CompareField::Left,
        ),
        SegmentedItem::new(
            "Head",
            act(RefPickerAction::SetSide(CompareField::Right)),
            side == CompareField::Right,
        ),
    ])
    .id("ref-picker.side");
    let input = text_input("Search refs", "")
        .field(&picker.query)
        .placeholder("Search refs or type a revision")
        .focus_target(PICKER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h(tokens::scaled(Sz::INPUT, scale));
    let body = rows(picker, theme);
    let footer = footer(picker, theme);
    let card = hover_card(picker, theme).map(|card| {
        let selected = picker.selected.unwrap_or(0) as f32;
        let header_h = tokens::scaled(Sz::INPUT, scale);
        let y = top + header_h + (selected * stride - picker.scroll_px).max(0.0);
        let x = left + width + tokens::scaled(Sp::MD, scale);
        let card_w = tokens::scaled(Layout::HOVER_CARD_W, scale);
        let fits = x + card_w <= size.0;
        view! {
            <div class="absolute" left={x} top={y.min(size.1 - stride * 4.0).max(top)} w={card_w} z_index={320}>
                if fits { {card} }
            </div>
        }
    });
    Some(view! {
        <div
            class="absolute top-0 left-0"
            w={size.0}
            h={size.1}
            z_index={300}
            bg={tc.overlay_scrim}
            test_id="ref-picker-backdrop"
            on:click={CommandsAction::BackdropDismiss}
            block_mouse
        >
            <div
                class="absolute flex-col overflow-hidden"
                left={left}
                top={top}
                w={width}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={tokens::scaled(Rad::XL, scale)}
                on:click={NoopAction}
                test_id="ref-picker"
                focus_scope="diffy.ref-picker"
                trap_focus={true}
                accessibility_role={AccessibilityRole::Dialog}
                aria-label={if side == CompareField::Left { "Base ref" } else { "Head ref" }}
            >
                <div
                    class="flex-row items-center"
                    gap={tokens::scaled(Sp::SM, scale)}
                    px={tokens::scaled(Sp::MD, scale)}
                    border_b={tc.border_variant}
                >
                    <div class="shrink-0">{sides}</div>
                    <icon svg={lucide::SEARCH} size={Ico::SM} color={tc.text_muted} />
                    <div class="flex-1 min-w-0">{input}</div>
                    <div class="flex-row items-center shrink-0" gap={tokens::scaled(Sp::XS, scale)}>
                        {kbd("\u{21E5}", theme)}
                        <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"swap side"</text>
                    </div>
                </div>
                <div p={tokens::scaled(Sp::XS, scale)}>{body}</div>
                {footer}
            </div>
            {?card}
        </div>
    })
}

fn rows(picker: &RefPicker, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let Some(catalog) = picker.catalog.as_ref() else {
        return status_line("Loading refs\u{2026}", theme);
    };
    let results = &picker.results;
    if results.rows.is_empty() {
        let typed = picker.query.text().trim();
        if typed.is_empty() {
            return status_line("No refs", theme);
        }
        return list::option_row(
            list::OptionRow {
                id: "ref-picker.typed".to_owned(),
                label: typed,
                highlights: &[],
                detail: Some("Use this revision"),
                rhs: None,
                icon: Some(icons::COMMIT),
                selected: true,
                current: false,
            },
            theme,
            Action::from(CommandsAction::ConfirmOverlaySelection).into(),
        );
    }
    let querying = !picker.query.text().trim().is_empty();
    let current = picker.current_name();
    list::virtual_list(
        results.rows.len(),
        picker.scroll_px,
        REF_PICKER_VISIBLE_ROWS,
        theme,
        "Refs",
        |row| match &results.rows[row] {
            RefRow::Section { section, count } => {
                let label = if querying {
                    format!("{} \u{00B7} {count} match", section.label())
                } else {
                    format!("{} \u{00B7} {count}", section.label())
                };
                view! {
                    <div
                        class="w-full flex-row items-end"
                        h={theme.metrics.ui_row_height.round()}
                        px={tokens::scaled(Sp::SM, scale)}
                        pb={tokens::scaled(Sp::XXS, scale)}
                        accessibility_role={AccessibilityRole::Heading}
                        aria-label={label.clone()}
                    >
                        <text size={TextStyle::Caption.font_size(theme)} class="font-semibold" color={tc.text_muted}>{label.to_uppercase()}</text>
                    </div>
                }
            }
            RefRow::Entry { index, ranges } => {
                let entry = &catalog.entries[*index as usize];
                let age = relative_age(picker.now, entry.timestamp);
                let detail = if entry.subject.is_empty() {
                    None
                } else {
                    Some(entry.subject.as_str())
                };
                let selected = picker.selected == Some(row);
                let is_current = entry.name == current;
                let id = format!("ref:{}:{}", section_key(entry), entry.name);
                view! {
                    <div class="w-full flex-row items-center">
                        {list::option_row(
                            list::OptionRow {
                                id,
                                label: &entry.name,
                                highlights: ranges,
                                detail,
                                rhs: Some(&age),
                                icon: Some(kind_icon(entry.kind)),
                                selected,
                                current: is_current,
                            },
                            theme,
                            act(RefPickerAction::Choose(row)),
                        )}
                    </div>
                }
            }
        },
        ScrollActionBuilder::new(|lines| Action::from(CommandsAction::ScrollOverlay(lines)).into()),
    )
}

fn section_key(entry: &RefEntry) -> &'static str {
    entry.section.label()
}

fn status_line(message: &str, theme: &Theme) -> AnyElement {
    let scale = tokens::ui_scale(theme);
    view! {
        <div class="flex-row items-center" h={theme.metrics.ui_row_height} px={tokens::scaled(Sp::MD, scale)}>
            <text size={TextStyle::BodySmall.font_size(theme)} color={theme.colors.text_muted}>{message}</text>
        </div>
    }
}

fn footer(picker: &RefPicker, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let mode = picker.request.mode;
    let renderer = picker.request.renderer;
    let modes = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Since merge base",
            act(RefPickerAction::SetMode(CompareMode::ThreeDot)),
            !picker.request.working_copy && mode == CompareMode::ThreeDot,
        ),
        SegmentedItem::new(
            "Directly",
            act(RefPickerAction::SetMode(CompareMode::TwoDot)),
            !picker.request.working_copy && mode == CompareMode::TwoDot,
        ),
    ])
    .id("ref-picker.mode");
    let engines = SegmentedControl::new(vec![
        SegmentedItem::new(
            "Git",
            act(RefPickerAction::SetEngine(RendererKind::Builtin)),
            renderer == RendererKind::Builtin,
        ),
        SegmentedItem::new(
            "Difftastic",
            act(RefPickerAction::SetEngine(RendererKind::Difftastic)),
            renderer == RendererKind::Difftastic,
        ),
    ])
    .id("ref-picker.engine");
    view! {
        <div
            class="flex-row items-center w-full"
            gap={tokens::scaled(Sp::SM, scale)}
            px={tokens::scaled(Sp::MD, scale)}
            py={tokens::scaled(Sp::SM, scale)}
            border_t={tc.border_variant}
            bg={tc.panel}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Compare options"
        >
            <text size={small} color={tc.text_muted}>"Compare"</text>
            <div class="shrink-0">{modes}</div>
            <div class="flex-1" />
            <text size={small} color={tc.text_muted}>"Engine"</text>
            <div class="shrink-0">{engines}</div>
        </div>
    }
}

fn hover_card(picker: &RefPicker, theme: &Theme) -> Option<AnyElement> {
    let entry = picker.selected_entry()?;
    if entry.kind == RefEntryKind::WorkingTree {
        return None;
    }
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let mono = TextStyle::MonoSmall.font_size(theme);
    let age = relative_age(picker.now, entry.timestamp);
    let byline = match (entry.author.is_empty(), age.is_empty()) {
        (false, false) => format!("{} \u{00B7} {age} ago", entry.author),
        (false, true) => entry.author.clone(),
        (true, false) => format!("{age} ago"),
        (true, true) => String::new(),
    };
    let other = match picker.other_name() {
        "" | crate::core::ref_catalog::WORKING_TREE_LABEL => "HEAD",
        name => name,
    };
    let range = match picker.side() {
        CompareField::Left => format!("{} \u{2192} {other}", entry.name),
        CompareField::Right => format!("{other} \u{2192} {}", entry.name),
    };
    let size_line = picker.size.map(|size| {
        let plural =
            |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let files = size
            .files
            .map(|f| format!("{} \u{00B7} ", plural(f, "file", "files")))
            .unwrap_or_default();
        let plus = if size.commits_capped { "+" } else { "" };
        (
            format!("+{}", size.additions),
            format!("\u{2212}{}", size.deletions),
            format!("{files}{}{plus}", plural(size.commits, "commit", "commits")),
        )
    });
    Some(view! {
        <div
            class="flex-col w-full"
            gap={tokens::scaled(Sp::SM, scale)}
            p={tokens::scaled(Sp::MD, scale)}
            bg={tc.elevated_surface}
            border={tc.border}
            rounded={tokens::scaled(Rad::XL, scale)}
            accessibility_role={AccessibilityRole::Group}
            aria-label={format!("{} details", entry.name)}
            test_id="ref-picker.card"
        >
            <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::SM, scale)}>
                <icon svg={kind_icon(entry.kind)} size={Ico::SM} color={tc.text_muted} />
                <text size={mono} class="font-semibold truncate" color={tc.text_strong}>{entry.name.clone()}</text>
                <text size={mono} color={tc.text_muted}>{entry.short_oid().to_owned()}</text>
                <div class="flex-1" />
                <div
                    class="flex-row items-center justify-center"
                    w={theme.metrics.ui_row_height}
                    h={theme.metrics.ui_row_height}
                    rounded={theme.metrics.control_radius}
                    hover_bg={tc.ghost_element_hover}
                    cursor={CursorHint::Pointer}
                    id="ref-picker.copy-sha"
                    role="button"
                    aria-label="Copy SHA"
                    on:click={act(RefPickerAction::CopySha)}
                >
                    <icon svg={lucide::COPY} size={Ico::XS} color={tc.icon} />
                </div>
            </div>
            if !entry.subject.is_empty() {
                <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={tc.text}>{entry.subject.clone()}</text>
            }
            if !byline.is_empty() {
                <text size={small} color={tc.text_muted}>{byline}</text>
            }
            <div class="w-full" h={1.0} bg={tc.border_variant} />
            <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::SM, scale)}>
                <text size={small} class="truncate" color={tc.text_muted}>{range}</text>
                <div class="flex-1" />
                {?size_line.as_ref().map(|(add, del, _)| view! {
                    <div class="flex-row" gap={tokens::scaled(Sp::XS, scale)}>
                        <text size={mono} color={tc.line_add_text}>{add.clone()}</text>
                        <text size={mono} color={tc.line_del_text}>{del.clone()}</text>
                    </div>
                })}
            </div>
            <text size={small} color={tc.text_muted}>
                {size_line.map(|(_, _, counts)| counts).unwrap_or_else(|| "Measuring\u{2026}".to_owned())}
            </text>
        </div>
    })
}
