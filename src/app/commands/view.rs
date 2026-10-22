use quark::{Px, view};
use quark_app::ViewContext;
use quark_components::{Modal, PALETTE_INPUT, PaletteEvent, kbd, kbd_icon, progress_bar};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::CommandsController;
use super::keycaps;
use super::keymap;
use super::list::{self, OptionRow};
use super::picker::{PickerChoice, PickerKind, PickerRow, RowKind, TYPED_PATH};
use super::state::{CONFIRM_ACCEPT, CONFIRM_CANCEL, LayerKind};
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::PICKER;
use crate::app::tokens::{self, Ico, Layout, Rad, Shadow, Sp, Sz, TextStyle};
use crate::contracts::commands::{CommandsAction, ToastKind};
use crate::contracts::shell::Action;
use crate::core::tips;

fn palette_event(event: PaletteEvent) -> quark_ui::Action {
    Action::from(CommandsAction::Palette(event)).into()
}

pub fn overlays(
    commands: &mut CommandsController,
    slot: &Slot,
    external: Option<AnyElement>,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let theme = cx.theme;
    let row_stride =
        theme.metrics.ui_row_height.round() + tokens::scaled(Sp::XS, tokens::ui_scale(theme));
    commands.observe_view(cx.focus(), row_stride);
    let size = (slot.width, slot.height);
    let mut children: Vec<AnyElement> = Vec::new();
    children.extend(super::settings_view::sheet(commands, size, external, cx));
    let layers: Vec<LayerKind> = [
        LayerKind::Picker,
        LayerKind::RefPicker,
        LayerKind::Palette,
        LayerKind::Confirmation,
        LayerKind::ContextMenu,
        LayerKind::Shortcuts,
    ]
    .into_iter()
    .filter(|kind| commands.has_layer(*kind))
    .collect();
    for kind in layers {
        let element = match kind {
            LayerKind::Palette => {
                let focused = cx.is_focused(PALETTE_INPUT);
                commands
                    .palette()
                    .render(size, cx.theme, focused, palette_event)
            }
            LayerKind::Picker => picker_panel(commands, size, cx),
            LayerKind::RefPicker => super::ref_picker_view::ref_picker_panel(commands, size, cx),
            LayerKind::Confirmation => confirmation(commands, size, cx.theme),
            LayerKind::Shortcuts => Some(shortcuts(commands, slot, cx.theme)),
            LayerKind::ContextMenu => context_menu(commands, size, cx.theme),
        };
        children.extend(element);
    }
    if children.is_empty() {
        return None;
    }
    Some(view! {
        <div class="absolute top-0 left-0" w={slot.width} h={slot.height}>
            {...children}
        </div>
    })
}

fn picker_panel(
    commands: &mut CommandsController,
    size: (f32, f32),
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let focused = cx.is_focused(PICKER);
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let row_stride = list::row_stride(theme);
    let picker = commands.picker_mut()?;
    picker.reveal_pending(row_stride);
    let picker = &*picker;
    let title = picker.kind.title();
    let repository = picker.kind == PickerKind::Repository;
    let panel_w = if repository {
        tokens::scaled(Layout::REPO_PICKER_W, scale)
    } else {
        Sz::MODAL_MD * scale
    }
    .min(size.0 - tokens::scaled(Sz::MODAL_MARGIN, scale))
    .max(1.0);
    let input = text_input(title, "")
        .field(&picker.query)
        .placeholder(picker.kind.placeholder())
        .focus_target(PICKER)
        .focused(focused)
        .search(true)
        .bare()
        .w_full()
        .h(tokens::scaled(Sz::INPUT, scale));
    let list = if picker.rows.is_empty() {
        let loading = picker.kind == PickerKind::File && picker.files.is_none();
        let h = if loading {
            row_stride * picker.visible_rows() as f32
        } else {
            theme.metrics.ui_row_height
        };
        view! {
            <div class="flex-row items-start" h={h} px={tokens::scaled(Sp::MD, scale)}>
                <div class="flex-row items-center" h={theme.metrics.ui_row_height}>
                    <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>
                        {picker.empty_message()}
                    </text>
                </div>
            </div>
        }
    } else {
        list::virtual_list(
            picker.rows.len(),
            picker.scroll_px,
            &picker.scrollbar,
            picker.visible_rows(),
            theme,
            title,
            |i| {
                let row = &picker.rows[i];
                let selected = i == picker.selected;
                if picker.repos.is_some() && row.detail.as_deref() != Some(TYPED_PATH) {
                    return repo_row(row, i, selected, theme);
                }
                list::option_row(
                    OptionRow {
                        id: format!("picker-row:{i}:{}", row.label),
                        label: &row.label,
                        highlights: &row.highlights,
                        detail: row.detail.as_deref(),
                        rhs: row.meta.as_deref(),
                        icon: repository.then(|| match row.choice {
                            PickerChoice::Repository(_)
                                if row.detail.as_deref() == Some(TYPED_PATH) =>
                            {
                                lucide::ARROW_RIGHT
                            }
                            _ => lucide::FOLDER_GIT,
                        }),
                        selected: i == picker.selected,
                        current: row.current,
                    },
                    theme,
                    Action::from(CommandsAction::SelectOverlayEntry(i)).into(),
                )
            },
            ScrollActionBuilder::new(|lines| {
                Action::from(CommandsAction::ScrollOverlay(lines)).into()
            }),
        )
    };
    let browsing = picker.repos.is_some() && picker.query.text().trim().is_empty();
    let count = picker.repos.as_ref().filter(|_| !browsing).map(|view| {
        let shown = picker
            .rows
            .iter()
            .filter(|row| matches!(row.choice, PickerChoice::Repository(_)))
            .filter(|row| row.detail.as_deref() != Some(TYPED_PATH))
            .count();
        format!("{shown} of {}", view.total())
    });
    let footer = repository.then(|| repository_footer(browsing, theme));
    let anchored = repository && picker.anchored;
    let left = if anchored {
        tokens::title_leading_inset(theme).min((size.0 - panel_w).max(0.0))
    } else {
        ((size.0 - panel_w) / 2.0).max(0.0)
    };
    let top = if anchored {
        theme.metrics.title_bar_height + tokens::scaled(Layout::POPOVER_GAP, scale)
    } else {
        tokens::scaled(Sz::MODAL_TOP_OFFSET, scale)
    };
    Some(view! {
        <div
            class="absolute top-0 left-0"
            w={size.0}
            h={size.1}
            z_index={300}
            bg={tc.overlay_scrim}
            test_id="picker-backdrop"
            on:click={CommandsAction::BackdropDismiss}
            block_mouse
        >
            <div
                class="absolute flex-col overflow-hidden"
                left={left}
                top={top}
                w={panel_w}
                bg={tc.elevated_surface}
                border={tc.border}
                rounded={tokens::scaled(Rad::XL, scale)}
                on:click={NoopAction}
                test_id="picker"
                role="dialog"
                focus_scope="diffy.picker"
                trap_focus={true}
                accessibility_role={AccessibilityRole::Dialog}
                aria-label={title}
            >
                <div
                    class="flex-row items-center"
                    gap={tokens::scaled(Sp::SM, scale)}
                    px={tokens::scaled(Sp::MD, scale)}
                    border_b={tc.border_variant}
                >
                    <icon svg={lucide::SEARCH} size={Ico::SM} color={tc.icon} />
                    <div class="flex-1 min-w-0">{input}</div>
                    {?count.map(|count| view! {
                        <text size={TextStyle::Caption.font_size(theme)} color={tc.text_muted}>{count}</text>
                    })}
                </div>
                <div p={tokens::scaled(Sp::XS, scale)}>{list}</div>
                {?footer}
            </div>
        </div>
    })
}

fn repo_row(row: &PickerRow, index: usize, selected: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    let caption = TextStyle::Caption.font_size(theme);
    let height = theme.metrics.ui_row_height.round();
    let pad = tokens::scaled(Sp::SM, scale);
    let paint = tokens::row_paint(theme, tc.elevated_surface, selected, true);
    let fill = paint.bg;
    let id = format!("picker-row:{index}:{}", row.label);
    let click: quark_ui::Action = Action::from(CommandsAction::SelectOverlayEntry(index)).into();
    match row.kind {
        RowKind::Section { count, busy } => {
            let status = if count == Some(0) {
                "scanning"
            } else {
                "updating"
            };
            view! {
                <div
                    class="w-full flex-row items-center"
                    h={height}
                    px={pad}
                    gap={tokens::scaled(Sp::SM - Sp::XXS, scale)}
                    key={id}
                    accessibility_role={AccessibilityRole::Heading}
                    aria-label={row.label.clone()}
                >
                    <text size={caption} class="font-semibold" color={tc.text_muted}>{row.label.to_uppercase()}</text>
                    {?count.map(|count| view! { <text size={caption} color={tc.text_muted}>{count.to_string()}</text> })}
                    <div class="flex-1" />
                    if busy {
                        <icon svg={lucide::LOADER} size={Ico::XS} color={tc.text_muted} />
                        <text size={caption} color={tc.text_muted}>{status}</text>
                    }
                </div>
            }
        }
        RowKind::Group { count, expanded } => view! {
            <div
                class="w-full flex-row items-center"
                h={height}
                px={pad}
                gap={tokens::scaled(Sp::SM - Sp::XXS, scale)}
                rounded={theme.metrics.control_radius}
                bg={fill}
                @when {!selected} { hover_bg={tc.ghost_element_hover} }
                cursor={CursorHint::Pointer}
                id={id.clone()}
                key={id}
                accessibility_role={AccessibilityRole::ListBoxOption}
                aria-label={format!("{}, {count} repositories", row.label)}
                aria-selected={selected}
                aria-expanded={expanded}
                on:click={click}
            >
                <icon svg={if expanded { lucide::CHEVRON_DOWN } else { lucide::CHEVRON_RIGHT }} size={Ico::XS} color={paint.muted} />
                <icon svg={lucide::FOLDER} size={Ico::SM} color={paint.icon} />
                <div class="flex-row items-center overflow-hidden" min_w={0.0}>
                    <text size={TextStyle::Body.font_size(theme)} class="font-medium truncate" color={paint.text}>{row.label.clone()}</text>
                </div>
                <div
                    class="flex-row items-center shrink-0"
                    px={tokens::scaled(Sp::XS + Sp::XXS, scale)}
                    rounded={tokens::scaled(Sp::SM, scale)}
                    bg={tc.element_background}
                >
                    <text size={caption} class="font-mono" color={tc.text_muted}>{count.to_string()}</text>
                </div>
            </div>
        },
        RowKind::Item => {
            let indent = if row.indent {
                tokens::scaled(Sp::XL + Sp::SM, scale)
            } else {
                pad
            };
            view! {
                <div
                    class="w-full flex-row items-center"
                    h={height}
                    pl={indent}
                    pr={pad}
                    gap={tokens::scaled(Sp::SM, scale)}
                    rounded={theme.metrics.control_radius}
                    bg={fill}
                    @when {!selected} { hover_bg={tc.ghost_element_hover} }
                    cursor={CursorHint::Pointer}
                    id={id.clone()}
                    key={id}
                    accessibility_role={AccessibilityRole::ListBoxOption}
                    aria-label={row.label.clone()}
                    aria-selected={selected}
                    on:click={click}
                >
                    <icon svg={lucide::FOLDER_GIT} size={Ico::SM} color={if row.current { tc.accent } else { paint.icon }} />
                    {list::highlighted(&row.label, &row.highlights, paint.text, theme)}
                    <div class="flex-1 flex-row overflow-hidden" min_w={0.0}>
                        {?row.detail.as_ref().map(|detail| view! {
                            <text size={small} class="truncate" color={paint.muted}>{detail.clone()}</text>
                        })}
                    </div>
                    {?row.branch.as_ref().map(|branch| view! {
                        <div
                            class="flex-row items-center overflow-hidden"
                            max_w={tokens::scaled(Layout::BRANCH_MAX_W, scale)}
                            gap={tokens::scaled(Sp::XS, scale)}
                        >
                            <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={paint.muted} />
                            <text size={TextStyle::MonoSmall.font_size(theme)} class="font-mono truncate" color={paint.muted}>{branch.clone()}</text>
                        </div>
                    })}
                    <div class="flex-row justify-end shrink-0" w={tokens::scaled(Layout::AGE_W, scale)}>
                        {?row.meta.as_ref().map(|age| view! { <text size={caption} color={paint.muted}>{age.clone()}</text> })}
                    </div>
                    if row.current {
                        <icon svg={lucide::CHECK} size={Ico::SM} color={tc.accent} />
                    }
                </div>
            }
        }
    }
}

fn repository_footer(browsing: bool, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let small = TextStyle::BodySmall.font_size(theme);
    view! {
        <div
            class="flex-row items-center w-full"
            gap={tokens::scaled(Sp::SM, scale)}
            px={tokens::scaled(Sp::XS, scale)}
            py={tokens::scaled(Sp::XS, scale)}
            border_t={tc.border_variant}
            bg={tc.panel}
        >
            <div
                class="flex-row items-center"
                h={tokens::scaled(Layout::CONTROL_H, scale)}
                px={tokens::scaled(Sp::SM, scale)}
                gap={tokens::scaled(Sp::SM - Sp::XXS, scale)}
                rounded={theme.metrics.control_radius}
                hover_bg={tc.ghost_element_hover}
                cursor={CursorHint::Pointer}
                id="picker.open-folder"
                role="button"
                aria-label="Open folder"
                on:click={CommandsAction::OpenRepositoryFolder}
            >
                <icon svg={lucide::FOLDER_OPEN} size={Ico::SM} color={tc.icon} />
                <text size={small} class="font-medium" color={tc.text}>"Open folder\u{2026}"</text>
            </div>
            <div class="flex-1" />
            <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)} pr={tokens::scaled(Sp::SM, scale)}>
                if browsing {
                    {kbd_icon(lucide::ARROW_UP, theme)}
                    {kbd_icon(lucide::ARROW_DOWN, theme)}
                    <text size={small} color={tc.text_muted}>"move"</text>
                    {kbd_icon(lucide::ARROW_RIGHT, theme)}
                    <text size={small} color={tc.text_muted}>"expand"</text>
                } else {
                    {kbd_icon(icons::ARROW_RIGHT_TO_LINE, theme)}
                    <text size={small} color={tc.text_muted}>"complete"</text>
                }
                {kbd_icon(icons::CORNER_DOWN_LEFT, theme)}
                <text size={small} color={tc.text_muted}>"open"</text>
            </div>
        </div>
    }
}

fn confirmation(
    commands: &CommandsController,
    size: (f32, f32),
    theme: &Theme,
) -> Option<AnyElement> {
    let request = commands.confirmation()?;
    let scale = tokens::ui_scale(theme);
    let icon = if request.destructive {
        lucide::ALERT_CIRCLE
    } else {
        lucide::INFO
    };
    let cancel = action_button(
        "Cancel",
        CommandsAction::CancelConfirmation.into(),
        CONFIRM_CANCEL,
        ButtonTone::Plain,
        theme,
    );
    let confirm = action_button(
        &request.confirm_label,
        CommandsAction::Confirm.into(),
        CONFIRM_ACCEPT,
        if request.destructive {
            ButtonTone::Danger
        } else {
            ButtonTone::Primary
        },
        theme,
    );
    let modal = Modal::new(
        request.title.clone(),
        request.message.clone(),
        icon,
        tokens::scaled(Sz::MODAL_SM, scale),
        size.0,
        size.1,
        CommandsAction::CancelConfirmation,
    )
    .footer_child(view! { <div class="flex-1" /> })
    .footer_child(cancel)
    .footer_child(confirm);
    Some(view! { <div class="absolute top-0 left-0" z_index={350}>{modal}</div> })
}

fn context_menu(
    commands: &mut CommandsController,
    size: (f32, f32),
    theme: &Theme,
) -> Option<AnyElement> {
    let menu = commands.context_menu_mut().render(size, theme)?;
    Some(view! {
        <div
            class="absolute top-0 left-0"
            w={size.0}
            h={size.1}
            z_index={450}
            test_id="context-menu-backdrop"
            on:click={CommandsAction::CloseContextMenu}
        >
            {menu}
        </div>
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonTone {
    Plain,
    Primary,
    Danger,
}

pub fn action_button(
    label: &str,
    action: Action,
    focus: FocusId,
    tone: ButtonTone,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let (bg, hover, fg) = match tone {
        ButtonTone::Plain => (tc.element_background, tc.element_hover, tc.text),
        ButtonTone::Primary => (tc.accent, tc.accent_strong, tc.on_accent),
        ButtonTone::Danger => (tc.status_error, tc.status_error, tc.on_accent),
    };
    view! {
        <div
            class="flex-row items-center justify-center shrink-0"
            px={tokens::scaled(Sp::MD, scale)}
            h={theme.metrics.ui_row_height}
            rounded={theme.metrics.control_radius}
            bg={bg}
            hover_bg={hover}
            cursor={CursorHint::Pointer}
            focus_ring={focus}
            role="button"
            aria-label={label}
            on:click={action}
        >
            <text size={TextStyle::Body.font_size(theme)} class="font-medium" color={fg}>{label}</text>
        </div>
    }
}

pub fn toasts(
    commands: &mut CommandsController,
    slot: &Slot,
    dock: f32,
    tip: Option<AnyElement>,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let now = cx.frame.elapsed().as_millis() as u64;
    if let Some(next) = commands.toasts_mut().tick(now) {
        cx.frame
            .request_frame_in(std::time::Duration::from_millis(next.max(1)));
    }
    let items = commands.toasts().items();
    if items.is_empty() && tip.is_none() {
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let width = tokens::scaled(Sz::TOAST_MAX_W, scale)
        .min(slot.width - tokens::scaled(Sp::LG, scale) * 2.0)
        .max(1.0);
    let margin = tokens::scaled(Sp::LG, scale);
    let rows: Vec<AnyElement> = items
        .iter()
        .map(|toast| {
            let (icon, tint) = match toast.kind {
                ToastKind::Info => (lucide::INFO, tc.text_muted),
                ToastKind::Success => (icons::CIRCLE_CHECK, tc.line_add_text),
                ToastKind::Error => (lucide::ALERT_CIRCLE, tc.status_error),
            };
            let role = match toast.kind {
                ToastKind::Error => AccessibilityRole::Alert,
                ToastKind::Info | ToastKind::Success => AccessibilityRole::Status,
            };
            let progress = toast.fraction().map(|f| view! { <div class="w-full">{progress_bar(f)}</div> });
            let control = tokens::scaled(Layout::CONTROL_SM_H, scale);
            view! {
                <div
                    class="flex-row items-start w-full"
                    gap={tokens::scaled(Sp::SM, scale)}
                    pl={tokens::scaled(Sp::MD, scale)}
                    pr={tokens::scaled(Sp::XS, scale)}
                    py={tokens::scaled(Sp::XS, scale)}
                    bg={tc.elevated_surface}
                    border={tc.border_variant}
                    rounded={tokens::scaled(Rad::LG, scale)}
                    shadow_preset={Shadow::TOAST}
                    key={toast.id.to_string()}
                    accessibility_role={role}
                    aria-label={toast.message.clone()}
                    test_id="toast"
                >
                    <div class="flex-row items-center shrink-0" h={control}>
                        <icon svg={icon} size={Ico::SM} color={tint} />
                    </div>
                    <div class="flex-1 flex-col justify-center" min_w={0.0} min_h={control} gap={tokens::scaled(Sp::XXS, scale)} py={tokens::scaled(Sp::XXS, scale)}>
                        <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.text}>{toast.message.clone()}</text>
                        {?toast.description.clone().map(|d| view! { <text size={TextStyle::Caption.font_size(theme)} color={tc.text_muted}>{d}</text> })}
                        {?progress}
                    </div>
                    if toast.undo.is_some() {
                        <div
                            class="flex-row items-center shrink-0"
                            h={control}
                            px={tokens::scaled(Sp::SM, scale)}
                            rounded={theme.metrics.control_radius}
                            hover_bg={tc.ghost_element_hover}
                            cursor={CursorHint::Pointer}
                            id={format!("toast.undo.{}", toast.id)}
                            role="button"
                            aria-label="Undo"
                            on:click={CommandsAction::UndoToast(toast.id)}
                        >
                            <text size={TextStyle::BodySmall.font_size(theme)} class="font-medium" color={tc.accent}>"Undo"</text>
                        </div>
                    }
                    <div
                        class="flex-row items-center justify-center shrink-0"
                        w={control}
                        h={control}
                        rounded={theme.metrics.control_radius}
                        hover_bg={tc.ghost_element_hover}
                        cursor={CursorHint::Pointer}
                        id={format!("toast.dismiss.{}", toast.id)}
                        role="button"
                        aria-label="Dismiss"
                        on:click={CommandsAction::DismissToast(toast.id)}
                    >
                        <icon svg={lucide::X} size={Ico::XS} color={tc.text_muted} />
                    </div>
                </div>
            }
        })
        .collect();
    Some(view! {
        <div
            class="absolute flex-col"
            right={margin}
            bottom={theme.metrics.status_bar_height + dock + margin}
            w={width}
            gap={tokens::scaled(Sp::SM, scale)}
            z_index={500}
            accessibility_role={AccessibilityRole::Log}
            aria-label="Notifications"
        >
            {...rows}
            {?tip}
        </div>
    })
}

const LEADER_MENU_DELAY_MS: u64 = 350;

pub fn leader_menu(
    commands: &CommandsController,
    slot: &Slot,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    if !commands.go_pending() {
        return None;
    }
    let now = cx.frame.elapsed().as_millis() as u64;
    let shown_at = commands.go_since(now) + LEADER_MENU_DELAY_MS;
    if now < shown_at {
        cx.frame
            .request_frame_in(std::time::Duration::from_millis(shown_at - now));
        return None;
    }
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let margin = tokens::scaled(Sp::LG, scale);
    let width = tokens::scaled(Sz::TOAST_MAX_W, scale)
        .min(slot.width - margin * 2.0)
        .max(1.0);
    let small = TextStyle::BodySmall.font_size(theme);
    let row = |leader: &keymap::Leader| {
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM} py={Sp::XXS}
                 accessibility_role={AccessibilityRole::ListItem}
                 aria-label={format!("{}: {}", leader.key, leader.label)}>
                {kbd(leader.key, theme)}
                <text size={Px(small)} color={tc.text}>{leader.label}</text>
            </div>
        }
    };
    let half = keymap::GO.len().div_ceil(2);
    let left: Vec<AnyElement> = keymap::GO[..half].iter().map(row).collect();
    let right: Vec<AnyElement> = keymap::GO[half..].iter().map(row).collect();
    Some(view! { scale,
        <div
            class="absolute flex-col"
            right={Px(margin)}
            bottom={Px(theme.metrics.status_bar_height + margin)}
            w={Px(width)}
            gap={Sp::XS}
            p={Sp::MD}
            bg={tc.elevated_surface}
            border={tc.border_variant}
            rounded={Rad::LG}
            shadow_preset={Shadow::TOAST}
            z_index={600}
            accessibility_role={AccessibilityRole::List}
            aria-label="Go to"
            test_id="leader.go"
        >
            <div class="flex-row items-center w-full" gap={Sp::XS}>
                {kbd("g", theme)}
                <text size={Px(small)} color={tc.text_muted}>"Go to\u{2026}"</text>
                <spacer />
                <text size={Px(TextStyle::Caption.font_size(theme))} color={tc.text_muted}>"esc"</text>
            </div>
            <div class="flex-row items-start w-full" gap={Sp::LG}>
                <div class="flex-col flex-1">{...left}</div>
                <div class="flex-col flex-1">{...right}</div>
            </div>
        </div>
    })
}

fn shortcuts(commands: &CommandsController, slot: &Slot, theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &slot.shared.settings.settings.keymap_overrides;
    let small = TextStyle::BodySmall.font_size(theme);
    let caption = TextStyle::Caption.font_size(theme);
    let reviewing = commands.shortcuts_reviewing();
    let order: &[&str] = if reviewing {
        &[
            "Review",
            "Navigation",
            "Areas",
            "View",
            "Search",
            "Scrolling",
        ]
    } else {
        &[
            "Navigation",
            "Areas",
            "Working Tree",
            "View",
            "Search",
            "Repository",
            "Scrolling",
        ]
    };
    let row = |label: String, binding: String, caps: AnyElement| {
        view! { scale,
            <div class="flex-row items-center w-full" gap={Sp::SM} py={Sp::XXS}
                 accessibility_role={AccessibilityRole::ListItem}
                 aria-label={format!("{label}: {binding}")}>
                <div class="flex-1 flex-row" min_w={0.0}>
                    <text size={Px(small)} class="truncate" color={tc.text}>{label}</text>
                </div>
                {caps}
            </div>
        }
    };
    let block = |title: &str, rows: Vec<AnyElement>| {
        let title = title.to_owned();
        view! { scale,
            <div class="flex-col w-full" gap={Sp::XXS}
                 accessibility_role={AccessibilityRole::List} aria-label={title.clone()}>
                <text size={Px(caption)} class="font-semibold" color={tc.text_muted}>{title.to_uppercase()}</text>
                {...rows}
            </div>
        }
    };
    let mut blocks: Vec<(usize, AnyElement)> = Vec::new();
    for title in order {
        let Some(group) = keymap::KEYMAP.groups().iter().find(|g| g.title == *title) else {
            continue;
        };
        let rows: Vec<AnyElement> = group
            .entries
            .iter()
            .filter_map(|entry| {
                let binding = keymap::display_binding(saved, entry.command)?;
                let caps = keycaps::command(saved, entry.command, theme)?;
                Some(row(entry.description.to_owned(), binding, caps))
            })
            .collect();
        if !rows.is_empty() {
            blocks.push((rows.len(), block(title, rows)));
        }
        if *title == "Navigation" {
            let go: Vec<AnyElement> = keymap::GO
                .iter()
                .map(|leader| {
                    let binding = format!("g {}", leader.key);
                    let caps = keycaps::caps(&binding, theme);
                    row(leader.label.to_owned(), binding, caps)
                })
                .collect();
            blocks.push((go.len(), block("Go to", go)));
        }
    }
    let mut columns: [Vec<AnyElement>; 2] = Default::default();
    let mut heights = [0usize; 2];
    for (count, element) in blocks {
        let column = usize::from(heights[1] < heights[0]);
        heights[column] += count + 2;
        columns[column].push(element);
    }
    let [left, right] = columns;
    let margin = tokens::scaled(Sz::MODAL_MARGIN, scale);
    let height = (slot.height - margin * 2.0).max(1.0);
    let customize = action_button(
        "Customize",
        CommandsAction::OpenKeymaps.into(),
        SHORTCUTS_CUSTOMIZE,
        ButtonTone::Plain,
        theme,
    );
    let modal = Modal::new(
        "Keyboard shortcuts",
        if reviewing {
            "Reviewing a pull request"
        } else {
            "Everything you can do without the mouse"
        },
        icons::KEYBOARD,
        tokens::scaled(Sz::MODAL_XL, scale),
        slot.width,
        slot.height,
        CommandsAction::CloseOverlay,
    )
    .height(height)
    .body_child(view! { scale,
        <div class="flex-row items-start w-full overflow-y-scroll scrollbar-auto-hide" gap={Sp::XL}
             focus_target={crate::app::text_targets::SHORTCUTS}
             test_id="shortcuts.sheet">
            <div class="flex-col flex-1" min_w={0.0} gap={Sp::LG}>{...left}</div>
            <div class="flex-col flex-1" min_w={0.0} gap={Sp::LG}>{...right}</div>
        </div>
    })
    .footer_child(view! { <div class="flex-1" /> })
    .footer_child(customize);
    view! { <div class="absolute top-0 left-0" z_index={350}>{modal}</div> }
}

const SHORTCUTS_CUSTOMIZE: FocusId = FocusId::from_key("diffy.shortcuts.customize");

pub fn tip_card(
    commands: &CommandsController,
    slot: &Slot,
    reviewing: bool,
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let context = if reviewing {
        tips::Context::Reviewing
    } else {
        tips::Context::Workspace
    };
    let now = cx.frame.elapsed().as_millis() as u64;
    let tip = match commands.visible_tip(context, now) {
        Ok(tip) => tip?,
        Err(wait) => {
            cx.frame
                .request_frame_in(std::time::Duration::from_millis(wait.max(1)));
            return None;
        }
    };
    let theme = cx.theme;
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &slot.shared.settings.settings.keymap_overrides;
    let small = TextStyle::BodySmall.font_size(theme);
    let row = |caps: AnyElement, label: &str| {
        let label = label.to_owned();
        view! { scale,
            <div class="flex-row items-center" gap={Sp::SM}>
                {caps}
                <text size={Px(small)} color={tc.text_muted}>{label}</text>
            </div>
        }
    };
    let mut rows: Vec<AnyElement> = tip
        .keys
        .iter()
        .filter_map(|(command, label)| Some(row(keycaps::command(saved, *command, theme)?, label)))
        .collect();
    if tip.leader {
        rows.extend(
            keymap::GO
                .iter()
                .filter(|leader| matches!(leader.key, "f" | "h" | "p"))
                .map(|leader| {
                    row(
                        keycaps::caps(&format!("g {}", leader.key), theme),
                        leader.label,
                    )
                }),
        );
        rows.push(view! { scale,
            <text size={Px(small)} color={tc.text_muted}>"Pause after g to see every destination."</text>
        });
    }
    let dismiss = action_button(
        "Got it",
        CommandsAction::DismissTip.into(),
        TIP_DISMISS,
        ButtonTone::Plain,
        theme,
    );
    Some(view! { scale,
        <div
            class="flex-col w-full"
            gap={Sp::SM}
            p={Sp::MD}
            bg={tc.elevated_surface}
            border={tc.border_variant}
            rounded={Rad::LG}
            shadow_preset={Shadow::TOAST}
            accessibility_role={AccessibilityRole::Note}
            aria-label={format!("Tip: {}", tip.title)}
            test_id="tip.card"
        >
            <div class="flex-row items-center w-full" gap={Sp::SM}>
                <icon svg={icons::KEYBOARD} size={Ico::SM} color={tc.accent} />
                <text size={Px(TextStyle::Body.font_size(theme))} class="font-semibold" color={tc.text_strong}>{tip.title}</text>
            </div>
            <div class="flex-col" gap={Sp::XS}>{...rows}</div>
            <div class="flex-row items-center w-full">
                <spacer />
                {dismiss}
            </div>
        </div>
    })
}

pub fn hint_bar(
    commands: &CommandsController,
    slot: &Slot,
    reviewing: bool,
    theme: &Theme,
) -> Option<AnyElement> {
    if !commands.hint_bar() {
        return None;
    }
    use crate::core::keymap::ShortcutCommand as C;
    let keys: &[(C, &str)] = if reviewing {
        &[
            (C::NextHunk, "hunk"),
            (C::NextThread, "thread"),
            (C::NextFile, "file"),
            (C::ReviewComment, "comment"),
            (C::ReviewReply, "reply"),
            (C::ReviewResolve, "resolve"),
            (C::ReviewMarkViewed, "viewed"),
            (C::SubmitReview, "submit"),
        ]
    } else {
        &[
            (C::NextHunk, "hunk"),
            (C::NextFile, "file"),
            (C::JumpToElement, "jump"),
            (C::GoPrefix, "go to"),
            (C::OpenCommandPalette, "anything"),
        ]
    };
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let saved = &slot.shared.settings.settings.keymap_overrides;
    let small = TextStyle::BodySmall.font_size(theme);
    let margin = tokens::scaled(Sp::LG, scale);
    let hint = |caps: AnyElement, label: &str| {
        let label = label.to_owned();
        view! { scale,
            <div class="flex-row items-center shrink-0" gap={Sp::XS}>
                {caps}
                <text size={Px(small)} color={tc.text_muted}>{label}</text>
            </div>
        }
    };
    let hints: Vec<AnyElement> = keys
        .iter()
        .filter_map(|(command, label)| Some(hint(keycaps::command(saved, *command, theme)?, label)))
        .collect();
    let all = keycaps::command(saved, C::ShowKeymaps, theme).map(|caps| hint(caps, "all keys"));
    let close = view! { scale,
        <div class="flex-row items-center justify-center shrink-0"
             w={Px(tokens::scaled(Layout::CONTROL_SM_H, scale))}
             h={Px(tokens::scaled(Layout::CONTROL_SM_H, scale))}
             rounded={theme.metrics.control_radius}
             hover_bg={tc.ghost_element_hover}
             cursor={CursorHint::Pointer}
             id="hints.hide"
             role="button"
             aria-label="Hide key hints"
             tooltip="Hide key hints"
             on:click={CommandsAction::HideHintBar}>
            <icon svg={lucide::X} size={Ico::XS} color={tc.text_muted} />
        </div>
    };
    Some(view! { scale,
        <div
            class="absolute flex-row items-center overflow-hidden"
            left={Px(margin)}
            right={Px(margin)}
            bottom={Px(theme.metrics.status_bar_height + margin)}
            h={Px(hint_bar_height(theme))}
            gap={Sp::LG}
            px={Sp::MD}
            bg={tc.title_bar_background}
            border={tc.border_variant}
            rounded={Rad::LG}
            shadow_preset={Shadow::TOAST}
            z_index={400}
            accessibility_role={AccessibilityRole::Toolbar}
            aria-label="Key hints"
            test_id="hints.bar"
        >
            {...hints}
            <spacer />
            {?all}
            {close}
        </div>
    })
}

pub fn hint_bar_height(theme: &Theme) -> f32 {
    tokens::scaled(Layout::COMPARE_BAR_H, tokens::ui_scale(theme))
}

const TIP_DISMISS: FocusId = FocusId::from_key("diffy.tip.dismiss");
