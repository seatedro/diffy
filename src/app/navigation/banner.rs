use std::rc::Rc;

use diffy_ui::diff_view::decorator::DiffDecorator;
use quark::view;
use quark_components::{AvatarImage, SegmentedControl, SegmentedItem, avatar};
use quark_ui::accessibility::AccessibilityRole;
use quark_ui::element::*;
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::NavigationController;
use super::view::{now_secs, pill};
use crate::app::review::ReviewController;
use crate::app::tokens::{self, Ico, Layout, Sp, TextStyle};
use crate::contracts::commands::CommandsAction;
use crate::contracts::navigation::NavigationAction;
use crate::contracts::shell::{Action, ShellAction};
use crate::contracts::viewport::pipeline::FileStats;
use crate::core::compare::LayoutMode;
use crate::core::ref_catalog::relative_age;

#[derive(Debug, Clone)]
pub struct CommitBanner {
    summary: String,
    author: String,
    byline: String,
    short: String,
    revision: String,
    files: usize,
    totals: Option<(u32, u32)>,
    unified: bool,
    image: Option<AvatarImage>,
}

impl PartialEq for CommitBanner {
    fn eq(&self, other: &Self) -> bool {
        self.summary == other.summary
            && self.author == other.author
            && self.byline == other.byline
            && self.revision == other.revision
            && self.files == other.files
            && self.totals == other.totals
            && self.unified == other.unified
            && self.image.as_ref().map(|i| i.cache_key) == other.image.as_ref().map(|i| i.cache_key)
    }
}

pub fn banner_height(theme: &Theme) -> f32 {
    let scale = tokens::ui_scale(theme);
    tokens::scaled(Sp::MD, scale) * 3.0
        + tokens::scaled(Layout::AVATAR_MD, scale).max(TextStyle::Heading.font_size(theme) * 2.2)
        + theme.metrics.ui_row_height.round()
}

pub struct NavigationDecorator {
    revision: u64,
    banner: Option<CommitBanner>,
    height: f32,
}

impl std::fmt::Debug for NavigationDecorator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavigationDecorator")
            .field("revision", &self.revision)
            .field("banner", &self.banner.is_some())
            .finish()
    }
}

impl DiffDecorator for NavigationDecorator {
    fn revision(&self) -> u64 {
        self.revision
    }

    fn banner(&self, _width: f32) -> Option<(f32, AnyElement)> {
        let banner = self.banner.clone()?;
        Some((self.height, banner.into_any()))
    }
}

pub type BannerCache = Option<(Option<CommitBanner>, u32, Rc<NavigationDecorator>)>;

impl NavigationController {
    pub fn decorator(
        &self,
        review: &ReviewController,
        unified: bool,
        theme: &Theme,
    ) -> Rc<dyn DiffDecorator> {
        let model = self.banner_model(review, unified);
        let height = banner_height(theme).to_bits();
        let mut cache = self.banner_cache().borrow_mut();
        if let Some((cached, cached_height, decorator)) = &*cache
            && *cached == model
            && *cached_height == height
        {
            return decorator.clone();
        }
        let revision = cache
            .as_ref()
            .map_or(1, |(_, _, d)| d.revision.wrapping_add(1));
        let decorator = Rc::new(NavigationDecorator {
            revision,
            banner: model.clone(),
            height: f32::from_bits(height),
        });
        *cache = Some((model, height, decorator.clone()));
        decorator
    }

    fn banner_model(&self, review: &ReviewController, unified: bool) -> Option<CommitBanner> {
        let selected = self.history().selected.as_deref()?;
        let commit = self
            .history_commits()?
            .iter()
            .find(|c| c.revision.id.starts_with(selected) || c.short_revision == selected)?;
        let age = relative_age(now_secs(), commit.timestamp);
        let byline = if age.is_empty() {
            format!("{} committed", commit.author_name)
        } else {
            format!("{} committed {age} ago", commit.author_name)
        };
        let (files, totals) = self
            .manifest()
            .map(|m| {
                let totals = match m.totals {
                    FileStats::Known {
                        additions,
                        deletions,
                    } => Some((additions, deletions)),
                    FileStats::Pending => None,
                };
                (m.files.len(), totals)
            })
            .unwrap_or_default();
        Some(CommitBanner {
            summary: commit.summary.clone(),
            author: commit.author_name.clone(),
            byline,
            short: commit.short_revision.clone(),
            revision: commit.revision.id.clone(),
            files,
            totals,
            unified,
            image: review.author_face(""),
        })
    }
}

impl RenderOnce for CommitBanner {
    fn render(self, cx: &ElementContext) -> AnyElement {
        let theme = cx.theme;
        let tc = &theme.colors;
        let scale = tokens::ui_scale(theme);
        let small = TextStyle::BodySmall.font_size(theme);
        let mono = TextStyle::MonoSmall.font_size(theme);
        let face = avatar(self.author.clone())
            .size(Layout::AVATAR_MD)
            .image(self.image.clone());
        let layout = SegmentedControl::new(vec![
            SegmentedItem::new(
                "Unified",
                Action::from(CommandsAction::SetLayout(LayoutMode::Unified)),
                self.unified,
            ),
            SegmentedItem::new(
                "Split",
                Action::from(CommandsAction::SetLayout(LayoutMode::Split)),
                !self.unified,
            ),
        ])
        .id("commit-header.layout");
        let noun = if self.files == 1 { "file" } else { "files" };
        view! {
            <div
                class="flex-col w-full"
                h={banner_height(theme)}
                gap={tokens::scaled(Sp::MD, scale)}
                px={tokens::scaled(Sp::LG, scale)}
                py={tokens::scaled(Sp::MD, scale)}
                bg={tc.title_bar_background}
                border_b={tc.border_variant}
                accessibility_role={AccessibilityRole::Region}
                aria-label={format!("Commit {}", self.short)}
                test_id="commit-header"
            >
                <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::MD, scale)}>
                    {face}
                    <div class="flex-1 flex-col" min_w={0.0} gap={tokens::scaled(Sp::XXS, scale)}>
                        <text size={TextStyle::Heading.font_size(theme)} class="truncate font-semibold" color={tc.text_strong}>{self.summary.clone()}</text>
                        <text size={small} class="truncate" color={tc.text_muted}>{self.byline.clone()}</text>
                    </div>
                    <div
                        class="flex-row items-center shrink-0"
                        gap={tokens::scaled(Sp::XS, scale)}
                        h={theme.metrics.ui_row_height}
                        px={tokens::scaled(Sp::SM, scale)}
                        rounded={theme.metrics.control_radius}
                        bg={tc.element_background}
                        border={tc.control_border}
                        hover_bg={tc.element_hover}
                        cursor={CursorHint::Pointer}
                        id="commit-header.copy"
                        role="button"
                        aria-label={format!("Copy {}", self.short)}
                        on:click={ShellAction::CopyText(self.revision.clone())}
                    >
                        <text size={mono} class="font-mono" color={tc.text}>{self.short.clone()}</text>
                        <icon svg={lucide::COPY} size={Ico::XS} color={tc.icon} />
                    </div>
                    <div
                        class="flex-row items-center shrink-0"
                        h={theme.metrics.ui_row_height}
                        px={tokens::scaled(Sp::MD, scale)}
                        rounded={theme.metrics.control_radius}
                        bg={tc.element_background}
                        border={tc.control_border}
                        hover_bg={tc.element_hover}
                        cursor={CursorHint::Pointer}
                        id="commit-header.compare-worktree"
                        role="button"
                        aria-label="Compare with working tree"
                        on:click={NavigationAction::CompareWithWorkingTree(self.revision.clone())}
                    >
                        <text size={small} class="font-medium" color={tc.text}>"Compare with working tree"</text>
                    </div>
                </div>
                <div class="flex-row items-center w-full" gap={tokens::scaled(Sp::LG, scale)}>
                    <text size={small} color={tc.text_muted}>{format!("{} {noun}", self.files)}</text>
                    {?self.totals.map(|(a, d)| view! {
                        <div class="flex-row" gap={tokens::scaled(Sp::XS, scale)}>
                            <text size={mono} color={tc.line_add_text}>{format!("+{a}")}</text>
                            <text size={mono} color={tc.line_del_text}>{format!("\u{2212}{d}")}</text>
                        </div>
                    })}
                    {pill("Read-only", theme)}
                    <div class="flex-1" />
                    <div class="shrink-0">{layout}</div>
                </div>
            </div>
        }
    }
}
