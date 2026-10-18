use std::collections::BTreeSet;

use diffy_diff::RowKind;
use diffy_ui::diff_view::decorator::{
    DiffDecorator, GutterContext, HeaderContext, HeaderSlot, SeparatorContext,
};
use quark::view;
use quark_components::{Button, ButtonSize};
use quark_ui::element::*;
use quark_ui::style::Styled;

use super::state::LineKey;
use quark_ui::theme::{Color, Theme};

use crate::app::tokens::{self, Sp};
use crate::contracts::mutations::MutationsAction;
use crate::contracts::shared::{FileKey, FileRevision, HunkTarget, Side, SourceTarget};
use crate::core::vcs::patch::lines_before;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedFile {
    pub key: FileKey,
    pub revision: FileRevision,
    pub path: String,
}

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
    files: Vec<PresentedFile>,
    selection: BTreeSet<LineKey>,
}

impl MutationPresentation {
    pub fn new(
        revision: u64,
        style: PresentationStyle,
        staging: bool,
        partial: bool,
        files: Vec<PresentedFile>,
        selection: BTreeSet<LineKey>,
    ) -> Self {
        Self {
            revision,
            style,
            staging,
            partial,
            files,
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
            files: self.files.clone(),
            selection: self.selection.clone(),
        }
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.style == other.style
            && self.staging == other.staging
            && self.partial == other.partial
            && self.files == other.files
            && self.selection == other.selection
    }

    fn file_at(&self, index: u32) -> Option<&PresentedFile> {
        self.files.get(index as usize)
    }

    fn file_titled(&self, title: &str) -> Option<&PresentedFile> {
        let path = title.rsplit(" \u{2192} ").next().unwrap_or(title);
        self.files.iter().find(|file| file.path == path)
    }

    pub fn hunk_actions(&self, target: &HunkTarget) -> Vec<(&'static str, MutationsAction)> {
        if !self.partial {
            return Vec::new();
        }
        let mut actions = Vec::with_capacity(3);
        if self.staging {
            actions.push(("Stage hunk", MutationsAction::StageHunkAt(target.clone())));
            actions.push(("Unstage hunk", MutationsAction::UnstageHunkAt(target.clone())));
        }
        actions.push(("Discard hunk", MutationsAction::DiscardHunkAt(target.clone())));
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

    pub fn is_selected(&self, file: FileKey, revision: FileRevision, side: Side, line: u32) -> bool {
        self.selection.contains(&LineKey {
            file,
            revision,
            side,
            line,
        })
    }
}

pub fn hunk_target_from_title(file: &PresentedFile, title: &str) -> Option<HunkTarget> {
    let rest = &title[title.find("@@ -")? + 4..];
    let ranges = &rest[..rest.find(" @@")?];
    let (old, new) = ranges.split_once(" +")?;
    let parse = |range: &str| -> Option<(u32, u32)> {
        match range.split_once(',') {
            Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
            None => Some((range.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = parse(old)?;
    let (new_start, new_len) = parse(new)?;
    let (side, start, len) = if new_len > 0 {
        (Side::New, new_start, new_len)
    } else {
        (Side::Old, old_start, old_len)
    };
    let before = lines_before(start, len);
    Some(HunkTarget {
        source: SourceTarget {
            file: file.key,
            revision: file.revision,
            side,
            lines: before..before + len,
        },
        hunk: 0,
    })
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

impl DiffDecorator for MutationPresentation {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn header_slot(&self, slot: HeaderSlot, cx: &HeaderContext) -> Option<AnyElement> {
        if slot != HeaderSlot::Actions {
            return None;
        }
        let file = self.file_titled(cx.title)?;
        Some(action_buttons(self.file_actions(file.key), self.style.gap))
    }

    fn separator(&self, cx: &SeparatorContext) -> Option<AnyElement> {
        if cx.gap.is_some() {
            return None;
        }
        let file = self.file_at(cx.file)?;
        let target = hunk_target_from_title(file, cx.title)?;
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
        if !self.partial || !matches!(cx.kind, RowKind::Added | RowKind::Removed | RowKind::Modified) {
            return None;
        }
        let file = self.file_at(cx.file)?;
        let target = SourceTarget {
            file: file.key,
            revision: file.revision,
            side: cx.side,
            lines: cx.line..cx.line + 1,
        };
        let label = if self.is_selected(file.key, file.revision, cx.side, cx.line) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_titles_map_to_source_coverage() {
        let file = PresentedFile {
            key: FileKey(3),
            revision: FileRevision(2),
            path: "f".to_owned(),
        };
        let target = hunk_target_from_title(&file, "@@ -10,4 +12,6 @@ fn main").unwrap();
        assert_eq!(target.source.side, Side::New);
        assert_eq!(target.source.lines, 11..17);
        let deletion = hunk_target_from_title(&file, "@@ -3,2 +2,0 @@").unwrap();
        assert_eq!(deletion.source.side, Side::Old);
        assert_eq!(deletion.source.lines, 2..4);
        assert!(hunk_target_from_title(&file, "4 unchanged lines").is_none());
    }
}
