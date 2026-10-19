use quark::view;
use quark_app::ViewContext;
use quark_components::{Modal, PALETTE_INPUT, PaletteEvent, progress_bar};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::CommandsController;
use super::list::{self, OptionRow};
use super::picker::{PickerChoice, PickerKind, PickerRow, RowKind, TYPED_PATH};
use super::state::{CONFIRM_ACCEPT, CONFIRM_CANCEL, LayerKind};
use crate::app::icons;
use crate::app::shell::Slot;
use crate::app::text_targets::PICKER;
use crate::app::tokens::{self, Ico, Layout, Rad, Shadow, Sp, Sz, TextStyle};
use crate::contracts::commands::{CommandsAction, ToastKind};
use crate::contracts::shell::Action;

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
        view! {
            <div class="flex-row items-center" h={theme.metrics.ui_row_height} px={tokens::scaled(Sp::MD, scale)}>
                <text size={TextStyle::BodySmall.font_size(theme)} color={tc.text_muted}>"No matches"</text>
            </div>
        }
    } else {
        list::virtual_list(
            picker.rows.len(),
            picker.scroll_px,
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
    let left = if repository {
        tokens::title_leading_inset(theme).min((size.0 - panel_w).max(0.0))
    } else {
        ((size.0 - panel_w) / 2.0).max(0.0)
    };
    let top = if repository {
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
    let fill = if selected {
        tokens::selected_fill(tc.elevated_surface, true, theme)
    } else {
        Color::TRANSPARENT
    };
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
                <icon svg={if expanded { lucide::CHEVRON_DOWN } else { lucide::CHEVRON_RIGHT }} size={Ico::XS} color={tc.text_muted} />
                <icon svg={lucide::FOLDER} size={Ico::SM} color={tc.icon} />
                <div class="flex-row items-center overflow-hidden" min_w={0.0}>
                    <text size={TextStyle::Body.font_size(theme)} class="font-medium truncate" color={tc.text}>{row.label.clone()}</text>
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
            let name_color = if selected { tc.text_strong } else { tc.text };
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
                    <icon svg={lucide::FOLDER_GIT} size={Ico::SM} color={if row.current { tc.accent } else { tc.icon }} />
                    {list::highlighted(&row.label, &row.highlights, name_color, theme)}
                    <div class="flex-1 flex-row overflow-hidden" min_w={0.0}>
                        {?row.detail.as_ref().map(|detail| view! {
                            <text size={small} class="truncate" color={tc.text_muted}>{detail.clone()}</text>
                        })}
                    </div>
                    {?row.branch.as_ref().map(|branch| view! {
                        <div
                            class="flex-row items-center overflow-hidden"
                            max_w={tokens::scaled(Layout::BRANCH_MAX_W, scale)}
                            gap={tokens::scaled(Sp::XS, scale)}
                        >
                            <icon svg={lucide::GIT_BRANCH} size={Ico::XS} color={tc.text_muted} />
                            <text size={TextStyle::MonoSmall.font_size(theme)} class="font-mono truncate" color={tc.text_muted}>{branch.clone()}</text>
                        </div>
                    })}
                    <div class="flex-row justify-end shrink-0" w={tokens::scaled(Layout::AGE_W, scale)}>
                        {?row.meta.as_ref().map(|age| view! { <text size={caption} color={tc.text_muted}>{age.clone()}</text> })}
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
    let inner = (tokens::scaled(Rad::XL, scale) - 1.0).max(0.0);
    view! {
        <div
            class="flex-row items-center w-full"
            gap={tokens::scaled(Sp::SM, scale)}
            px={tokens::scaled(Sp::XS, scale)}
            py={tokens::scaled(Sp::XS, scale)}
            border_t={tc.border_variant}
            bg={tc.panel}
            rounded_corners={[0.0, 0.0, inner, inner]}
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
                    {key_icons(&[lucide::ARROW_UP, lucide::ARROW_DOWN], theme)}
                    <text size={small} color={tc.text_muted}>"move"</text>
                    {key_icons(&[lucide::ARROW_RIGHT], theme)}
                    <text size={small} color={tc.text_muted}>"expand"</text>
                } else {
                    {key_icons(&[icons::ARROW_RIGHT_TO_LINE], theme)}
                    <text size={small} color={tc.text_muted}>"complete"</text>
                }
                {key_icons(&[icons::CORNER_DOWN_LEFT], theme)}
                <text size={small} color={tc.text_muted}>"open"</text>
            </div>
        </div>
    }
}

fn key_icons(svgs: &[&'static str], theme: &Theme) -> AnyElement {
    let tc = &theme.colors;
    let scale = tokens::ui_scale(theme);
    let recipe = theme.components.kbd;
    let height = (recipe.height.unwrap_or(Ico::MD) * scale).round();
    let icons: Vec<AnyElement> = svgs
        .iter()
        .map(
            |svg| view! { <icon svg={*svg} size={(height * 0.6).round()} color={tc.text_muted} /> },
        )
        .collect();
    view! {
        <div
            class="shrink-0 flex-row items-center justify-center"
            h={height}
            min_w={height}
            px={tokens::scaled(Sp::XXS, scale)}
            rounded={recipe.radius.unwrap_or(Rad::SM) * scale}
            border={tc.border_variant}
        >
            {...icons}
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
    cx: &mut ViewContext,
) -> Option<AnyElement> {
    let now = cx.frame.elapsed().as_millis() as u64;
    if let Some(next) = commands.toasts_mut().tick(now) {
        cx.frame
            .request_frame_in(std::time::Duration::from_millis(next.max(1)));
    }
    let items = commands.toasts().items();
    if items.is_empty() {
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
            bottom={theme.metrics.status_bar_height + margin}
            w={width}
            gap={tokens::scaled(Sp::SM, scale)}
            z_index={500}
            accessibility_role={AccessibilityRole::Log}
            aria-label="Notifications"
        >
            {...rows}
        </div>
    })
}
