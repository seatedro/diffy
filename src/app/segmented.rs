use std::hash::{DefaultHasher, Hash, Hasher};

use quark::{TabStop, view};
use quark_components::segmented_focus_id;
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use crate::app::tokens::{self, Layout, Sp, TextStyle};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentStyle {
    height: f32,
    inset: f32,
    pad_x: f32,
    radius: f32,
    font: f32,
    border: Color,
    selected: Color,
    hover: Color,
    text: Color,
    muted: Color,
}

impl SegmentStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        Self {
            height: tokens::scaled(Layout::CONTROL_H, scale),
            inset: tokens::scaled(Sp::XXS, scale),
            pad_x: tokens::scaled(Sp::SM + Sp::XXS, scale),
            radius: theme.metrics.control_radius.round(),
            font: TextStyle::BodySmall.font_size(theme),
            border: tc.border_variant,
            selected: tc.element_hover,
            hover: tc.ghost_element_hover,
            text: tc.text_strong,
            muted: tc.text_muted,
        }
    }

    pub fn key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for value in [self.height, self.inset, self.pad_x, self.radius, self.font] {
            value.to_bits().hash(&mut hasher);
        }
        for c in [
            self.border,
            self.selected,
            self.hover,
            self.text,
            self.muted,
        ] {
            [c.r, c.g, c.b, c.a].hash(&mut hasher);
        }
        hasher.finish()
    }
}

pub struct Segment {
    pub label: String,
    pub action: quark_ui::Action,
    pub selected: bool,
}

impl Segment {
    pub fn new(
        label: impl Into<String>,
        action: impl Into<quark_ui::Action>,
        selected: bool,
    ) -> Self {
        Self {
            label: label.into(),
            action: action.into(),
            selected,
        }
    }
}

pub fn segmented(id: &str, segments: Vec<Segment>, style: SegmentStyle) -> AnyElement {
    let len = segments.len();
    let selected = segments.iter().position(|segment| segment.selected);
    let stop = selected.unwrap_or(0);
    let step = |delta: usize| selected.map(|at| segments[(at + delta) % len].action.clone());
    let next = step(1);
    let previous = step(len.saturating_sub(1));
    let item_h = style.height - 2.0 * (style.inset + 1.0);
    let items: Vec<AnyElement> = segments
        .into_iter()
        .enumerate()
        .map(|(i, segment)| {
            let key = format!("segmented:{:?}:{}", segment.action, segment.label);
            view! {
                <div
                    class="flex-row items-center justify-center"
                    h={item_h}
                    px={style.pad_x}
                    rounded={style.radius - style.inset}
                    cursor={CursorHint::Pointer}
                    id={key.clone()}
                    key={segment.label.clone()}
                    accessibility_id={key}
                    accessibility_role={AccessibilityRole::RadioButton}
                    aria-label={segment.label.clone()}
                    aria-selected={segment.selected}
                    aria-checked={segment.selected}
                    focus_ring={segmented_focus_id(id, i)}
                    on:click={segment.action}
                    @when {segment.selected} { bg={style.selected} }
                    @when {!segment.selected} { hover_bg={style.hover} }
                    @when {i != stop} { tab_stop={TabStop::disabled(0)} }
                >
                    <text
                        size={style.font}
                        class="font-medium"
                        color={if segment.selected { style.text } else { style.muted }}
                    >
                        {segment.label}
                    </text>
                </div>
            }
        })
        .collect();
    view! {
        <div
            class="shrink-0 flex-row items-center"
            h={style.height}
            p={style.inset}
            gap={style.inset}
            rounded={style.radius}
            border={style.border}
            id={id.to_owned()}
            accessibility_id={id.to_owned()}
            accessibility_role={AccessibilityRole::RadioGroup}
            @when {let Some(next) = next} { on_key={("arrowright", next)} }
            @when {let Some(previous) = previous} { on_key={("arrowleft", previous)} }
        >
            {...items}
        </div>
    }
}
