use std::hash::{DefaultHasher, Hash, Hasher};

use quark::view;
use quark_ui::FocusId;
use quark_ui::design::Alpha;
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use crate::app::tokens::{self, Ico, Layout, Sp, TextStyle};
use crate::contracts::shell::Action;

const ROW_FILL: f32 = 0.72;
const ICON_FILL: f32 = 0.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Ghost,
    Destructive,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub action: Action,
    pub tone: Tone,
    pub shortcut: Option<&'static str>,
    pub icon: Option<&'static str>,
    pub disabled: bool,
    pub icon_only: bool,
    pub title: Option<String>,
}

impl Control {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        action: impl Into<Action>,
        tone: Tone,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            action: action.into(),
            tone,
            shortcut: None,
            icon: None,
            disabled: false,
            icon_only: false,
            title: None,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn shortcut(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn icon_if(mut self, show: bool, icon: &'static str) -> Self {
        if show {
            self.icon = Some(icon);
        }
        self
    }

    pub fn icon_only(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self.icon_only = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn focus(&self) -> FocusId {
        focus_id(&self.id)
    }
}

pub fn focus_id(id: &str) -> FocusId {
    let mut hasher = DefaultHasher::new();
    "diffy.mutations".hash(&mut hasher);
    id.hash(&mut hasher);
    FocusId::new(hasher.finish())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    pub height: f32,
    pub radius: f32,
    pub pad_x: f32,
    pub gap: f32,
    pub font: f32,
    pub small: f32,
    pub icon: f32,
    pub accent: Color,
    pub accent_strong: Color,
    pub on_accent: Color,
    pub element: Color,
    pub element_hover: Color,
    pub ghost_hover: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub danger: Color,
}

impl Chrome {
    pub fn from_theme(theme: &Theme) -> Self {
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        Self {
            height: tokens::scaled(Layout::CONTROL_H, scale),
            radius: theme.metrics.control_radius,
            pad_x: tokens::scaled(Sp::SM, scale),
            gap: tokens::scaled(Sp::SM - Sp::XXS, scale),
            font: TextStyle::BodySmall.font_size(theme),
            small: TextStyle::Caption.font_size(theme),
            icon: tokens::scaled(Ico::SM, scale),
            accent: tc.accent,
            accent_strong: tc.accent_strong,
            on_accent: tc.on_accent,
            element: tc.element_background,
            element_hover: tc.element_hover,
            ghost_hover: tc.ghost_element_hover,
            border: tc.border_variant,
            text: tc.text,
            muted: tc.text_muted,
            danger: tc.status_error,
        }
    }
}

impl Chrome {
    pub fn fit(mut self, row: f32) -> Self {
        let height = (row * ROW_FILL).floor().min(self.height);
        if height < self.height {
            self.icon = self.icon.min((height * ICON_FILL).round());
            self.pad_x = self.pad_x.min(height * 0.5);
            self.height = height;
        }
        self
    }

    pub fn square(mut self, size: f32) -> Self {
        self.height = size;
        self.icon = self.icon.min((size * ICON_FILL).round());
        self.radius = self.radius.min(size * 0.25);
        self
    }
}

impl Default for Chrome {
    fn default() -> Self {
        Self::from_theme(&crate::app::theme::default_dark())
    }
}

pub fn kbd(label: &str, chrome: &Chrome, fg: Color) -> AnyElement {
    let label = label.to_owned();
    view! {
        <div
            class="shrink-0 flex-row items-center justify-center"
            px={chrome.gap}
            rounded={chrome.radius * 0.5}
            border={fg.with_alpha(Alpha::MUTED)}
        >
            <text size={chrome.small} class="font-mono" color={fg.with_alpha(Alpha::STRONG)}>{label}</text>
        </div>
    }
}

pub fn render(control: Control, chrome: &Chrome) -> AnyElement {
    let (bg, hover, fg, border) = match control.tone {
        Tone::Primary => (chrome.accent, chrome.accent_strong, chrome.on_accent, None),
        Tone::Secondary => (
            chrome.element,
            chrome.element_hover,
            chrome.text,
            Some(chrome.border),
        ),
        Tone::Ghost => (Color::TRANSPARENT, chrome.ghost_hover, chrome.muted, None),
        Tone::Destructive => (
            chrome.element,
            chrome.element_hover,
            chrome.danger,
            Some(chrome.border),
        ),
    };
    let fg = if control.disabled {
        fg.with_alpha(Alpha::MUTED)
    } else {
        fg
    };
    let focus = control.focus();
    let action = control.action.clone();
    let enabled = !control.disabled;
    let label = control
        .title
        .clone()
        .unwrap_or_else(|| control.label.clone());
    let shown = (!control.icon_only).then(|| control.label.clone());
    let shortcut = control.shortcut.map(|keys| kbd(keys, chrome, fg));
    let width = control.icon_only.then_some(chrome.height);
    view! {
        <div
            class="shrink-0 flex-row items-center justify-center"
            h={chrome.height}
            @when {width.is_some()} { w={width.unwrap_or_default()} }
            @when {width.is_none()} { px={chrome.pad_x} }
            gap={chrome.gap}
            rounded={chrome.radius}
            bg={bg}
            @when {border.is_some()} { border={border.unwrap_or(bg)} }
            @when {enabled} {
                hover_bg={hover}
                cursor={CursorHint::Pointer}
                on:click={action.clone()}
                on:key:enter={action.clone()}
                on:key:space={action}
            }
            focus_ring={focus}
            role="button"
            aria-label={label.clone()}
            aria-disabled={!enabled}
            tooltip={label}
            test_id={control.id.clone()}
        >
            if let Some(icon) = control.icon {
                <icon svg={icon} size={chrome.icon} color={fg} />
            }
            if let Some(caption) = shown {
                <text size={chrome.font} class="font-medium" color={fg}>{caption}</text>
            }
            {?shortcut}
        </div>
    }
}

pub fn menu_item(control: Control, chrome: &Chrome) -> AnyElement {
    let focus = control.focus();
    let action = control.action.clone();
    let label = control.label.clone();
    let shortcut = control.shortcut.map(|keys| kbd(keys, chrome, chrome.muted));
    view! {
        <div
            class="w-full flex-row items-center"
            h={chrome.height}
            px={chrome.pad_x}
            gap={chrome.gap}
            rounded={chrome.radius}
            hover_bg={chrome.ghost_hover}
            cursor={CursorHint::Pointer}
            on:click={action.clone()}
            on:key:enter={action.clone()}
            on:key:space={action}
            focus_ring={focus}
            role="menuitem"
            aria-label={label.clone()}
            test_id={control.id.clone()}
        >
            if let Some(icon) = control.icon {
                <icon svg={icon} size={chrome.icon} color={chrome.accent} />
            }
            <text size={chrome.font} color={chrome.text}>{label}</text>
            <div class="flex-1" />
            {?shortcut}
        </div>
    }
}

pub fn row(controls: Vec<Control>, chrome: &Chrome) -> AnyElement {
    let gap = chrome.gap;
    let items: Vec<AnyElement> = controls
        .into_iter()
        .map(|control| render(control, chrome))
        .collect();
    view! {
        <div class="flex-row items-center shrink-0" gap={gap}>
            {...items}
        </div>
    }
}
