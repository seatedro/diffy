use std::collections::BTreeSet;

use diffy_diff::RowKind;
use diffy_ui::diff_view::decorator::{
    DiffDecorator, FileSource, GutterContext, HeaderContext, HeaderSlot, LineContext, LineMark,
    SeparatorContext,
};
use quark::view;
use quark_components::{Button, ButtonSize};
use quark_ui::element::*;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::state::LineKey;
use crate::app::tokens::{self, Sp};
use crate::app::viewport::interaction::hunk_target;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::shared::{FileKey, FileRevision, HunkTarget, Side, SourceTarget};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationStyle {
    pub code_size: f32,
    pub muted: Color,
    pub gap: f32,
    pub inset: f32,
}

impl PresentationStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        let scale = tokens::ui_scale(theme);
        Self {
            code_size: theme.metrics.mono_font_size,
            muted: theme.colors.text_muted,
            gap: tokens::scaled(Sp::XS, scale),
            inset: tokens::scaled(Sp::SM, scale),
        }
    }
}

impl Default for PresentationStyle {
    fn default() -> Self {
        Self::from_theme(&Theme::default_dark())
    }
}

#[derive(Debug, Default)]
pub struct MutationPresentation {
    revision: u64,
    style: PresentationStyle,
    staging: bool,
    partial: bool,
    selection: BTreeSet<LineKey>,
}

fn file_revision(source: FileSource) -> FileRevision {
    FileRevision(source.revision.0)
}

impl MutationPresentation {
    pub fn new(
        revision: u64,
        style: PresentationStyle,
        staging: bool,
        partial: bool,
        selection: BTreeSet<LineKey>,
    ) -> Self {
        Self {
            revision,
            style,
            staging,
            partial,
            selection,
        }
    }

    pub fn style(&self) -> PresentationStyle {
        self.style
    }

    pub fn restyled(&self, style: PresentationStyle) -> Self {
        Self {
            revision: self.revision.wrapping_add(1),
            style,
            staging: self.staging,
            partial: self.partial,
            selection: self.selection.clone(),
        }
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.style == other.style
            && self.staging == other.staging
            && self.partial == other.partial
            && self.selection == other.selection
    }

    pub fn hunk_actions(&self, target: &HunkTarget) -> Vec<(&'static str, MutationsAction)> {
        if !self.partial {
            return Vec::new();
        }
        let mut actions = Vec::with_capacity(3);
        if self.staging {
            actions.push(("Stage hunk", MutationsAction::StageHunkAt(target.clone())));
            actions.push((
                "Unstage hunk",
                MutationsAction::UnstageHunkAt(target.clone()),
            ));
        }
        actions.push((
            "Discard hunk",
            MutationsAction::DiscardHunkAt(target.clone()),
        ));
        actions
    }

    pub fn file_actions(&self, file: FileKey) -> Vec<(&'static str, MutationsAction)> {
        let mut actions = Vec::with_capacity(3);
        if self.staging {
            actions.push(("Stage file", MutationsAction::StageFile(file)));
            actions.push(("Unstage file", MutationsAction::UnstageFile(file)));
        }
        actions.push(("Discard file", MutationsAction::DiscardFile(file)));
        actions
    }

    pub fn is_selected(&self, source: FileSource, side: Side, line: u32) -> bool {
        self.selection.contains(&LineKey {
            file: source.id,
            revision: file_revision(source),
            side,
            line,
        })
    }
}

pub fn action_buttons(actions: Vec<(&'static str, MutationsAction)>, gap: f32) -> AnyElement {
    view! {
        <div class="flex-row items-center shrink-0" gap={gap}>
            for (label, action) in actions key={label} {
                <div>{Button::new(action).label(label).size(ButtonSize::Compact)}</div>
            }
        </div>
    }
}

fn changed(kind: RowKind) -> bool {
    matches!(kind, RowKind::Added | RowKind::Removed | RowKind::Modified)
}

impl DiffDecorator for MutationPresentation {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        if slot != HeaderSlot::Actions {
            return None;
        }
        let source = cx.source?;
        Some(action_buttons(self.file_actions(source.id), self.style.gap))
    }

    fn separator(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        if cx.gap.is_some() {
            return None;
        }
        let target = hunk_target(cx.hunk.as_ref()?);
        let actions = self.hunk_actions(&target);
        if actions.is_empty() {
            return None;
        }
        let title = cx.title.to_owned();
        let style = self.style;
        let left = cx
            .columns
            .sides
            .iter()
            .flatten()
            .next()
            .map_or(style.inset, |column| column.text_x);
        Some(view! {
            <div
                w={cx.width}
                h={cx.height}
                class="flex-row items-center"
                pl={left}
                pr={style.inset}
                gap={style.inset}
                test_id="mutations.hunk"
            >
                <div class="flex-1" min_w={0.0}>
                    <text size={style.code_size} color={style.muted} class="truncate font-mono">{title}</text>
                </div>
                {action_buttons(actions, style.gap)}
            </div>
        })
    }

    fn gutter_utility(&self, cx: &GutterContext) -> Option<AnyElement> {
        if !self.partial || !changed(cx.kind) {
            return None;
        }
        let source = cx.source?;
        let target = SourceTarget {
            file: source.id,
            revision: file_revision(source),
            side: cx.side,
            lines: cx.line..cx.line + 1,
        };
        let label = if self.is_selected(source, cx.side, cx.line) {
            "Deselect line"
        } else {
            "Select line"
        };
        Some(view! {
            <div w={cx.width} h={cx.height} class="flex-row items-center justify-center">
                {Button::new(MutationsAction::ToggleLineSelection(target)).label(label).size(ButtonSize::Compact)}
            </div>
        })
    }

    fn line_mark(&self, cx: &LineContext) -> Option<LineMark> {
        let source = cx.source?;
        self.is_selected(source, cx.side, cx.line)
            .then_some(LineMark::Selected)
    }
}
