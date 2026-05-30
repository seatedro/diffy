use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::actions::{Action, AppAction, ContextMenuEntry, GitHubAction};
use crate::core::forge::github::{GitHubReviewSide, PullRequestReviewComment};
use crate::core::review::{
    ReviewComment, ReviewReactionGroup, ReviewResolution, ReviewSide, ReviewThread, ReviewThreadId,
};
use crate::render::{
    BorderPrimitive, FontKind, FontWeight, Rect, RoundedRectPrimitive, ShadowPrimitive,
    TextPrimitive,
};
use crate::ui::components::avatar::AvatarImage;
use crate::ui::components::{Button, ButtonSize, ButtonStyle, avatar};
use crate::ui::design::{Ico, Rad, Shadow, Sp};
use crate::ui::editor::decoration::{
    BlockDecoration, BlockPaintCtx, BlockPlacement, BlockRegistry,
};
use crate::ui::editor::display_layout::DisplayLayoutMetrics;
use crate::ui::editor::render_doc::{INVALID_U32, RenderDoc, RenderRowKind};
use crate::ui::element::*;
use crate::ui::icons::lucide;
use crate::ui::shell::CursorHint;
use crate::ui::style::Styled;
use crate::ui::theme::{Color, Theme};
use halogen::view;

pub(crate) const MAX_VISIBLE_COMMENTS_PER_THREAD: usize = 3;
pub(crate) const MAX_BODY_LINES_PER_COMMENT: usize = 3;
pub(crate) const PANEL_MAX_WIDTH: f32 = 760.0;
pub(crate) const REVIEW_AVATAR_PX: f32 = 22.0;
/// Pixel size requested when fetching comment-author avatars. Must be used at BOTH
/// the fetch site (`enqueue_review_avatar_fetches`) and the render lookup so the
/// `avatar_cache_key` matches; the bitmap is downscaled to `REVIEW_AVATAR_PX`.
pub(crate) const REVIEW_AVATAR_FETCH_PX: u32 = 128;

#[derive(Debug, Clone)]
pub struct ReviewThreadBlock {
    thread: ReviewThread,
    expanded: bool,
    measured_height: u16,
}

impl ReviewThreadBlock {
    pub fn new(thread: ReviewThread, expanded: bool, measured_height: u16) -> Self {
        Self {
            thread,
            expanded,
            measured_height,
        }
    }

    fn accessibility_summary(&self) -> String {
        let (status_word, status_meta) = thread_status_parts(&self.thread);
        let status_label = if status_meta.is_empty() {
            status_word.to_owned()
        } else {
            format!("{status_word}, {status_meta}")
        };
        let line_label = thread_line_label(&self.thread);
        let (author, snippet) = first_comment_summary(&self.thread);
        let state = if self.expanded {
            "expanded"
        } else {
            "collapsed"
        };
        let toggle = if self.expanded { "collapse" } else { "expand" };
        format!(
            "Review thread, {line_label}, {status_label}, {state}. @{author}: {snippet}. Activate to {toggle}."
        )
    }
}

pub(crate) fn visible_comment_count(thread: &ReviewThread) -> usize {
    thread.comments.len().min(MAX_VISIBLE_COMMENTS_PER_THREAD)
}

pub(crate) fn thread_has_footer(thread: &ReviewThread) -> bool {
    thread.permissions.can_reply
        || thread.permissions.can_resolve
        || thread.permissions.can_unresolve
}

pub(crate) fn first_comment_summary(thread: &ReviewThread) -> (String, String) {
    match thread.comments.first() {
        Some(comment) => {
            let author = comment
                .author_login
                .as_deref()
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown")
                .to_owned();
            let snippet = clean_comment_body(&comment.body)
                .into_iter()
                .next()
                .unwrap_or_else(|| "(empty comment)".to_owned());
            (author, snippet)
        }
        None => ("unknown".to_owned(), "(no comments)".to_owned()),
    }
}

/// Generous over-estimate used ONLY when the precompute measure pass did not supply
/// a height for this thread (e.g. first frame before the viewport is sized). Over-
/// reserving leaves a harmless gap; under-reserving would let the card overlap the
/// code below, so this deliberately rounds up.
pub(crate) fn estimate_thread_height_px(thread: &ReviewThread, expanded: bool) -> u16 {
    let mut rows = 1_u32; // header
    if expanded {
        for comment in thread.comments.iter().take(visible_comment_count(thread)) {
            let body_lines = clean_comment_body(&comment.body).len().max(1) as u32;
            rows = rows.saturating_add(1 + body_lines);
            if !comment.reactions.is_empty() {
                rows = rows.saturating_add(1);
            }
        }
        if thread.comments.len() > visible_comment_count(thread) {
            rows = rows.saturating_add(1);
        }
        if thread_has_footer(thread) {
            rows = rows.saturating_add(2);
        }
    } else {
        rows = rows.saturating_add(1);
    }
    // ~30px per row + slack, comfortably above the real element row height.
    rows.saturating_add(2)
        .saturating_mul(30)
        .min(u32::from(u16::MAX)) as u16
}

fn build_reaction_chips(
    reactions: &[ReviewReactionGroup],
    theme: &Theme,
    ui_scale: f32,
) -> AnyElement {
    let tc = &theme.colors;
    let small = theme.metrics.ui_small_font_size;
    let chips: Vec<AnyElement> = reactions
        .iter()
        .filter(|r| r.count > 0)
        .take(6)
        .map(|r| {
            let label = format!("{} {}", reaction_emoji(&r.content), r.count);
            let border = if r.viewer_has_reacted {
                tc.accent
            } else {
                tc.border_soft
            };
            view! { ui_scale,
                <div class="flex-row items-center"
                     px={Sp::XS} py={Sp::XXS}
                     rounded={Rad::SM}
                     bg={tc.element_background}
                     border_t={border} border_b={border} border_l={border} border_r={border}>
                    {text(label).size(small * 0.92).color(tc.text_muted)}
                </div>
            }
        })
        .collect();
    view! { ui_scale,
        <div class="flex-row items-center" gap={Sp::XS}>
            {...chips}
        </div>
    }
}

/// Builds the review-thread card as a real `view!` element tree. Strictly
/// content-sized on the vertical axis (no `h=`, no vertical grow, spacers only in
/// horizontal rows) so `compute_layout` against a tall sentinel returns the true
/// summed height instead of saturating. `card_width` is the pinned width used
/// identically at measure and render so wrapped line counts match.
/// Stable identity for a single comment's selectable body, surviving re-wrap and
/// re-render. Combines the thread id with the comment's index in the thread.
pub(crate) fn card_source_key(thread_id: &ReviewThreadId, comment_index: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    thread_id.hash(&mut hasher);
    comment_index.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn build_review_thread_card(
    thread: &ReviewThread,
    expanded: bool,
    theme: &Theme,
    ui_scale: f32,
    card_width: f32,
    avatars: &HashMap<u64, AvatarImage>,
    selection: Option<&crate::ui::state::CardTextSelection>,
) -> AnyElement {
    let tc = &theme.colors;
    let small = theme.metrics.ui_small_font_size;
    let resolved = thread.status.resolution == ReviewResolution::Resolved;
    let status_color = resolution_color(thread.status.resolution, tc);
    let status_bg = Color::rgba(status_color.r, status_color.g, status_color.b, 30);
    let (status_word, _meta) = thread_status_parts(thread);

    // Discoverable actions menu (the `…` button) — opens the same entries as right-click,
    // anchored at the click position via a position-aware handler.
    let menu_entries = review_context_menu_entries(thread);
    let menu_handler = ClickHandler::new(move |e: ClickEvent| {
        ClickResult::Actions(vec![
            AppAction::OpenContextMenu {
                entries: menu_entries.clone(),
                x: e.x.round() as i32,
                y: e.y.round() as i32,
            }
            .into(),
        ])
    });
    let line_label = thread_line_label(thread);
    let header_label = if thread.status.outdated {
        format!("{line_label} · outdated")
    } else {
        line_label
    };
    let chevron = if expanded {
        lucide::CHEVRON_DOWN
    } else {
        lucide::CHEVRON_RIGHT
    };
    // Square chevron button (icon + even padding), pre-scaled because `w`/`h` are not
    // auto-scaled by the macro. The icon (Ico::SM, scaled internally) centers within it.
    let chevron_box = ((Ico::SM + Sp::SM) * ui_scale).round();

    // Body wrap column: card minus horizontal padding, avatar, gaps, reply indent.
    // Match the view! macro, which scales AND rounds spatial attrs (p/gap/pl); `w` is
    // not scaled, so card_width is used as-is. Rounding here keeps the wrap column equal
    // to the painted column so line counts match exactly.
    let pad = (Sp::SM * ui_scale).round();
    let avatar_px = REVIEW_AVATAR_PX * ui_scale;
    let gap = (Sp::XS * ui_scale).round();

    let body: Vec<AnyElement> = if expanded {
        let mut rows: Vec<AnyElement> = Vec::new();
        let visible = visible_comment_count(thread);
        for (idx, comment) in thread.comments.iter().take(visible).enumerate() {
            let author = comment
                .author_login
                .as_deref()
                .filter(|l| !l.is_empty())
                .unwrap_or("unknown")
                .to_owned();
            let ts = relative_time_label(comment.created_at.as_deref());
            let header = match (idx == 0, ts) {
                (true, Some(t)) => format!("@{author} · {t}"),
                (false, Some(t)) => format!("@{author} replied · {t}"),
                (true, None) => format!("@{author}"),
                (false, None) => format!("@{author} replied"),
            };
            let header_color = if resolved || idx != 0 {
                tc.text_muted
            } else {
                tc.text_strong
            };
            let body_color = if resolved { tc.text_muted } else { tc.text };
            let indent = if idx == 0 {
                0.0
            } else {
                (Sp::MD * ui_scale).round()
            };
            // Exact text-column width for THIS row: card inner minus reply indent, the
            // avatar, and the avatar→text gap. Wrapping at this width matches the painted
            // column so lines never run past the card edge.
            let text_w = (card_width - pad * 2.0 - indent - avatar_px - gap).max(40.0);
            let cleaned = clean_comment_body(&comment.body).join(" ");
            // One selectable, self-wrapping text block per comment body. Selection
            // state keys on byte offsets into `cleaned` (re-wrap stable) and is wired
            // in from app state; `None` here renders identically to plain text.
            let source_key = card_source_key(&thread.id, idx);
            let selected_range = selection
                .filter(|sel| sel.source_key == source_key)
                .map(|sel| sel.normalized());
            let body_element: AnyElement = if cleaned.is_empty() {
                text("(empty comment)")
                    .size(small)
                    .color(tc.text_muted)
                    .into_any()
            } else {
                selectable_text(cleaned)
                    .width(text_w)
                    .size(small)
                    .color(body_color)
                    .max_lines(8)
                    .source(source_key)
                    .selection(selected_range)
                    .into_any()
            };
            let reactions = (!comment.reactions.is_empty())
                .then(|| build_reaction_chips(&comment.reactions, theme, ui_scale));
            let avatar_image = resolve_review_avatar(comment.author_avatar_url.as_deref(), avatars);
            rows.push(view! { ui_scale,
                <div class="flex-row items-start" gap={Sp::XS} pl={indent}>
                    {avatar(author).image(avatar_image).size(REVIEW_AVATAR_PX)}
                    <div class="flex-col" gap={Sp::XXS} min_w={0.0}>
                        {text(header).size(small).color(header_color).medium()}
                        {body_element}
                        {?reactions}
                    </div>
                </div>
            });
        }
        let hidden = thread.comments.len().saturating_sub(visible);
        if hidden > 0 {
            let label = if hidden == 1 {
                "1 more reply".to_owned()
            } else {
                format!("{hidden} more replies")
            };
            rows.push(view! { ui_scale,
                <div class="flex-row" pl={Sp::MD}>
                    {text(label).size(small).color(tc.text_muted)}
                </div>
            });
        }
        rows
    } else {
        let (author, snippet) = first_comment_summary(thread);
        vec![view! { ui_scale,
            <div class="flex-row" pl={Sp::MD}>
                {text(format!("@{author}: {snippet}")).size(small).color(tc.text_muted).truncate()}
            </div>
        }]
    };

    let footer = expanded
        .then(|| review_resolution_action(thread))
        .flatten()
        .map(|(target, label)| {
            view! { ui_scale,
                // Just the action, sitting below the conversation with a little breathing
                // room. No divider: a horizontal line inside the bordered card frames the
                // sparse footer row into what looks like an empty text box.
                <div class="flex-row items-center" gap={Sp::XS} pt={Sp::XXS}>
                    <Button action={GitHubAction::SetReviewThreadResolved {
                                id: thread.id.clone(),
                                resolved: target,
                            }.into()}
                            style={ButtonStyle::Subtle}
                            size={ButtonSize::Compact}>
                        <Label>{label}</Label>
                    </Button>
                    <spacer />
                </div>
            }
        });

    // Collapsed: the whole card is a click target that expands (root carries the
    // toggle). Expanded: the root is an inert click sink so clicks on the body do
    // nothing; only the header row collapses. Clicks resolve topmost-first (the
    // element hit-test registers parents before children and searches in reverse),
    // so the Resolve button and header still win over the root within their bounds.
    let toggle: Action = GitHubAction::ToggleReviewThread(thread.id.clone()).into();
    // Only the chevron toggles and highlights on hover. Collapsed: the whole card is still
    // a click target (convenience), but it does NOT highlight — the chevron is the sole
    // hover affordance. Expanded: the card is fully inert; only the chevron collapses it.
    let (root_action, root_cursor) = if expanded {
        (Action::Noop, CursorHint::Default)
    } else {
        (toggle.clone(), CursorHint::Pointer)
    };

    view! { ui_scale,
        <div class="flex-col"
             w={card_width}
             z_index={55}
             bg={tc.elevated_surface}
             border_t={tc.border} border_b={tc.border} border_l={tc.border} border_r={tc.border}
             rounded={Rad::LG}
             shadow_preset={Shadow::DROPDOWN}
             p={Sp::SM}
             gap={Sp::XS}
             on_click={root_action}
             cursor={root_cursor}
             block_mouse>
            // Header row collapses the card when expanded (and also expands when
            // collapsed). The thread action menu is reached via right-click
            // (BlockDecoration::context_menu_entries, preserved). A dedicated left-click
            // menu button is omitted until it can open at the cursor — a button that
            // instead collapsed the card would read as broken.
            <div class="flex-row items-center" gap={Sp::XS}>
                // Only the chevron is the collapse/expand target and the only header
                // element that highlights on hover. A fixed square with the glyph centered
                // (justify-center) keeps it aligned with the header text like GitHub.
                <div class="flex-row items-center justify-center shrink-0"
                     w={chevron_box} h={chevron_box}
                     rounded={Rad::SM}
                     on_click={toggle}
                     cursor={CursorHint::Pointer}
                     hover_bg={tc.element_hover}>
                    <icon svg={chevron} size={Ico::SM} color={tc.text_muted} />
                </div>
                {text(header_label).size(small).color(tc.text_strong).medium().truncate()}
                <spacer />
                <div class="flex-row items-center"
                     px={Sp::SM} py={Sp::XXS}
                     rounded={Rad::SM}
                     bg={status_bg}>
                    {text(status_word).size(small * 0.9).color(status_color).medium()}
                </div>
                <div class="flex-row items-center"
                     px={Sp::XXS} py={Sp::XXS}
                     rounded={Rad::SM}
                     cursor={CursorHint::Pointer}
                     hover_bg={tc.element_hover}
                     on_click_handler={menu_handler}>
                    <icon svg={lucide::MORE_HORIZONTAL} size={Ico::SM} color={tc.text_muted} />
                </div>
            </div>
            {...body}
            {?footer}
        </div>
    }
}

/// Resolves the fetched avatar bitmap for a comment author, keyed by the same
/// sized URL + cache key used at the fetch site so a hit is found. Returns `None`
/// (initials fallback) until the bitmap arrives. Avatars are fixed-size, so this
/// never affects layout — measure and render produce identical heights regardless.
fn resolve_review_avatar(
    raw_url: Option<&str>,
    avatars: &HashMap<u64, AvatarImage>,
) -> Option<AvatarImage> {
    let url = crate::ui::state::avatar_url_sized(raw_url?, REVIEW_AVATAR_FETCH_PX)?;
    let key = crate::ui::state::avatar_cache_key(&url);
    avatars.get(&key).cloned()
}

/// Measures the natural pixel height of the card tree by laying it out against a
/// tall sentinel at the pinned `card_width`. Relies on the card being strictly
/// content-sized vertically (see `build_review_thread_card`).
pub(crate) fn measure_review_thread_card_height(
    thread: &ReviewThread,
    expanded: bool,
    theme: &Theme,
    ui_scale: f32,
    card_width: f32,
    cx: &mut ElementContext,
) -> u16 {
    const LARGE_SENTINEL: f32 = 100_000.0;
    // Avatars do not affect layout (fixed size) and selection only paints a
    // highlight behind text, so measure with neither — height is identical.
    let avatars = HashMap::new();
    let mut card = build_review_thread_card(
        thread,
        expanded,
        theme,
        ui_scale,
        card_width,
        &avatars,
        None,
    );
    let mut engine = LayoutEngine::new();
    let root = card.request_layout(&mut engine, cx);
    engine.compute_layout(root, card_width, LARGE_SENTINEL);
    let h = engine.layout_bounds(root).height.ceil();
    h.max(1.0).min(f32::from(u16::MAX)) as u16
}

impl BlockDecoration for ReviewThreadBlock {
    // Returns the height measured at populate time via compute_layout against the
    // pinned card_width (see shell.rs measure pass). Valid ONLY because blocks are
    // cleared+rebuilt every frame — never cache cross-frame. `metrics` is ignored
    // deliberately: the real card is a view! tree, not a fixed row grid.
    fn height(&self, _metrics: &DisplayLayoutMetrics) -> u16 {
        self.measured_height
    }

    // No-op: the card is rendered as a view! element overlay from shell.rs, after
    // editor.paint, positioned at this block's reserved on-screen rect.
    fn paint(&self, _ctx: &mut BlockPaintCtx) {}

    // Inert: all interaction (toggle, resolve/unresolve, menu) is dispatched by the
    // overlay's element HitRegions. Right-click still routes through context_menu_entries.
    fn on_click(&self) -> Option<Action> {
        None
    }

    fn context_menu_entries(&self) -> Option<Vec<ContextMenuEntry>> {
        Some(review_context_menu_entries(&self.thread))
    }

    fn review_card(&self) -> Option<(&ReviewThread, bool)> {
        Some((&self.thread, self.expanded))
    }

    fn accessibility_label(&self) -> Option<String> {
        Some(self.accessibility_summary())
    }
}

#[derive(Debug, Clone)]
pub struct ReviewCommentBlock {
    comments: Vec<PullRequestReviewComment>,
}

impl ReviewCommentBlock {
    pub fn new(comments: Vec<PullRequestReviewComment>) -> Self {
        Self { comments }
    }

    fn row_count(&self) -> u16 {
        let rows = self
            .comments
            .iter()
            .map(|comment| {
                let body_lines = clean_comment_body(&comment.body).len().max(1);
                1_u32.saturating_add(body_lines as u32)
            })
            .sum::<u32>()
            .saturating_add(1)
            .max(2);
        rows.min(u32::from(u16::MAX)) as u16
    }

    fn accessibility_summary(&self) -> String {
        let count = self.comments.len();
        let mut label = if count == 1 {
            "1 review comment".to_owned()
        } else {
            format!("{count} review comments")
        };
        for comment in self.comments.iter().take(3) {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.login.as_str())
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown");
            let snippet = clean_comment_body(&comment.body)
                .into_iter()
                .next()
                .unwrap_or_else(|| "(empty comment)".to_owned());
            label.push_str("; @");
            label.push_str(author);
            label.push_str(": ");
            label.push_str(&snippet);
        }
        label
    }
}

impl BlockDecoration for ReviewCommentBlock {
    fn height(&self, metrics: &DisplayLayoutMetrics) -> u16 {
        metrics.body_row_height_px.saturating_mul(self.row_count())
    }

    fn paint(&self, ctx: &mut BlockPaintCtx) {
        let panel = review_panel_rect(ctx);
        paint_panel_chrome(ctx, panel);

        let m = &ctx.theme.metrics;
        let mut y = panel.y + m.spacing_sm;
        let x = panel.x + m.spacing_md;
        let w = (panel.width - m.spacing_md * 2.0).max(0.0);
        let line_h = review_line_height(ctx);
        let max_chars = visual_line_char_limit(w, m.ui_small_font_size);
        for comment in &self.comments {
            let author = comment
                .user
                .as_ref()
                .map(|user| user.login.as_str())
                .filter(|login| !login.is_empty())
                .unwrap_or("unknown");
            let header = if comment.line.is_none() && comment.original_line.is_some() {
                format!("@{author} commented on an outdated line")
            } else {
                format!("@{author}")
            };
            ctx.scene.text(TextPrimitive {
                rect: Rect {
                    x,
                    y,
                    width: w,
                    height: line_h,
                },
                text: truncate_display_line(&header, max_chars).into(),
                color: ctx.theme.colors.text_strong,
                font_size: ctx.theme.metrics.ui_small_font_size,
                font_kind: FontKind::Ui,
                font_weight: FontWeight::Medium,
            });
            y += line_h;

            let body = clean_comment_body(&comment.body);
            if body.is_empty() {
                ctx.scene.text(TextPrimitive {
                    rect: Rect {
                        x,
                        y,
                        width: w,
                        height: line_h,
                    },
                    text: "(empty comment)".into(),
                    color: ctx.theme.colors.text_muted,
                    font_size: m.ui_small_font_size,
                    font_kind: FontKind::Ui,
                    font_weight: FontWeight::Normal,
                });
                y += line_h;
            } else {
                for body_line in &body {
                    ctx.scene.text(TextPrimitive {
                        rect: Rect {
                            x,
                            y,
                            width: w,
                            height: line_h,
                        },
                        text: truncate_display_line(body_line, max_chars).into(),
                        color: ctx.theme.colors.text,
                        font_size: m.ui_small_font_size,
                        font_kind: FontKind::Ui,
                        font_weight: FontWeight::Normal,
                    });
                    y += line_h;
                }
            }
        }
    }

    fn accessibility_label(&self) -> Option<String> {
        Some(self.accessibility_summary())
    }
}

pub fn populate_review_thread_blocks(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    file: &carbon::FileDiff,
    threads: &[ReviewThread],
    heights: &std::collections::HashMap<crate::core::review::ReviewThreadId, u16>,
    is_expanded: impl Fn(&ReviewThread) -> bool,
) {
    populate_review_thread_blocks_in_range(
        blocks,
        render_doc,
        file,
        0..render_doc.lines.len(),
        threads,
        heights,
        is_expanded,
    );
}

pub fn populate_review_thread_blocks_in_range(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    file: &carbon::FileDiff,
    line_range: Range<usize>,
    threads: &[ReviewThread],
    heights: &std::collections::HashMap<crate::core::review::ReviewThreadId, u16>,
    is_expanded: impl Fn(&ReviewThread) -> bool,
) {
    let mut grouped: BTreeMap<u32, Vec<ReviewThread>> = BTreeMap::new();
    for thread in threads {
        let Some(anchor) = thread_anchor_line_index(render_doc, file, line_range.clone(), thread)
        else {
            continue;
        };
        grouped.entry(anchor).or_default().push(thread.clone());
    }

    for (anchor, mut threads) in grouped {
        // Push one block per thread; multiple blocks at the same anchor stack in
        // push order, so sort here (unresolved first) to control display order.
        threads.sort_by(|a, b| {
            thread_sort_rank(a)
                .cmp(&thread_sort_rank(b))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        for thread in threads {
            let expanded = is_expanded(&thread);
            // Reserve the height measured by the shell's precompute pass; fall back to a
            // generous over-estimate only if this thread was not measured this frame.
            let measured = heights
                .get(&thread.id)
                .copied()
                .unwrap_or_else(|| estimate_thread_height_px(&thread, expanded));
            blocks.push(
                BlockPlacement::Below(anchor),
                Box::new(ReviewThreadBlock::new(thread, expanded, measured)),
            );
        }
    }
}

pub fn populate_review_comment_blocks(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    comments: &[PullRequestReviewComment],
) {
    populate_review_comment_blocks_in_range(
        blocks,
        render_doc,
        0..render_doc.lines.len(),
        comments,
    );
}

pub fn populate_review_comment_blocks_in_range(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    line_range: Range<usize>,
    comments: &[PullRequestReviewComment],
) {
    let mut grouped: BTreeMap<u32, Vec<PullRequestReviewComment>> = BTreeMap::new();
    for comment in comments {
        let Some(anchor) = anchor_line_index(render_doc, line_range.clone(), comment) else {
            continue;
        };
        grouped.entry(anchor).or_default().push(comment.clone());
    }

    for (anchor, mut comments) in grouped {
        comments.sort_by_key(|comment| (comment.in_reply_to_id.unwrap_or(comment.id), comment.id));
        blocks.push(
            BlockPlacement::Below(anchor),
            Box::new(ReviewCommentBlock::new(comments)),
        );
    }
}

pub fn render_doc_file_line_range(render_doc: &RenderDoc, path: &str) -> Option<Range<usize>> {
    let mut start = None;
    for (idx, line) in render_doc.lines.iter().enumerate() {
        if line.row_kind() != RenderRowKind::FileHeader {
            continue;
        }
        let Some(meta) = render_doc.file_meta(line) else {
            continue;
        };
        if let Some(start) = start {
            return Some(start..idx);
        }
        if meta.path == path || meta.old_path.as_deref() == Some(path) {
            start = Some(idx);
        }
    }
    start.map(|start| start..render_doc.lines.len())
}

fn review_panel_rect(ctx: &BlockPaintCtx) -> Rect {
    let m = &ctx.theme.metrics;
    let scale = m.ui_scale();
    let text_rect = if ctx.layout.split_mode {
        let left = ctx.layout.left_text_rect.x;
        let right = ctx.layout.right_text_rect.x + ctx.layout.right_text_rect.width;
        Rect {
            x: left,
            y: ctx.row_rect.y,
            width: right - left,
            height: ctx.row_rect.height,
        }
    } else {
        Rect {
            x: ctx.layout.unified_text_rect.x,
            y: ctx.row_rect.y,
            width: ctx.layout.unified_text_rect.width,
            height: ctx.row_rect.height,
        }
    };
    Rect {
        x: text_rect.x,
        y: ctx.row_rect.y + m.spacing_xs,
        width: text_rect.width.min(PANEL_MAX_WIDTH * scale),
        height: (ctx.row_rect.height - m.spacing_xs * 2.0).max(0.0),
    }
}

fn paint_panel_chrome(ctx: &mut BlockPaintCtx, panel: Rect) {
    let scale = ctx.theme.metrics.ui_scale();
    let radius = ctx.theme.metrics.control_radius;
    for layer in Shadow::DROPDOWN {
        ctx.scene.shadow(ShadowPrimitive {
            rect: panel,
            blur_radius: layer.blur * scale,
            corner_radius: radius,
            offset: [0.0, layer.offset_y * scale],
            color: Color::rgba(0, 0, 0, layer.alpha),
        });
    }
    ctx.scene.rounded_rect(RoundedRectPrimitive::uniform(
        panel,
        radius,
        ctx.theme.colors.elevated_surface,
    ));
    ctx.scene.border(BorderPrimitive::uniform(
        panel,
        1.0,
        radius,
        ctx.theme.colors.border,
    ));
}

fn review_line_height(ctx: &BlockPaintCtx) -> f32 {
    ctx.layout.line_height.max(1.0).min(ctx.row_rect.height)
}

pub(crate) fn clean_comment_body(body: &str) -> Vec<String> {
    strip_html_comments(body)
        .lines()
        .map(clean_markdown_line)
        .filter(|line| !line.is_empty())
        .take(MAX_BODY_LINES_PER_COMMENT)
        .collect()
}

fn strip_html_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn clean_markdown_line(line: &str) -> String {
    let mut s = line.trim();
    if s.starts_with("```") || s.starts_with("~~~") {
        return String::new();
    }
    loop {
        let next = s
            .strip_prefix("> ")
            .or_else(|| s.strip_prefix('>'))
            .or_else(|| s.strip_prefix("- "))
            .or_else(|| s.strip_prefix("* "))
            .or_else(|| s.strip_prefix("+ "))
            .or_else(|| s.strip_prefix('#'))
            .map(str::trim_start);
        match next {
            Some(n) if n != s => s = n,
            _ => break,
        }
    }
    let s = strip_ordered_list_marker(s);
    let s = strip_markdown_links(s);
    let s = s.replace("**", "").replace("__", "").replace('`', "");
    collapse_whitespace(&s)
}

fn strip_ordered_list_marker(s: &str) -> &str {
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return s;
    }
    let rest = &s[digits..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .map(str::trim_start)
        .unwrap_or(s)
}

fn strip_markdown_links(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let bracket = match chars[i] {
            '!' if chars.get(i + 1) == Some(&'[') => Some(i + 1),
            '[' => Some(i),
            _ => None,
        };
        if let Some(open) = bracket
            && let Some(close) = find_char(&chars, open + 1, ']')
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = find_char(&chars, close + 2, ')')
        {
            out.extend(chars[open + 1..close].iter());
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_char(chars: &[char], start: usize, target: char) -> Option<usize> {
    chars[start..]
        .iter()
        .position(|&c| c == target)
        .map(|offset| start + offset)
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

fn visual_line_char_limit(width: f32, font_size: f32) -> usize {
    let estimated_char_width = (font_size * 0.56).max(5.0);
    ((width / estimated_char_width).floor() as usize).clamp(16, 96)
}

fn truncate_display_line(line: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in line.chars().enumerate() {
        if idx >= max_chars.saturating_sub(3) {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

pub(crate) fn thread_line_label(thread: &ReviewThread) -> String {
    let Some(anchor) = thread.anchor.as_ref() else {
        return "Comment on file".to_owned();
    };
    let Some(line) = anchor.active_line_range().map(|range| range.github_line()) else {
        return "Comment on file".to_owned();
    };
    let side = match anchor.side {
        Some(ReviewSide::New) => "R",
        Some(ReviewSide::Old) => "L",
        None => "",
    };
    format!("Comment on line {side}{line}")
}

fn reaction_emoji(content: &str) -> &'static str {
    match content {
        "THUMBS_UP" => "👍",
        "THUMBS_DOWN" => "👎",
        "LAUGH" => "😄",
        "HOORAY" => "🎉",
        "CONFUSED" => "😕",
        "HEART" => "❤️",
        "ROCKET" => "🚀",
        "EYES" => "👀",
        _ => "•",
    }
}

pub(crate) fn review_resolution_action(thread: &ReviewThread) -> Option<(bool, &'static str)> {
    match thread.status.resolution {
        ReviewResolution::Resolved if thread.permissions.can_unresolve => {
            Some((false, "Unresolve"))
        }
        ReviewResolution::Unresolved | ReviewResolution::Unknown
            if thread.permissions.can_resolve =>
        {
            Some((true, "Resolve"))
        }
        _ => None,
    }
}

pub(crate) fn review_context_menu_entries(thread: &ReviewThread) -> Vec<ContextMenuEntry> {
    let first = thread.comments.first();
    let link = first
        .and_then(|comment| comment.html_url.clone())
        .unwrap_or_default();
    let link_missing = link.is_empty();
    let markdown = review_comment_markdown(thread);
    let quote = first.map(markdown_quote).unwrap_or_else(|| "> ".to_owned());
    let can_edit = first.is_some_and(|comment| comment.viewer_can_update);
    let can_delete = first.is_some_and(|comment| comment.viewer_can_delete);

    let mut entries = vec![
        ContextMenuEntry::item(
            "Copy link",
            if link.is_empty() {
                Action::Noop
            } else {
                AppAction::CopyText(link).into()
            },
        )
        .icon(lucide::COPY)
        .disabled_if(link_missing),
        ContextMenuEntry::item(
            "Copy Markdown",
            AppAction::CopyText(markdown.clone()).into(),
        )
        .icon(lucide::COPY),
        ContextMenuEntry::item("Quote reply", AppAction::CopyText(quote).into())
            .icon(lucide::CORNER_UP_LEFT),
        ContextMenuEntry::separator(),
        ContextMenuEntry::item("Reference in a new issue", Action::Noop)
            .icon(lucide::CIRCLE_DOT)
            .disabled(),
    ];

    // Author-only actions, surfaced only when the viewer has the permission so the menu
    // mirrors what they could do on GitHub. Disabled until the edit/hide/delete effects
    // are wired (no GitHubAction yet) — shown-but-disabled rather than dead-active.
    if can_edit || can_delete {
        entries.push(ContextMenuEntry::separator());
        if can_edit {
            entries.push(
                ContextMenuEntry::item("Edit", Action::Noop)
                    .icon(lucide::PENCIL)
                    .disabled(),
            );
            entries.push(
                ContextMenuEntry::item("Hide", Action::Noop)
                    .icon(lucide::EYE_OFF)
                    .disabled(),
            );
        }
        if can_delete {
            entries.push(
                ContextMenuEntry::item("Delete", Action::Noop)
                    .icon(lucide::X)
                    .destructive()
                    .disabled(),
            );
        }
    }
    entries
}

fn review_comment_markdown(thread: &ReviewThread) -> String {
    let line_label = thread_line_label(thread);
    let Some(comment) = thread.comments.first() else {
        return line_label;
    };
    let quote = markdown_quote(comment);
    match comment.html_url.as_deref().filter(|url| !url.is_empty()) {
        Some(url) => format!("[{line_label}]({url})\n\n{quote}"),
        None => format!("{line_label}\n\n{quote}"),
    }
}

fn markdown_quote(comment: &ReviewComment) -> String {
    let body = strip_html_comments(&comment.body);
    let mut out = String::new();
    for line in body.lines().take(24) {
        out.push_str("> ");
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("> \n");
    }
    out
}

pub(crate) fn relative_time_label(iso: Option<&str>) -> Option<String> {
    let then = parse_rfc3339_utc_seconds(iso?)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let elapsed = now.saturating_sub(then);
    Some(if elapsed < 60 {
        "just now".to_owned()
    } else if elapsed < 3_600 {
        plural_time(elapsed / 60, "minute")
    } else if elapsed < 86_400 {
        plural_time(elapsed / 3_600, "hour")
    } else if elapsed < 2_592_000 {
        plural_time(elapsed / 86_400, "day")
    } else if elapsed < 31_536_000 {
        plural_time(elapsed / 2_592_000, "month")
    } else {
        plural_time(elapsed / 31_536_000, "year")
    })
}

fn plural_time(value: u64, unit: &str) -> String {
    if value == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{value} {unit}s ago")
    }
}

fn parse_rfc3339_utc_seconds(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = parse_digits(&bytes[0..4])? as i32;
    let month = parse_digits(&bytes[5..7])? as u32;
    let day = parse_digits(&bytes[8..10])? as u32;
    let hour = parse_digits(&bytes[11..13])? as u64;
    let minute = parse_digits(&bytes[14..16])? as u64;
    let second = parse_digits(&bytes[17..19])? as u64;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    if days < 0 {
        return None;
    }
    Some(days as u64 * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn parse_digits(bytes: &[u8]) -> Option<u32> {
    let mut value = 0_u32;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(byte - b'0');
    }
    Some(value)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let year = year - i32::from(month <= 2);
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era * 146_097 + doe - 719_468))
}

/// Splits the thread status into the state word (e.g. "Resolved") and a muted
/// metadata run (e.g. "1 reply · outdated"), so the header can render state and
/// metadata with distinct emphasis instead of one flat text run.
pub(crate) fn thread_status_parts(thread: &ReviewThread) -> (&'static str, String) {
    let word = match thread.status.resolution {
        ReviewResolution::Resolved => "Resolved",
        ReviewResolution::Unresolved => "Unresolved",
        ReviewResolution::Unknown => "Open",
    };
    let reply_count = thread.comments.len().saturating_sub(1);
    let mut meta = if reply_count == 1 {
        "1 reply".to_owned()
    } else if reply_count > 1 {
        format!("{reply_count} replies")
    } else {
        String::new()
    };
    if thread.status.outdated {
        if !meta.is_empty() {
            meta.push_str(" · ");
        }
        meta.push_str("outdated");
    }
    (word, meta)
}

pub(crate) fn resolution_color(
    resolution: ReviewResolution,
    colors: &crate::ui::theme::ThemeColors,
) -> crate::ui::theme::Color {
    match resolution {
        ReviewResolution::Resolved => colors.text_muted,
        ReviewResolution::Unresolved | ReviewResolution::Unknown => colors.accent,
    }
}

fn thread_sort_rank(thread: &ReviewThread) -> u8 {
    match thread.status.resolution {
        ReviewResolution::Unresolved | ReviewResolution::Unknown => 0,
        ReviewResolution::Resolved => 1,
    }
}

fn thread_anchor_line_index(
    render_doc: &RenderDoc,
    file: &carbon::FileDiff,
    line_range: Range<usize>,
    thread: &ReviewThread,
) -> Option<u32> {
    let anchor = thread.anchor.as_ref()?;
    anchor.to_carbon_anchor(file)?;
    let line = anchor.active_line_range().map(|range| range.github_line());
    if let Some(line) = line {
        let side = anchor.side;
        if let Some(index) = render_doc
            .lines
            .get(clamp_line_range(render_doc, line_range.clone()))
            .unwrap_or_default()
            .iter()
            .enumerate()
            .find_map(|(idx, render_line)| match side {
                Some(ReviewSide::New) if render_line.new_line_no == line => {
                    Some(line_range.start.saturating_add(idx) as u32)
                }
                Some(ReviewSide::Old) if render_line.old_line_no == line => {
                    Some(line_range.start.saturating_add(idx) as u32)
                }
                None if render_line.new_line_no == line || render_line.old_line_no == line => {
                    Some(line_range.start.saturating_add(idx) as u32)
                }
                _ => None,
            })
        {
            return Some(index);
        }
    }
    first_content_line_index(render_doc, line_range)
}

fn anchor_line_index(
    render_doc: &RenderDoc,
    line_range: Range<usize>,
    comment: &PullRequestReviewComment,
) -> Option<u32> {
    let side = comment.side?;
    let line = comment.line.or(comment.original_line)?;
    render_doc
        .lines
        .get(clamp_line_range(render_doc, line_range.clone()))
        .unwrap_or_default()
        .iter()
        .enumerate()
        .find_map(|(idx, render_line)| match side {
            GitHubReviewSide::Right if render_line.new_line_no == line => {
                Some(line_range.start.saturating_add(idx) as u32)
            }
            GitHubReviewSide::Left if render_line.old_line_no == line => {
                Some(line_range.start.saturating_add(idx) as u32)
            }
            _ => None,
        })
        .or_else(|| first_content_line_index(render_doc, line_range))
}

fn first_content_line_index(render_doc: &RenderDoc, line_range: Range<usize>) -> Option<u32> {
    render_doc
        .lines
        .get(clamp_line_range(render_doc, line_range.clone()))
        .unwrap_or_default()
        .iter()
        .enumerate()
        .find_map(|(idx, render_line)| {
            (render_line.old_line_no != INVALID_U32 || render_line.new_line_no != INVALID_U32)
                .then_some(line_range.start.saturating_add(idx) as u32)
        })
}

fn clamp_line_range(render_doc: &RenderDoc, line_range: Range<usize>) -> Range<usize> {
    let start = line_range.start.min(render_doc.lines.len());
    let end = line_range.end.min(render_doc.lines.len()).max(start);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_comment_metadata() {
        let body = "<!-- tool-marker {\"id\": \"abc-123\"} -->\nFirst visible sentence";
        assert_eq!(clean_comment_body(body), vec!["First visible sentence"]);
    }

    #[test]
    fn strips_multiline_html_comment() {
        let body = "<!-- meta\nspanning\nlines -->visible text";
        assert_eq!(clean_comment_body(body), vec!["visible text"]);
    }

    #[test]
    fn strips_markdown_emphasis_and_code() {
        assert_eq!(
            clean_markdown_line("**bold word** and `inline code` together"),
            "bold word and inline code together"
        );
    }

    #[test]
    fn strips_heading_quote_and_list_markers() {
        assert_eq!(clean_markdown_line("## Heading text"), "Heading text");
        assert_eq!(clean_markdown_line("> quoted text"), "quoted text");
        assert_eq!(clean_markdown_line("- bullet item"), "bullet item");
        assert_eq!(clean_markdown_line("1. first item"), "first item");
    }

    #[test]
    fn strips_links_and_images_keeping_text() {
        assert_eq!(
            clean_markdown_line("see [the label](https://example.com/x) here"),
            "see the label here"
        );
        assert_eq!(
            clean_markdown_line("![alt text](https://example.com/img.png)"),
            "alt text"
        );
    }

    #[test]
    fn drops_code_fence_lines() {
        assert!(clean_markdown_line("```rust").is_empty());
        assert!(clean_markdown_line("~~~").is_empty());
    }

    #[test]
    fn caps_body_lines() {
        let body = "one\ntwo\nthree\nfour\nfive";
        assert_eq!(clean_comment_body(body).len(), MAX_BODY_LINES_PER_COMMENT);
    }

    #[test]
    fn metadata_prefixed_comment_is_readable() {
        let body = "<!-- tool-marker {\"id\": \"x1\"} -->\n\
                    **Heading** with `code` and trailing words";
        let cleaned = clean_comment_body(body);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0], "Heading with code and trailing words");
    }
}
