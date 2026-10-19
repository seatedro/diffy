use quark::view;
use quark_components::{Button, ButtonSize, ButtonStyle};
use quark_ui::FocusId;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::theme::{ControlMetrics, Theme};

use crate::app::tokens::{Ico, Rad, Sp, TextStyle};

const TEXT_LINE_HEIGHT: f32 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Ghost,
    Destructive,
}

impl Tone {
    fn style(self) -> ButtonStyle {
        match self {
            Self::Primary => ButtonStyle::Filled,
            Self::Secondary => ButtonStyle::Subtle,
            Self::Ghost => ButtonStyle::Ghost,
            Self::Destructive => ButtonStyle::Danger,
        }
    }
}

pub fn button(
    label: impl Into<String>,
    on_click: impl Into<quark_ui::Action>,
    tone: Tone,
) -> Button {
    Button::new(on_click)
        .label(label)
        .style(tone.style())
        .size(ButtonSize::Compact)
}

pub fn control_height(theme: &Theme) -> f32 {
    let scale = theme.metrics.ui_scale();
    let m = theme.components.button_compact;
    m.height.map_or_else(
        || {
            TextStyle::Caption.font_size(theme) * TEXT_LINE_HEIGHT
                + 2.0 * (m.padding_y.unwrap_or(Sp::XXS) * scale).round()
        },
        |height| (height * scale).round(),
    )
}

pub fn icon_button(
    icon: &'static str,
    tooltip: &str,
    on_click: impl Into<quark_ui::Action>,
    theme: &Theme,
) -> Button {
    let m = theme.components.button_compact;
    Button::new(on_click)
        .icon(icon)
        .tooltip(tooltip.to_owned())
        .style(ButtonStyle::Ghost)
        .size(ButtonSize::Compact)
        .fixed_size(control_height(theme) / theme.metrics.ui_scale())
        .metrics(ControlMetrics {
            radius: Some(m.radius.unwrap_or(Rad::XL)),
            ..ControlMetrics::default()
        })
}

pub struct GhostChip<'a> {
    pub icon: &'static str,
    pub label: &'a str,
    pub accessible: &'a str,
    pub focus: FocusId,
    pub test_id: &'a str,
}

pub fn ghost_chip(
    chip: GhostChip,
    on_click: impl Into<quark_ui::Action>,
    theme: &Theme,
) -> AnyElement {
    let tc = &theme.colors;
    let scale = theme.metrics.ui_scale();
    let m = theme.components.button_compact;
    let icon = m.icon_size.unwrap_or(Ico::BUTTON_COMPACT);
    let on_click: quark_ui::Action = on_click.into();
    view! { scale,
        <div
            class="flex-row items-center shrink-0"
            gap={m.gap.unwrap_or(Sp::SM)}
            px={m.padding_x.unwrap_or(Sp::SM)}
            py={m.padding_y.unwrap_or(Sp::XXS)}
            rounded={m.radius.unwrap_or(Rad::XL)}
            hover_bg={tc.ghost_element_hover}
            cursor={CursorHint::Pointer}
            focus_ring={chip.focus}
            accessibility_role={AccessibilityRole::Button}
            aria-label={chip.accessible}
            test_id={chip.test_id}
            on:click={on_click}
            h={?m.height}
            @when {m.height.is_some()} {
                pt={0.0}
                pb={0.0}
            }
        >
            <icon svg={chip.icon} size={icon} color={tc.text_muted} />
            <text class="text-xs font-medium" color={tc.text_muted}>{chip.label}</text>
        </div>
    }
}
