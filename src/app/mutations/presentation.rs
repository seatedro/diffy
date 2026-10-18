use std::collections::{BTreeSet, HashMap};

use diffy_diff::RowKind;
use diffy_ui::diff_view::decorator::{
    DiffDecorator, FileSource, GutterContext, HeaderContext, HeaderSlot, LineContext, LineMark,
    SeparatorContext,
};
use quark::view;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::controls::{self, Chrome, Control, Tone};
use super::state::LineKey;
use crate::app::tokens::{self, Sp};
use crate::app::viewport::interaction::hunk_target;
use crate::contracts::mutations::MutationsAction;
use crate::contracts::shared::{FileKey, FileRevision, HunkTarget, Side, SourceTarget};
use crate::contracts::viewport::CompareManifestSnapshot;
use crate::core::vcs::model::ChangeBucket;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationStyle {
    pub code_size: f32,
    pub muted: Color,
    pub inset: f32,
    pub chrome: Chrome,
}

impl PresentationStyle {
    pub fn from_theme(theme: &Theme) -> Self {
        let scale = tokens::ui_scale(theme);
        Self {
            code_size: theme.metrics.mono_font_size,
            muted: theme.colors.text_muted,
            inset: tokens::scaled(Sp::SM, scale),
            chrome: Chrome::from_theme(theme),
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
    partial: bool,
    buckets: HashMap<FileKey, Vec<ChangeBucket>>,
    selection: BTreeSet<LineKey>,
}

fn file_revision(source: FileSource) -> FileRevision {
    FileRevision(source.revision.0)
}

pub fn mutable_buckets(
    manifest: Option<&CompareManifestSnapshot>,
) -> HashMap<FileKey, Vec<ChangeBucket>> {
    manifest
        .filter(|manifest| manifest.working_tree)
        .map(|manifest| {
            manifest
                .files
                .iter()
                .filter(|file| !file.buckets.is_empty())
                .map(|file| (file.key, file.buckets.clone()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn file_controls(file: FileKey, buckets: &[ChangeBucket]) -> Vec<Control> {
    let id = |name: &str| format!("mutations.file.{}.{name}", file.0);
    let mut controls: Vec<Control> = Vec::new();
    for bucket in buckets {
        let next = match bucket {
            ChangeBucket::Unstaged => vec![
                Control::new(
                    id("stage"),
                    "Stage file",
                    MutationsAction::StageFile(file),
                    Tone::Secondary,
                )
                .icon(lucide::PLUS)
                .shortcut("S"),
                Control::new(
                    id("discard"),
                    "Discard\u{2026}",
                    MutationsAction::DiscardFile(file),
                    Tone::Destructive,
                )
                .icon(lucide::CORNER_UP_LEFT),
            ],
            ChangeBucket::Staged => vec![
                Control::new(
                    id("unstage"),
                    "Unstage file",
                    MutationsAction::UnstageFile(file),
                    Tone::Secondary,
                )
                .icon(lucide::MINUS),
            ],
            ChangeBucket::Untracked => vec![
                Control::new(
                    id("stage"),
                    "Stage file",
                    MutationsAction::StageFile(file),
                    Tone::Secondary,
                )
                .icon(lucide::PLUS)
                .shortcut("S"),
                Control::new(
                    id("delete"),
                    "Delete\u{2026}",
                    MutationsAction::DiscardFile(file),
                    Tone::Destructive,
                )
                .icon(lucide::X),
            ],
            ChangeBucket::WorkingCopy => vec![
                Control::new(
                    id("discard"),
                    "Discard\u{2026}",
                    MutationsAction::DiscardFile(file),
                    Tone::Destructive,
                )
                .icon(lucide::CORNER_UP_LEFT),
            ],
            ChangeBucket::Conflicted => Vec::new(),
        };
        for control in next {
            if !controls.iter().any(|existing| existing.id == control.id) {
                controls.push(control);
            }
        }
    }
    controls
}

pub fn hunk_controls(target: &HunkTarget, buckets: &[ChangeBucket]) -> Vec<Control> {
    let key = format!(
        "mutations.hunk.{}.{}.{}",
        target.source.file.0, target.source.revision.0, target.source.lines.start
    );
    let mut controls: Vec<Control> = Vec::new();
    for bucket in buckets {
        let next = match bucket {
            ChangeBucket::Unstaged => vec![
                Control::new(
                    format!("{key}.stage"),
                    "Stage hunk",
                    MutationsAction::StageHunkAt(target.clone()),
                    Tone::Secondary,
                ),
                Control::new(
                    format!("{key}.discard"),
                    "Discard hunk",
                    MutationsAction::DiscardHunkAt(target.clone()),
                    Tone::Destructive,
                ),
            ],
            ChangeBucket::Staged => vec![Control::new(
                format!("{key}.unstage"),
                "Unstage hunk",
                MutationsAction::UnstageHunkAt(target.clone()),
                Tone::Secondary,
            )],
            ChangeBucket::Untracked => vec![Control::new(
                format!("{key}.stage"),
                "Stage hunk",
                MutationsAction::StageHunkAt(target.clone()),
                Tone::Secondary,
            )],
            ChangeBucket::WorkingCopy => vec![Control::new(
                format!("{key}.discard"),
                "Discard hunk",
                MutationsAction::DiscardHunkAt(target.clone()),
                Tone::Destructive,
            )],
            ChangeBucket::Conflicted => Vec::new(),
        };
        for control in next {
            if !controls.iter().any(|existing| existing.id == control.id) {
                controls.push(control);
            }
        }
    }
    controls
}

impl MutationPresentation {
    pub fn new(
        revision: u64,
        style: PresentationStyle,
        partial: bool,
        buckets: HashMap<FileKey, Vec<ChangeBucket>>,
        selection: BTreeSet<LineKey>,
    ) -> Self {
        Self {
            revision,
            style,
            partial,
            buckets,
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
            partial: self.partial,
            buckets: self.buckets.clone(),
            selection: self.selection.clone(),
        }
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.style == other.style
            && self.partial == other.partial
            && self.buckets == other.buckets
            && self.selection == other.selection
    }

    pub fn buckets(&self, file: FileKey) -> &[ChangeBucket] {
        self.buckets.get(&file).map_or(&[], Vec::as_slice)
    }

    pub fn hunk_actions(&self, target: &HunkTarget) -> Vec<Control> {
        if !self.partial {
            return Vec::new();
        }
        hunk_controls(target, self.buckets(target.source.file))
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
        let controls = file_controls(source.id, self.buckets(source.id));
        if controls.is_empty() {
            return None;
        }
        Some(controls::row(controls, &self.style.chrome))
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
                {controls::row(actions, &style.chrome)}
            </div>
        })
    }

    fn gutter_utility(&self, cx: &GutterContext) -> Option<AnyElement> {
        if !self.partial || !changed(cx.kind) {
            return None;
        }
        let source = cx.source?;
        if self.buckets(source.id).is_empty() {
            return None;
        }
        let target = SourceTarget {
            file: source.id,
            revision: file_revision(source),
            side: cx.side,
            lines: cx.line..cx.line + 1,
        };
        let selected = self.is_selected(source, cx.side, cx.line);
        let (label, action) = if selected {
            (
                "Deselect line",
                MutationsAction::ToggleLineSelection(target),
            )
        } else {
            ("Select line", MutationsAction::SelectLines(target))
        };
        let control = Control::new(
            format!("mutations.line.{}.{:?}.{}", source.id.0, cx.side, cx.line),
            label,
            action,
            if selected { Tone::Primary } else { Tone::Ghost },
        )
        .icon_only(if selected {
            lucide::CHECK
        } else {
            lucide::PLUS
        });
        let mut chrome = self.style.chrome;
        chrome.height = cx.height.min(chrome.height);
        chrome.muted = chrome.accent;
        Some(view! {
            <div w={cx.width} h={cx.height} class="flex-row items-center justify-end">
                {controls::render(control, &chrome)}
            </div>
        })
    }

    fn line_mark(&self, cx: &LineContext) -> Option<LineMark> {
        let source = cx.source?;
        self.is_selected(source, cx.side, cx.line)
            .then_some(LineMark::Selected)
    }
}
