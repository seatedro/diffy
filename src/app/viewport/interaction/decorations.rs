use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use diffy_ui::diff_view::decorator::{
    AnnotationContext, DiffDecorator, GutterContext, HeaderContext, HeaderSlot, LineContext,
    LineMark, SeparatorContext,
};
use quark::view;
use quark_components::{Button, ButtonSize, SegmentedControl, SegmentedItem};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::scopes::SharedScopes;
use crate::app::tokens::{self, Ico, Layout, Sp};
use crate::contracts::commands::{CommandsAction, ContextMenuEntry};
use crate::contracts::shell::{Action, ShellAction};
use crate::contracts::viewport::interaction::InteractionAction;
use crate::core::compare::LayoutMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderControls {
    pub layout: LayoutMode,
    pub wrap: bool,
    pub search_open: bool,
    pub scopes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Palette {
    scale: f32,
    small: f32,
    muted: Color,
    icon: Color,
    accent: Color,
    hover: Color,
    selected: Color,
    divider: Color,
}

impl Palette {
    fn new(theme: &Theme) -> Self {
        let tc = &theme.colors;
        Self {
            scale: tokens::ui_scale(theme),
            small: tokens::TextStyle::BodySmall.font_size(theme),
            muted: tc.text_muted,
            icon: tc.icon,
            accent: tc.accent,
            hover: tc.ghost_element_hover,
            selected: tc.ghost_element_selected,
            divider: tc.border_variant,
        }
    }

    fn key(&self) -> [u64; 8] {
        let c = |c: Color| u64::from(u32::from_le_bytes([c.r, c.g, c.b, c.a]));
        [
            u64::from(self.scale.to_bits()),
            u64::from(self.small.to_bits()),
            c(self.muted),
            c(self.icon),
            c(self.accent),
            c(self.hover),
            c(self.selected),
            c(self.divider),
        ]
    }
}

pub struct Decorations {
    inner: Rc<dyn DiffDecorator>,
    controls: HeaderControls,
    palette: Palette,
    scopes: SharedScopes,
    revision: u64,
}

impl Decorations {
    pub fn new(
        inner: Rc<dyn DiffDecorator>,
        controls: HeaderControls,
        scopes: SharedScopes,
        theme: &Theme,
    ) -> Self {
        let palette = Palette::new(theme);
        let mut hasher = std::hash::DefaultHasher::new();
        inner.revision().hash(&mut hasher);
        (
            controls.layout == LayoutMode::Split,
            controls.wrap,
            controls.search_open,
            controls.scopes,
        )
            .hash(&mut hasher);
        palette.key().hash(&mut hasher);
        Self {
            revision: hasher.finish(),
            inner,
            controls,
            palette,
            scopes,
        }
    }

    pub fn matches(
        &self,
        inner: &Rc<dyn DiffDecorator>,
        controls: HeaderControls,
        theme: &Theme,
    ) -> bool {
        std::ptr::addr_eq(Rc::as_ptr(&self.inner), Rc::as_ptr(inner))
            && self.inner.revision() == inner.revision()
            && self.controls == controls
            && self.palette == Palette::new(theme)
    }

    fn view_controls(&self, cx: &HeaderContext) -> AnyElement {
        let controls = self.controls;
        let palette = self.palette;
        let scale = palette.scale;
        let unified = controls.layout == LayoutMode::Unified;
        let file = cx.source.map_or(u64::from(cx.file), |s| s.id.0);
        let layout = SegmentedControl::new(vec![
            SegmentedItem::new(
                "Unified",
                Action::from(CommandsAction::SetLayout(LayoutMode::Unified)),
                unified,
            ),
            SegmentedItem::new(
                "Split",
                Action::from(CommandsAction::SetLayout(LayoutMode::Split)),
                !unified,
            ),
        ])
        .id(format!("diff.layout.{file}"));
        let find = Button::new(Action::from(InteractionAction::OpenSearch))
            .icon(lucide::SEARCH)
            .tooltip("Find in diff \u{2318}F")
            .size(ButtonSize::Compact)
            .active(controls.search_open);
        let more = overflow(cx, palette);
        view! {
            <div class="flex-row items-center" gap={tokens::scaled(Sp::XS, scale)}>
                <div
                    w={1.0}
                    h={tokens::scaled(Sp::LG + Sp::XXS, scale)}
                    mx={tokens::scaled(Sp::XS, scale)}
                    bg={palette.divider}
                />
                {layout}
                {icon_toggle(lucide::WRAP_TEXT, "Wrap lines", controls.wrap, CommandsAction::ToggleWrap.into(), palette)}
                {find}
                {more}
            </div>
        }
    }
}

fn icon_toggle(
    icon: &'static str,
    label: &str,
    on: bool,
    action: Action,
    palette: Palette,
) -> AnyElement {
    let scale = palette.scale;
    let size = tokens::scaled(Layout::CONTROL_SM_H, scale);
    let color = if on { palette.accent } else { palette.icon };
    view! {
        <div
            w={size}
            h={size}
            class="flex-row items-center justify-center cursor-pointer"
            rounded={tokens::scaled(Sp::XS, scale)}
            hover_bg={palette.hover}
            @when {on} { bg={palette.selected} }
            accessibility_role={AccessibilityRole::Button}
            accessibility_toggled={on}
            aria-label={label.to_owned()}
            tooltip={label.to_owned()}
            on:click={quark_ui::Action::from(action)}
            test_id="diff.wrap"
        >
            <icon svg={icon} size={Ico::SM} color={color} />
        </div>
    }
}

fn overflow(cx: &HeaderContext, palette: Palette) -> AnyElement {
    let scale = palette.scale;
    let mut entries = vec![ContextMenuEntry::item(
        "Expand all context",
        InteractionAction::ExpandAllContext,
    )];
    if !cx.title.contains('\u{2192}') {
        entries.push(ContextMenuEntry::item(
            "Copy path",
            ShellAction::CopyText(cx.title.to_owned()),
        ));
    }
    let size = tokens::scaled(Layout::CONTROL_SM_H, scale);
    let handler = ClickHandler::new(move |event| {
        vec![
            Action::from(CommandsAction::OpenContextMenu {
                entries: entries.clone(),
                x: event.x.round() as i32,
                y: event.y.round() as i32,
            })
            .into(),
        ]
    });
    view! {
        <div
            w={size}
            h={size}
            class="flex-row items-center justify-center cursor-pointer"
            rounded={tokens::scaled(Sp::XS, scale)}
            hover_bg={palette.hover}
            accessibility_role={AccessibilityRole::Button}
            aria-label="More file actions"
            tooltip="More file actions"
            on_click_handler={handler}
            test_id="diff.more"
        >
            <icon svg={lucide::MORE_HORIZONTAL} size={Ico::SM} color={palette.icon} />
        </div>
    }
}

pub struct Plain;

impl DiffDecorator for Plain {
    fn revision(&self) -> u64 {
        0
    }
}

pub fn is_generated(path: &str) -> bool {
    const LOCKFILES: &[&str] = &[
        "Cargo.lock",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "Gemfile.lock",
        "poetry.lock",
        "Pipfile.lock",
        "uv.lock",
        "composer.lock",
        "go.sum",
        "flake.lock",
        "Package.resolved",
        "pubspec.lock",
        "mix.lock",
    ];
    const VENDORED: &[&str] = &["vendor", "node_modules", "third_party", "third-party"];
    let name = path.rsplit('/').next().unwrap_or(path);
    LOCKFILES.contains(&name)
        || name.contains(".min.")
        || path
            .split('/')
            .rev()
            .skip(1)
            .any(|dir| VENDORED.contains(&dir))
}

impl DiffDecorator for Decorations {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        match slot {
            HeaderSlot::Trailing => Some(self.view_controls(cx)),
            HeaderSlot::Collapsed if is_generated(cx.title) => {
                let palette = self.palette;
                Some(view! {
                    <text size={palette.small} color={palette.muted}>
                        "collapsed \u{b7} generated file"
                    </text>
                })
            }
            slot => self.inner.header_slot(slot, cx),
        }
    }

    fn header(&self, cx: &HeaderContext) -> Option<AnyElement> {
        self.inner.header(cx)
    }

    fn separator(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        self.inner.separator(cx)
    }

    fn annotation(&self, cx: &AnnotationContext) -> Option<AnyElement> {
        self.inner.annotation(cx)
    }

    fn gutter_utility(&self, cx: &GutterContext) -> Option<AnyElement> {
        self.inner.gutter_utility(cx)
    }

    fn line_mark(&self, cx: &LineContext) -> Option<LineMark> {
        self.inner.line_mark(cx)
    }

    fn line_badge(&self, cx: &LineContext) -> Option<u32> {
        self.inner.line_badge(cx)
    }

    fn file_state(&self, cx: &HeaderContext) -> Option<String> {
        self.inner.file_state(cx)
    }

    fn hunk_scope(&self, cx: &SeparatorContext) -> Option<Arc<[Arc<str>]>> {
        let own = cx
            .source
            .zip(cx.hunk.as_ref())
            .and_then(|(source, hunk)| self.scopes.borrow().label(source, hunk.hunk));
        own.or_else(|| self.inner.hunk_scope(cx))
    }

    fn hunk_actions(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        self.inner.hunk_actions(cx)
    }

    fn banner(&self, width: f32) -> Option<(f32, AnyElement)> {
        self.inner.banner(width)
    }
}
