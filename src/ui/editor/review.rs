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
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

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
            // Segment the body into ordered prose/code blocks. Prose runs reuse the
            // inline markdown parser (code/bold/italic/links → styled spans); fenced
            // code becomes a syntax-highlighted code_block. The concatenation of a
            // prose run's span texts is the plain body that selection state keys on
            // (byte offsets, re-wrap stable) and that copy yields.
            let source_key = card_source_key(&thread.id, idx);
            let blocks = segment_comment_blocks(&comment.body);
            let mut block_elements: Vec<AnyElement> = Vec::new();
            for (block_idx, block) in blocks.into_iter().enumerate() {
                // Distinct selection key per block so sibling prose runs don't
                // mirror each other's highlight; block 0 keeps the comment's key.
                let block_source = if block_idx == 0 {
                    source_key
                } else {
                    source_key ^ (block_idx as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                };
                let block_selection = selection
                    .filter(|sel| sel.source_key == block_source)
                    .map(|sel| sel.normalized());
                match block {
                    CommentBlock::Prose(inline) => {
                        let spans: Vec<StyledSpan> = inline
                            .into_iter()
                            .map(|span| inline_span_to_styled(span, tc))
                            .collect();
                        if spans.iter().all(|s| s.text.is_empty()) {
                            continue;
                        }
                        block_elements.push(
                            selectable_rich_text(spans)
                                .width(text_w)
                                .size(small)
                                .color(body_color)
                                .max_lines(8)
                                .source(block_source)
                                .selection(block_selection)
                                .into_any(),
                        );
                    }
                    CommentBlock::Heading { level, inline } => {
                        let spans: Vec<StyledSpan> = inline
                            .into_iter()
                            .map(|span| {
                                inline_span_to_styled_with_weight(span, tc, FontWeight::Semibold)
                            })
                            .collect();
                        if spans.iter().all(|s| s.text.is_empty()) {
                            continue;
                        }
                        block_elements.push(
                            selectable_rich_text(spans)
                                .width(text_w)
                                .size(heading_font_size(level, small))
                                .color(if resolved {
                                    tc.text_muted
                                } else {
                                    tc.text_strong
                                })
                                .max_lines(2)
                                .source(block_source)
                                .selection(block_selection)
                                .into_any(),
                        );
                    }
                    CommentBlock::Code { lang, source } => {
                        if source.is_empty() {
                            continue;
                        }
                        let highlighted =
                            highlight_code_lines(lang.as_deref(), &source, tc, thread.path());
                        block_elements.push(
                            code_block(highlighted)
                                .width(text_w)
                                .size(small * 0.9)
                                .source(block_source)
                                .into_any(),
                        );
                    }
                }
            }
            let body_element: AnyElement = if block_elements.is_empty() {
                text("(empty comment)")
                    .size(small)
                    .color(tc.text_muted)
                    .into_any()
            } else {
                view! { ui_scale,
                    <div class="flex-col" gap={Sp::XXS} min_w={0.0}>
                        {...block_elements}
                    </div>
                }
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
        .then(|| {
            let mut buttons: Vec<AnyElement> = Vec::new();
            if thread.permissions.can_reply {
                buttons.push(view! { ui_scale,
                    <Button action={GitHubAction::ReplyToReviewThread(thread.id.clone()).into()}
                            style={ButtonStyle::Subtle}
                            size={ButtonSize::Compact}>
                        <Label>{"Reply"}</Label>
                    </Button>
                });
            }
            if let Some((target, label)) = review_resolution_action(thread) {
                buttons.push(view! { ui_scale,
                    <Button action={GitHubAction::SetReviewThreadResolved {
                                id: thread.id.clone(),
                                resolved: target,
                            }.into()}
                            style={ButtonStyle::Subtle}
                            size={ButtonSize::Compact}>
                        <Label>{label}</Label>
                    </Button>
                });
            }
            buttons
        })
        .filter(|buttons| !buttons.is_empty())
        .map(|buttons| {
            view! { ui_scale,
                // The thread actions, sitting below the conversation with a little
                // breathing room. No divider: a horizontal line inside the bordered card
                // frames the sparse footer row into what looks like an empty text box.
                <div class="flex-row items-center" gap={Sp::XS} pt={Sp::XXS}>
                    {...buttons}
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
        thread, expanded, theme, ui_scale, card_width, &avatars, None,
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

/// Block that reserves space in the diff flow for the open comment composer, so the
/// composer pushes the lines below down (like a thread card) instead of overlaying
/// them. The shell renders the composer `view!` at this block's reserved rect.
#[derive(Debug)]
struct ComposerBlock {
    height: u16,
}

impl BlockDecoration for ComposerBlock {
    fn height(&self, _metrics: &DisplayLayoutMetrics) -> u16 {
        self.height
    }
    fn paint(&self, _ctx: &mut BlockPaintCtx) {}
    fn is_composer(&self) -> bool {
        true
    }
    fn accessibility_label(&self) -> Option<String> {
        Some("Review comment composer".to_owned())
    }
}

/// Pushes the composer block below the diff line identified by `side`/`line` (a
/// GitHub line number on that side). No-op if the line isn't visible in the doc.
pub fn push_review_composer_block(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    side: ReviewSide,
    line: u32,
    height: u16,
) {
    push_review_composer_block_in_range(
        blocks,
        render_doc,
        0..render_doc.lines.len(),
        side,
        line,
        height,
    );
}

/// Like [`push_review_composer_block`] but only scans `line_range` — needed for the
/// continuous (all-files) doc, where the same GitHub line number recurs per file.
pub fn push_review_composer_block_in_range(
    blocks: &mut BlockRegistry,
    render_doc: &RenderDoc,
    line_range: Range<usize>,
    side: ReviewSide,
    line: u32,
    height: u16,
) {
    let range = clamp_line_range(render_doc, line_range);
    let anchor = render_doc.lines[range.clone()]
        .iter()
        .enumerate()
        .find_map(|(idx, l)| {
            let hit = match side {
                ReviewSide::New => l.new_line_no == line,
                ReviewSide::Old => l.old_line_no == line,
            };
            (hit && l.hunk_index >= 0).then_some((range.start + idx) as u32)
        });
    if let Some(anchor) = anchor {
        blocks.push(
            BlockPlacement::Below(anchor),
            Box::new(ComposerBlock { height }),
        );
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

/// Inline emphasis carried by a span of comment text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InlineStyle {
    Normal,
    Bold,
    Italic,
    Code,
    Link,
}

/// A run of comment text with a single inline style. The concatenation of all
/// span texts is the plain body used for selection/copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InlineSpan {
    pub text: String,
    pub style: InlineStyle,
}

/// Parses a comment body into styled inline spans (no fences): strips HTML comments
/// and block markers but preserves inline emphasis. Superseded in production by
/// [`segment_comment_blocks`] (which handles fenced code too); kept under `cfg(test)`
/// as the focused subject of the inline-parser edge-case tests below.
#[cfg(test)]
fn parse_comment_body_rich(body: &str) -> Vec<InlineSpan> {
    const MAX_CHARS: usize = 600;
    let stripped = strip_html_comments(body);
    let mut joined = String::new();
    for line in stripped.lines() {
        let cleaned = block_strip_keep_inline(line);
        if cleaned.is_empty() {
            continue;
        }
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(&cleaned);
        if joined.chars().count() >= MAX_CHARS {
            break;
        }
    }
    parse_inline(&joined)
}

/// Maps a parsed inline span to its display style (code/bold/italic/link).
fn inline_span_to_styled(span: InlineSpan, tc: &crate::ui::theme::ThemeColors) -> StyledSpan {
    inline_span_to_styled_with_weight(span, tc, FontWeight::Normal)
}

fn inline_span_to_styled_with_weight(
    span: InlineSpan,
    tc: &crate::ui::theme::ThemeColors,
    base_weight: FontWeight,
) -> StyledSpan {
    let mut styled = StyledSpan::plain(span.text);
    styled.font_weight = base_weight;
    match span.style {
        InlineStyle::Normal => {}
        InlineStyle::Bold => styled.font_weight = FontWeight::Semibold,
        InlineStyle::Italic => styled.italic = true,
        InlineStyle::Code => {
            styled.font_kind = FontKind::Mono;
            styled.color = Some(tc.text);
            styled.pill = Some(tc.element_background);
        }
        InlineStyle::Link => styled.color = Some(tc.accent),
    }
    styled
}

/// An ordered block of comment content: inline prose, heading, or fenced code.
#[derive(Debug, Clone)]
pub(crate) enum CommentBlock {
    Prose(Vec<InlineSpan>),
    Heading {
        level: u8,
        inline: Vec<InlineSpan>,
    },
    Code {
        lang: Option<String>,
        source: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownInlineKind {
    Prose,
    Heading(u8),
}

/// Segments a body into ordered markdown blocks. A real markdown parser owns block
/// structure and inline emphasis; fenced code still feeds Diffy's syntax highlighter.
pub(crate) fn segment_comment_blocks(body: &str) -> Vec<CommentBlock> {
    let stripped = strip_html_comments(body);
    let normalized = normalize_inline_fences(&stripped);
    let parser = Parser::new_ext(
        &normalized,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    );
    let mut blocks: Vec<CommentBlock> = Vec::new();
    let mut inline: Option<(MarkdownInlineKind, Vec<InlineSpan>)> = None;
    let mut code: Option<(Option<String>, String)> = None;
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut pending_item_prefix: Option<String> = None;
    let mut block_quote_depth = 0usize;
    let mut strong_depth = 0usize;
    let mut emphasis_depth = 0usize;
    let mut link_depth = 0usize;

    for event in parser {
        match event {
            Event::Start(Tag::Paragraph) => {
                start_inline_block(
                    &mut inline,
                    MarkdownInlineKind::Prose,
                    block_quote_depth,
                    pending_item_prefix.take(),
                );
            }
            Event::End(TagEnd::Paragraph) => flush_inline_block(&mut inline, &mut blocks),
            Event::Start(Tag::Heading { level, .. }) => {
                start_inline_block(
                    &mut inline,
                    MarkdownInlineKind::Heading(heading_level_u8(level)),
                    block_quote_depth,
                    None,
                );
            }
            Event::End(TagEnd::Heading(_)) => flush_inline_block(&mut inline, &mut blocks),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_inline_block(&mut inline, &mut blocks);
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let tag = info
                            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                            .next()
                            .unwrap_or("");
                        (!tag.is_empty()).then(|| tag.to_owned())
                    }
                    CodeBlockKind::Indented => None,
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, mut source)) = code.take() {
                    if source.ends_with('\n') {
                        source.pop();
                        if source.ends_with('\r') {
                            source.pop();
                        }
                    }
                    blocks.push(CommentBlock::Code { lang, source });
                }
            }
            Event::Start(Tag::List(start)) => list_stack.push(start),
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
            }
            Event::Start(Tag::Item) => {
                pending_item_prefix = Some(next_list_item_prefix(&mut list_stack));
            }
            Event::End(TagEnd::Item) => flush_inline_block(&mut inline, &mut blocks),
            Event::Start(Tag::BlockQuote(_)) => {
                block_quote_depth = block_quote_depth.saturating_add(1)
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                block_quote_depth = block_quote_depth.saturating_sub(1)
            }
            Event::Start(Tag::Strong) => strong_depth = strong_depth.saturating_add(1),
            Event::End(TagEnd::Strong) => strong_depth = strong_depth.saturating_sub(1),
            Event::Start(Tag::Emphasis) => emphasis_depth = emphasis_depth.saturating_add(1),
            Event::End(TagEnd::Emphasis) => emphasis_depth = emphasis_depth.saturating_sub(1),
            Event::Start(Tag::Link { .. }) => link_depth = link_depth.saturating_add(1),
            Event::End(TagEnd::Link) => link_depth = link_depth.saturating_sub(1),
            Event::Text(text) => {
                if let Some((_, source)) = code.as_mut() {
                    push_code_text(source, text.as_ref());
                } else {
                    if inline.is_none() {
                        start_inline_block(
                            &mut inline,
                            MarkdownInlineKind::Prose,
                            block_quote_depth,
                            pending_item_prefix.take(),
                        );
                    }
                    let style = current_inline_style(strong_depth, emphasis_depth, link_depth);
                    push_inline_text(&mut inline, text.as_ref(), style);
                }
            }
            Event::Code(text) => {
                if inline.is_none() {
                    start_inline_block(
                        &mut inline,
                        MarkdownInlineKind::Prose,
                        block_quote_depth,
                        pending_item_prefix.take(),
                    );
                }
                push_inline_text(&mut inline, text.as_ref(), InlineStyle::Code);
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, source)) = code.as_mut() {
                    push_code_text(source, "\n");
                } else {
                    push_inline_text(
                        &mut inline,
                        " ",
                        current_inline_style(strong_depth, emphasis_depth, link_depth),
                    );
                }
            }
            Event::TaskListMarker(checked) => {
                if inline.is_none() {
                    start_inline_block(
                        &mut inline,
                        MarkdownInlineKind::Prose,
                        block_quote_depth,
                        pending_item_prefix.take(),
                    );
                }
                let marker = if checked { "[x] " } else { "[ ] " };
                push_inline_text(&mut inline, marker, InlineStyle::Normal);
            }
            Event::Rule => flush_inline_block(&mut inline, &mut blocks),
            Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
            Event::Start(_) | Event::End(_) => {}
        }
    }

    if let Some((lang, source)) = code.take() {
        blocks.push(CommentBlock::Code { lang, source });
    }
    flush_inline_block(&mut inline, &mut blocks);
    blocks
}

fn normalize_inline_fences(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    for (idx, line) in body.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let Some(fence_idx) = inline_block_fence_index(line) else {
            out.push_str(line);
            continue;
        };
        let before = line[..fence_idx].trim_end();
        if !before.is_empty() {
            out.push_str(before);
            out.push('\n');
        }
        out.push_str(line[fence_idx..].trim_start());
    }
    if body.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn inline_block_fence_index(line: &str) -> Option<usize> {
    let backtick = line.find("```");
    let tilde = line.find("~~~");
    let idx = match (backtick, tilde) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    if line[..idx].trim().is_empty() {
        return None;
    }
    let rest = &line[idx..];
    if !rest.starts_with("```") && !rest.starts_with("~~~") {
        return None;
    }
    let marker = rest.as_bytes()[0];
    let fence_len = rest
        .as_bytes()
        .iter()
        .take_while(|&&byte| byte == marker)
        .count();
    if fence_len < 3 {
        return None;
    }
    let info = rest[fence_len..].trim_start();
    if info.is_empty() {
        return Some(idx);
    }
    let tag = info
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .next()
        .unwrap_or("");
    code_language_from_tag(tag).is_some().then_some(idx)
}

fn heading_level_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn current_inline_style(
    strong_depth: usize,
    emphasis_depth: usize,
    link_depth: usize,
) -> InlineStyle {
    if link_depth > 0 {
        InlineStyle::Link
    } else if strong_depth > 0 {
        InlineStyle::Bold
    } else if emphasis_depth > 0 {
        InlineStyle::Italic
    } else {
        InlineStyle::Normal
    }
}

fn start_inline_block(
    inline: &mut Option<(MarkdownInlineKind, Vec<InlineSpan>)>,
    kind: MarkdownInlineKind,
    block_quote_depth: usize,
    prefix: Option<String>,
) {
    *inline = Some((kind, Vec::new()));
    if block_quote_depth > 0 {
        push_inline_text(inline, "> ", InlineStyle::Normal);
    }
    if let Some(prefix) = prefix {
        push_inline_text(inline, &prefix, InlineStyle::Normal);
    }
}

fn flush_inline_block(
    inline: &mut Option<(MarkdownInlineKind, Vec<InlineSpan>)>,
    blocks: &mut Vec<CommentBlock>,
) {
    let Some((kind, spans)) = inline.take() else {
        return;
    };
    if spans.iter().all(|span| span.text.trim().is_empty()) {
        return;
    }
    match kind {
        MarkdownInlineKind::Prose => blocks.push(CommentBlock::Prose(spans)),
        MarkdownInlineKind::Heading(level) => blocks.push(CommentBlock::Heading {
            level,
            inline: spans,
        }),
    }
}

fn push_inline_text(
    inline: &mut Option<(MarkdownInlineKind, Vec<InlineSpan>)>,
    text: &str,
    style: InlineStyle,
) {
    if text.is_empty() {
        return;
    }
    let Some((_, spans)) = inline.as_mut() else {
        return;
    };
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.text.push_str(text);
        return;
    }
    spans.push(InlineSpan {
        text: text.to_owned(),
        style,
    });
}

fn push_code_text(source: &mut String, text: &str) {
    source.push_str(text);
}

fn next_list_item_prefix(list_stack: &mut [Option<u64>]) -> String {
    match list_stack.last_mut() {
        Some(Some(next)) => {
            let current = *next;
            *next = next.saturating_add(1);
            format!("{current}. ")
        }
        _ => "• ".to_owned(),
    }
}

fn heading_font_size(level: u8, small: f32) -> f32 {
    match level {
        1 => small * 1.18,
        2 => small * 1.1,
        3 => small * 1.04,
        _ => small,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLanguageHint {
    Rust,
    Python,
    TypeScript,
    TypeScriptTsx,
    JavaScript,
    Go,
    C,
    Cpp,
    Json,
    Toml,
    Shell,
    Nix,
    Zig,
}

fn code_language_from_tag(tag: &str) -> Option<CodeLanguageHint> {
    let tag = tag
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match tag.as_str() {
        "rust" | "rs" => Some(CodeLanguageHint::Rust),
        "python" | "py" => Some(CodeLanguageHint::Python),
        "typescript" | "ts" => Some(CodeLanguageHint::TypeScript),
        "tsx" => Some(CodeLanguageHint::TypeScriptTsx),
        "javascript" | "js" | "mjs" => Some(CodeLanguageHint::JavaScript),
        "jsx" => Some(CodeLanguageHint::TypeScriptTsx),
        "go" | "golang" => Some(CodeLanguageHint::Go),
        "c" | "h" => Some(CodeLanguageHint::C),
        "cpp" | "c++" | "cc" | "cxx" | "hpp" => Some(CodeLanguageHint::Cpp),
        "json" => Some(CodeLanguageHint::Json),
        "toml" => Some(CodeLanguageHint::Toml),
        "sh" | "bash" | "shell" | "zsh" => Some(CodeLanguageHint::Shell),
        "nix" => Some(CodeLanguageHint::Nix),
        "zig" => Some(CodeLanguageHint::Zig),
        _ => return None,
    }
}

fn code_language_from_path(path: &str) -> Option<CodeLanguageHint> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(std::ffi::OsStr::to_str)?
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(CodeLanguageHint::Rust),
        "py" | "pyi" => Some(CodeLanguageHint::Python),
        "ts" => Some(CodeLanguageHint::TypeScript),
        "tsx" => Some(CodeLanguageHint::TypeScriptTsx),
        "js" | "mjs" => Some(CodeLanguageHint::JavaScript),
        "jsx" => Some(CodeLanguageHint::TypeScriptTsx),
        "go" => Some(CodeLanguageHint::Go),
        "c" | "h" => Some(CodeLanguageHint::C),
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => Some(CodeLanguageHint::Cpp),
        "json" => Some(CodeLanguageHint::Json),
        "toml" => Some(CodeLanguageHint::Toml),
        "bash" | "sh" | "zsh" => Some(CodeLanguageHint::Shell),
        "nix" => Some(CodeLanguageHint::Nix),
        "zig" => Some(CodeLanguageHint::Zig),
        _ => None,
    }
}

fn infer_code_language(source: &str) -> Option<CodeLanguageHint> {
    let sample = source.trim_start();
    if sample.starts_with("import ")
        || sample.starts_with("export ")
        || sample.contains(" from \"")
        || sample.contains(" from '")
        || sample.contains("=>")
    {
        return Some(CodeLanguageHint::TypeScript);
    }
    if sample.starts_with("fn ") || sample.contains("let mut ") || sample.contains("use crate::") {
        return Some(CodeLanguageHint::Rust);
    }
    if sample.starts_with("def ") || sample.contains(":\n    ") {
        return Some(CodeLanguageHint::Python);
    }
    None
}

fn code_language_hint(
    lang: Option<&str>,
    source: &str,
    fallback_path: Option<&str>,
) -> Option<CodeLanguageHint> {
    lang.and_then(code_language_from_tag)
        .or_else(|| fallback_path.and_then(code_language_from_path))
        .or_else(|| infer_code_language(source))
}

fn highlighter_candidate_paths(hint: CodeLanguageHint) -> &'static [&'static str] {
    match hint {
        CodeLanguageHint::Rust => &["snippet.rs"],
        CodeLanguageHint::Python => &["snippet.py"],
        CodeLanguageHint::TypeScript => &["snippet.ts"],
        // The bundled javascript query currently relies on inherited ecma rules;
        // TypeScript is a useful superset for snippets and gives imports/strings
        // real colors in review cards.
        CodeLanguageHint::JavaScript => &["snippet.ts", "snippet.js"],
        CodeLanguageHint::TypeScriptTsx => &["snippet.tsx", "snippet.ts"],
        CodeLanguageHint::Go => &["snippet.go"],
        CodeLanguageHint::C => &["snippet.c"],
        CodeLanguageHint::Cpp => &["snippet.cpp"],
        CodeLanguageHint::Json => &["snippet.json"],
        CodeLanguageHint::Toml => &["snippet.toml"],
        CodeLanguageHint::Shell => &["snippet.sh"],
        CodeLanguageHint::Nix => &["snippet.nix"],
        CodeLanguageHint::Zig => &["snippet.zig"],
    }
}

/// Highlights fenced code synchronously into one `Vec<StyledSpan>` per source line.
/// Spans tile each line exactly. Unknown language or parse error → plain mono.
fn highlight_code_lines(
    lang: Option<&str>,
    source: &str,
    tc: &crate::ui::theme::ThemeColors,
    fallback_path: Option<&str>,
) -> Vec<Vec<StyledSpan>> {
    let highlighter = phosphor::Highlighter::new();
    let hint = code_language_hint(lang, source, fallback_path);
    let language = hint.and_then(|hint| {
        highlighter_candidate_paths(hint)
            .iter()
            .filter_map(|path| highlighter.guess_language(std::path::Path::new(path)))
            .find(|language| highlighter.is_parser_available(*language))
    });
    let mut kinds = vec![phosphor::HighlightKind::Normal; source.len()];
    let mut semantic_tokens = false;
    if let Some(language) = language
        && let Ok(spans) = highlighter.highlight_language(language, source)
    {
        for span in spans {
            let start = (span.offset as usize).min(source.len());
            let end = (start + span.length as usize).min(source.len());
            if !matches!(
                span.kind,
                phosphor::HighlightKind::Normal
                    | phosphor::HighlightKind::Operator
                    | phosphor::HighlightKind::Punctuation
            ) {
                semantic_tokens = true;
            }
            for k in &mut kinds[start..end] {
                *k = span.kind;
            }
        }
    }
    if !semantic_tokens && let Some(hint) = hint {
        apply_fallback_code_highlighting(hint, source, &mut kinds);
    }

    let mut lines: Vec<Vec<StyledSpan>> = Vec::new();
    let mut line: Vec<StyledSpan> = Vec::new();
    let mut run = String::new();
    let mut run_kind: Option<phosphor::HighlightKind> = None;

    let flush_run = |run: &mut String,
                     run_kind: &mut Option<phosphor::HighlightKind>,
                     line: &mut Vec<StyledSpan>| {
        if let Some(kind) = run_kind.take()
            && !run.is_empty()
        {
            let mut styled = StyledSpan::plain(std::mem::take(run));
            styled.font_kind = FontKind::Mono;
            styled.color = Some(syntax_kind_color(kind, tc));
            line.push(styled);
        }
    };

    for (idx, ch) in source.char_indices() {
        if ch == '\n' {
            flush_run(&mut run, &mut run_kind, &mut line);
            lines.push(std::mem::take(&mut line));
            continue;
        }
        let kind = kinds[idx];
        if run_kind != Some(kind) {
            flush_run(&mut run, &mut run_kind, &mut line);
            run_kind = Some(kind);
        }
        run.push(ch);
    }
    flush_run(&mut run, &mut run_kind, &mut line);
    lines.push(line);
    lines
}

fn apply_fallback_code_highlighting(
    hint: CodeLanguageHint,
    source: &str,
    kinds: &mut [phosphor::HighlightKind],
) {
    use phosphor::HighlightKind;

    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if supports_slash_comments(hint) && source[i..].starts_with("//") {
            let end = source[i..]
                .find('\n')
                .map(|offset| i + offset)
                .unwrap_or(source.len());
            fill_kind(kinds, i, end, HighlightKind::Comment);
            i = end;
            continue;
        }
        if supports_slash_comments(hint) && source[i..].starts_with("/*") {
            let end = source[i + 2..]
                .find("*/")
                .map(|offset| i + 2 + offset + 2)
                .unwrap_or(source.len());
            fill_kind(kinds, i, end, HighlightKind::Comment);
            i = end;
            continue;
        }
        if supports_hash_comments(hint) && ch == '#' {
            let end = source[i..]
                .find('\n')
                .map(|offset| i + offset)
                .unwrap_or(source.len());
            fill_kind(kinds, i, end, HighlightKind::Comment);
            i = end;
            continue;
        }
        if ch == '"' || ch == '\'' || (ch == '`' && supports_backtick_strings(hint)) {
            let end = quoted_string_end(bytes, i, bytes[i]);
            fill_kind(kinds, i, end, HighlightKind::String);
            i = end;
            continue;
        }
        if ch.is_ascii_digit() {
            let end = scan_number(bytes, i);
            fill_kind(kinds, i, end, HighlightKind::Number);
            i = end;
            continue;
        }
        if is_ident_start(ch) {
            let end = scan_identifier(bytes, i);
            if let Some(kind) = fallback_keyword_kind(hint, &source[i..end]) {
                fill_kind(kinds, i, end, kind);
            }
            i = end;
            continue;
        }
        if is_operator_byte(bytes[i]) {
            kinds[i] = HighlightKind::Operator;
            i += 1;
            continue;
        }
        i += source[i..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
}

fn fill_kind(
    kinds: &mut [phosphor::HighlightKind],
    start: usize,
    end: usize,
    kind: phosphor::HighlightKind,
) {
    let len = kinds.len();
    let start = start.min(len);
    let end = end.min(len);
    for slot in &mut kinds[start..end] {
        *slot = kind;
    }
}

fn supports_slash_comments(hint: CodeLanguageHint) -> bool {
    !matches!(
        hint,
        CodeLanguageHint::Python | CodeLanguageHint::Shell | CodeLanguageHint::Toml
    )
}

fn supports_hash_comments(hint: CodeLanguageHint) -> bool {
    matches!(
        hint,
        CodeLanguageHint::Python | CodeLanguageHint::Shell | CodeLanguageHint::Nix
    )
}

fn supports_backtick_strings(hint: CodeLanguageHint) -> bool {
    matches!(
        hint,
        CodeLanguageHint::JavaScript
            | CodeLanguageHint::TypeScript
            | CodeLanguageHint::TypeScriptTsx
    )
}

fn quoted_string_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn scan_number(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len()
        && ((bytes[i] as char).is_ascii_alphanumeric() || matches!(bytes[i], b'.' | b'_'))
    {
        i += 1;
    }
    i
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}

fn scan_identifier(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_ident_continue(bytes[i] as char) {
        i += 1;
    }
    i
}

fn is_operator_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'=' | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'!'
            | b'<'
            | b'>'
            | b'&'
            | b'|'
            | b'^'
            | b'?'
            | b':'
            | b'.'
            | b','
            | b';'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'['
            | b']'
    )
}

fn fallback_keyword_kind(hint: CodeLanguageHint, word: &str) -> Option<phosphor::HighlightKind> {
    use phosphor::HighlightKind;
    match hint {
        CodeLanguageHint::JavaScript
        | CodeLanguageHint::TypeScript
        | CodeLanguageHint::TypeScriptTsx => match word {
            "import" | "from" | "export" | "default" | "const" | "let" | "var" | "function"
            | "return" | "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "break"
            | "continue" | "class" | "extends" | "implements" | "new" | "try" | "catch"
            | "finally" | "throw" | "async" | "await" | "yield" | "of" | "in" | "as" | "type"
            | "interface" | "enum" | "namespace" | "private" | "public" | "protected"
            | "readonly" | "static" => Some(HighlightKind::Keyword),
            "true" | "false" | "null" | "undefined" | "NaN" | "Infinity" => {
                Some(HighlightKind::Constant)
            }
            "Array" | "Boolean" | "Error" | "Map" | "Number" | "Object" | "Promise" | "Record"
            | "Set" | "String" | "Symbol" | "unknown" | "never" | "void" => {
                Some(HighlightKind::Type)
            }
            "console" | "JSON" | "Math" | "Date" | "RegExp" => Some(HighlightKind::Builtin),
            _ => None,
        },
        CodeLanguageHint::Rust => match word {
            "as" | "async" | "await" | "break" | "const" | "continue" | "crate" | "else"
            | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "in" | "let"
            | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "self"
            | "Self" | "static" | "struct" | "super" | "trait" | "true" | "type" | "unsafe"
            | "use" | "where" | "while" => Some(HighlightKind::Keyword),
            _ => None,
        },
        CodeLanguageHint::Python => match word {
            "and" | "as" | "assert" | "async" | "await" | "break" | "class" | "continue"
            | "def" | "del" | "elif" | "else" | "except" | "finally" | "for" | "from"
            | "global" | "if" | "import" | "in" | "is" | "lambda" | "nonlocal" | "not" | "or"
            | "pass" | "raise" | "return" | "try" | "while" | "with" | "yield" => {
                Some(HighlightKind::Keyword)
            }
            "True" | "False" | "None" => Some(HighlightKind::Constant),
            _ => None,
        },
        CodeLanguageHint::Json => match word {
            "true" | "false" | "null" => Some(HighlightKind::Constant),
            _ => None,
        },
        _ => None,
    }
}

/// Renders a markdown `body` to the same prose/code-block elements a comment uses,
/// for the composer's Preview tab. No selection (read-only preview).
pub(crate) fn render_markdown_body(
    body: &str,
    theme: &Theme,
    ui_scale: f32,
    text_w: f32,
    fallback_path: Option<&str>,
) -> AnyElement {
    let tc = &theme.colors;
    let small = theme.metrics.ui_small_font_size;
    let mut els: Vec<AnyElement> = Vec::new();
    for block in segment_comment_blocks(body) {
        match block {
            CommentBlock::Prose(inline) => {
                let spans: Vec<StyledSpan> = inline
                    .into_iter()
                    .map(|s| inline_span_to_styled(s, tc))
                    .collect();
                if spans.iter().all(|s| s.text.is_empty()) {
                    continue;
                }
                els.push(
                    selectable_rich_text(spans)
                        .width(text_w)
                        .size(small)
                        .color(tc.text)
                        .max_lines(64)
                        .into_any(),
                );
            }
            CommentBlock::Heading { level, inline } => {
                let spans: Vec<StyledSpan> = inline
                    .into_iter()
                    .map(|s| inline_span_to_styled_with_weight(s, tc, FontWeight::Semibold))
                    .collect();
                if spans.iter().all(|s| s.text.is_empty()) {
                    continue;
                }
                els.push(
                    selectable_rich_text(spans)
                        .width(text_w)
                        .size(heading_font_size(level, small))
                        .color(tc.text_strong)
                        .max_lines(3)
                        .into_any(),
                );
            }
            CommentBlock::Code { lang, source } => {
                if source.is_empty() {
                    continue;
                }
                let highlighted = highlight_code_lines(lang.as_deref(), &source, tc, fallback_path);
                els.push(
                    code_block(highlighted)
                        .width(text_w)
                        .size(small * 0.9)
                        .into_any(),
                );
            }
        }
    }
    if els.is_empty() {
        els.push(
            text("Nothing to preview")
                .size(small)
                .color(tc.text_muted)
                .into_any(),
        );
    }
    view! { ui_scale,
        <div class="flex-col" gap={Sp::XXS} min_w={0.0}>
            {...els}
        </div>
    }
}

/// Like `clean_markdown_line` but keeps inline markup (only block prefixes and
/// fences are removed), so the inline parser can recover emphasis.
#[cfg(test)]
fn block_strip_keep_inline(line: &str) -> String {
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
    collapse_whitespace(strip_ordered_list_marker(s))
}

#[cfg(test)]
fn flush_normal(normal: &mut String, spans: &mut Vec<InlineSpan>) {
    if !normal.is_empty() {
        spans.push(InlineSpan {
            text: std::mem::take(normal),
            style: InlineStyle::Normal,
        });
    }
}

#[cfg(test)]
fn find_double(chars: &[char], start: usize, marker: char) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == marker && chars[i + 1] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Flat inline-markdown scanner: emphasis does not nest (first match wins), and an
/// unbalanced marker is emitted as literal text.
#[cfg(test)]
fn parse_inline(s: &str) -> Vec<InlineSpan> {
    let chars: Vec<char> = s.chars().collect();
    let mut spans: Vec<InlineSpan> = Vec::new();
    let mut normal = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // `code`
        if c == '`'
            && let Some(close) = find_char(&chars, i + 1, '`')
            && close > i + 1
        {
            flush_normal(&mut normal, &mut spans);
            spans.push(InlineSpan {
                text: chars[i + 1..close].iter().collect(),
                style: InlineStyle::Code,
            });
            i = close + 1;
            continue;
        }
        // **bold** / __bold__
        if (c == '*' || c == '_')
            && chars.get(i + 1) == Some(&c)
            && let Some(close) = find_double(&chars, i + 2, c)
            && close > i + 2
        {
            flush_normal(&mut normal, &mut spans);
            spans.push(InlineSpan {
                text: chars[i + 2..close].iter().collect(),
                style: InlineStyle::Bold,
            });
            i = close + 2;
            continue;
        }
        // *italic* / _italic_ (no space right after the marker)
        if (c == '*' || c == '_')
            && chars
                .get(i + 1)
                .is_some_and(|n| !n.is_whitespace() && *n != c)
            && let Some(close) = find_char(&chars, i + 1, c)
        {
            flush_normal(&mut normal, &mut spans);
            spans.push(InlineSpan {
                text: chars[i + 1..close].iter().collect(),
                style: InlineStyle::Italic,
            });
            i = close + 1;
            continue;
        }
        // [text](url) — keep the text, styled as a link
        if c == '['
            && let Some(close) = find_char(&chars, i + 1, ']')
            && close > i + 1
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = find_char(&chars, close + 2, ')')
        {
            flush_normal(&mut normal, &mut spans);
            spans.push(InlineSpan {
                text: chars[i + 1..close].iter().collect(),
                style: InlineStyle::Link,
            });
            i = end + 1;
            continue;
        }
        normal.push(c);
        i += 1;
    }
    flush_normal(&mut normal, &mut spans);
    if spans.is_empty() {
        spans.push(InlineSpan {
            text: String::new(),
            style: InlineStyle::Normal,
        });
    }
    spans
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
    let node_id = first.and_then(|comment| comment.backend_node_id.clone());

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

    // Author-only actions. Edit/Delete need the comment's node id; Hide has no
    // backend yet, so it stays disabled.
    if can_edit || can_delete {
        entries.push(ContextMenuEntry::separator());
        if can_edit {
            entries.push(match node_id.clone() {
                Some(comment_node_id) => ContextMenuEntry::item(
                    "Edit",
                    GitHubAction::EditReviewComment { comment_node_id }.into(),
                )
                .icon(lucide::PENCIL),
                None => ContextMenuEntry::item("Edit", Action::Noop)
                    .icon(lucide::PENCIL)
                    .disabled(),
            });
            entries.push(
                ContextMenuEntry::item("Hide", Action::Noop)
                    .icon(lucide::EYE_OFF)
                    .disabled(),
            );
        }
        if can_delete {
            entries.push(match node_id {
                Some(comment_node_id) => ContextMenuEntry::item(
                    "Delete",
                    GitHubAction::DeleteReviewComment { comment_node_id }.into(),
                )
                .icon(lucide::X)
                .destructive(),
                None => ContextMenuEntry::item("Delete", Action::Noop)
                    .icon(lucide::X)
                    .destructive()
                    .disabled(),
            });
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
    fn parses_inline_emphasis_into_styled_spans() {
        let spans = parse_comment_body_rich(
            "Use `withDeepSerpConfigs` but it **hard-codes** team to *null*; see [the docs](https://x).",
        );
        let styled: Vec<(InlineStyle, &str)> =
            spans.iter().map(|s| (s.style, s.text.as_str())).collect();
        assert_eq!(
            styled,
            vec![
                (InlineStyle::Normal, "Use "),
                (InlineStyle::Code, "withDeepSerpConfigs"),
                (InlineStyle::Normal, " but it "),
                (InlineStyle::Bold, "hard-codes"),
                (InlineStyle::Normal, " team to "),
                (InlineStyle::Italic, "null"),
                (InlineStyle::Normal, "; see "),
                (InlineStyle::Link, "the docs"),
                (InlineStyle::Normal, "."),
            ]
        );
    }

    #[test]
    fn rich_parse_strips_block_markers_and_html_comments() {
        let spans =
            parse_comment_body_rich("<!-- meta -->\n> ## Heading with `code`\n- a bullet point");
        let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(plain, "Heading with code a bullet point");
        assert!(
            spans
                .iter()
                .any(|s| s.style == InlineStyle::Code && s.text == "code")
        );
    }

    #[test]
    fn rich_parse_emits_literal_for_unbalanced_markers() {
        let spans = parse_comment_body_rich("a * lone star and `unclosed code");
        let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(plain, "a * lone star and `unclosed code");
        assert!(spans.iter().all(|s| s.style == InlineStyle::Normal));
    }

    #[test]
    fn rich_parse_concatenation_is_the_plain_body() {
        // The concatenated span texts are the rendered *visible* text — links drop
        // the URL, code/emphasis drop their markers — and that exact string is what
        // selection/copy operate on. Hold the invariant across every inline form,
        // including the `_`/`__` syntax the other tests never exercise.
        let cases = [
            ("Fix **the** `bug` now", "Fix the bug now"),
            ("Fix __the__ `bug` now", "Fix the bug now"),
            ("use _this_ not `that`", "use this not that"),
            ("see [the docs](https://x/y?z=1) here", "see the docs here"),
            ("regex `a*b_c` stays literal", "regex a*b_c stays literal"),
        ];
        for (body, want) in cases {
            let plain: String = parse_comment_body_rich(body)
                .iter()
                .map(|s| s.text.as_str())
                .collect();
            assert_eq!(
                plain, want,
                "concatenation must equal visible text for {body:?}"
            );
        }
    }

    #[test]
    fn rich_parse_handles_underscore_emphasis_syntax() {
        let spans = parse_comment_body_rich("make __it__ _bold_ please");
        let styled: Vec<(InlineStyle, &str)> =
            spans.iter().map(|s| (s.style, s.text.as_str())).collect();
        assert_eq!(
            styled,
            vec![
                (InlineStyle::Normal, "make "),
                (InlineStyle::Bold, "it"),
                (InlineStyle::Normal, " "),
                (InlineStyle::Italic, "bold"),
                (InlineStyle::Normal, " please"),
            ]
        );
    }

    #[test]
    fn rich_parse_code_span_wins_over_inline_markers() {
        // Code is scanned first, so emphasis/link markers inside backticks stay
        // literal — the renderer must show `a*b*c`, not an italic run, and the
        // bracket text must not become a link.
        let spans = parse_comment_body_rich("call `fn(a*b*c)` and `[x](y)` now");
        let code: Vec<&str> = spans
            .iter()
            .filter(|s| s.style == InlineStyle::Code)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(code, vec!["fn(a*b*c)", "[x](y)"]);
        assert!(
            !spans
                .iter()
                .any(|s| matches!(s.style, InlineStyle::Italic | InlineStyle::Link)),
            "markers inside a code span must not produce italic/link runs: {spans:?}"
        );
    }

    #[test]
    fn rich_parse_emits_no_empty_spans_between_adjacent_runs() {
        // Adjacent styled runs must not be separated by an empty Normal span, and no
        // span of any style may be empty — empty spans become zero-width positioning
        // artifacts once each piece is laid out individually.
        let spans = parse_comment_body_rich("`a`*b*__c__[d](e)");
        let styled: Vec<(InlineStyle, &str)> =
            spans.iter().map(|s| (s.style, s.text.as_str())).collect();
        assert_eq!(
            styled,
            vec![
                (InlineStyle::Code, "a"),
                (InlineStyle::Italic, "b"),
                (InlineStyle::Bold, "c"),
                (InlineStyle::Link, "d"),
            ]
        );
        assert!(
            spans.iter().all(|s| !s.text.is_empty()),
            "no span may be empty: {spans:?}"
        );
    }

    #[test]
    fn rich_parse_empty_link_text_is_literal_not_an_empty_run() {
        // `[](url)` has no visible text; rather than emit an empty Link span the
        // scanner falls through and keeps the source literally.
        let spans = parse_comment_body_rich("before [](https://x) after");
        let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(plain, "before [](https://x) after");
        assert!(
            !spans.iter().any(|s| s.style == InlineStyle::Link),
            "empty-text link must not become a Link run: {spans:?}"
        );
    }

    #[test]
    fn rich_parse_caps_total_length() {
        // The cap engages as lines are joined, so a many-line body is what bounds.
        let body = "word\n".repeat(400);
        let spans = parse_comment_body_rich(&body);
        let chars: usize = spans.iter().map(|s| s.text.chars().count()).sum();
        assert!(
            (600..=610).contains(&chars),
            "joined body must be bounded near MAX_CHARS=600; got {chars} chars"
        );
    }

    #[test]
    fn metadata_prefixed_comment_is_readable() {
        let body = "<!-- tool-marker {\"id\": \"x1\"} -->\n\
                    **Heading** with `code` and trailing words";
        let cleaned = clean_comment_body(body);
        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0], "Heading with code and trailing words");
    }

    #[test]
    fn segments_prose_code_prose_in_order() {
        let body = "before text\n```rust\nlet x = 1;\nlet y = 2;\n```\nafter text";
        let blocks = segment_comment_blocks(body);
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            CommentBlock::Prose(spans) => {
                let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
                assert_eq!(plain, "before text");
            }
            other => panic!("expected prose, got {other:?}"),
        }
        match &blocks[1] {
            CommentBlock::Code { lang, source } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(source, "let x = 1;\nlet y = 2;");
            }
            other => panic!("expected code, got {other:?}"),
        }
        match &blocks[2] {
            CommentBlock::Prose(spans) => {
                let plain: String = spans.iter().map(|s| s.text.as_str()).collect();
                assert_eq!(plain, "after text");
            }
            other => panic!("expected prose, got {other:?}"),
        }
    }

    #[test]
    fn segments_long_markdown_code_without_truncation() {
        let prefix = "Execution keeps going after cancellation. ".repeat(20);
        let code = "const graph = compileDeepCanonSearchGraph({ ...req, featureFlags });\n\
const result = await processCanonGraphRequest(graph, new AbortController().signal, {\n\
  featureFlags,\n\
  searchEnv: req.env,\n\
});\n\
\n\
const requestScopedResult = await processCanonGraphRequest(graph, req.signal, {\n\
  featureFlags,\n\
  searchEnv: req.env,\n\
});";
        let body = format!("{prefix}\n```ts\n{code}\n```\nafter text");
        assert!(body.chars().count() > 600);

        let blocks = segment_comment_blocks(&body);
        let code_block = blocks
            .iter()
            .find_map(|block| match block {
                CommentBlock::Code { lang, source } => Some((lang, source)),
                _ => None,
            })
            .expect("code block");

        assert_eq!(code_block.0.as_deref(), Some("ts"));
        assert_eq!(code_block.1, code);
        assert!(blocks.iter().any(|block| {
            match block {
                CommentBlock::Prose(inline) => {
                    inline
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                        == "after text"
                }
                _ => false,
            }
        }));
    }

    #[test]
    fn segments_inline_opening_fence_as_code_block() {
        let body = "[🟡 Medium] [🔵 Bug]\n\
withDeepSerpConfigs rebuilds flags and changes behavior. ```ts // typescript/vulcan/vulcan/src/services/deep/deep-canon-search.ts\n\
return new FeatureFlags({ team: null, posthogFlags: flags.posthogFlags });\n\
```\n\
Preserve the original team metadata when cloning.";
        let blocks = segment_comment_blocks(body);
        let code_blocks = blocks
            .iter()
            .filter(|block| matches!(block, CommentBlock::Code { .. }))
            .count();
        assert_eq!(code_blocks, 1);

        let code_block = blocks
            .iter()
            .find_map(|block| match block {
                CommentBlock::Code { lang, source } => Some((lang, source)),
                _ => None,
            })
            .expect("code block");
        assert_eq!(code_block.0.as_deref(), Some("ts"));
        assert_eq!(
            code_block.1,
            "return new FeatureFlags({ team: null, posthogFlags: flags.posthogFlags });"
        );
        assert!(blocks.iter().any(|block| {
            match block {
                CommentBlock::Prose(inline) => {
                    inline
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                        == "Preserve the original team metadata when cloning."
                }
                _ => false,
            }
        }));
    }

    #[test]
    fn segments_markdown_headings_with_levels() {
        let body = "# Top\n\n## Mid with `code`\n\n### Low\n\nplain text";
        let blocks = segment_comment_blocks(body);
        assert_eq!(blocks.len(), 4);

        let plain =
            |inline: &[InlineSpan]| -> String { inline.iter().map(|s| s.text.as_str()).collect() };

        match &blocks[0] {
            CommentBlock::Heading { level, inline } => {
                assert_eq!(*level, 1);
                assert_eq!(plain(inline), "Top");
            }
            other => panic!("expected h1, got {other:?}"),
        }
        match &blocks[1] {
            CommentBlock::Heading { level, inline } => {
                assert_eq!(*level, 2);
                assert_eq!(plain(inline), "Mid with code");
                assert!(
                    inline
                        .iter()
                        .any(|s| s.style == InlineStyle::Code && s.text == "code")
                );
            }
            other => panic!("expected h2, got {other:?}"),
        }
        match &blocks[2] {
            CommentBlock::Heading { level, inline } => {
                assert_eq!(*level, 3);
                assert_eq!(plain(inline), "Low");
            }
            other => panic!("expected h3, got {other:?}"),
        }
        match &blocks[3] {
            CommentBlock::Prose(inline) => assert_eq!(plain(inline), "plain text"),
            other => panic!("expected prose, got {other:?}"),
        }
    }

    #[test]
    fn segments_markdown_lists_into_visible_prefixes() {
        let body = "- first\n- second\n\n3. third\n4. fourth";
        let blocks = segment_comment_blocks(body);
        assert_eq!(blocks.len(), 4);
        let visible: Vec<String> = blocks
            .iter()
            .map(|block| match block {
                CommentBlock::Prose(inline) => inline.iter().map(|s| s.text.as_str()).collect(),
                other => panic!("expected prose, got {other:?}"),
            })
            .collect();
        assert_eq!(
            visible,
            vec!["• first", "• second", "3. third", "4. fourth"]
        );
    }

    #[test]
    fn code_lines_tile_their_source_exactly() {
        // Per-line spans must concatenate back to the exact source line.
        let theme = Theme::default_dark();
        let source = "fn main() {\n    let x = 1;\n}";
        let lines = highlight_code_lines(Some("rust"), source, &theme.colors, None);
        let rebuilt: Vec<String> = lines
            .iter()
            .map(|line| line.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert_eq!(
            rebuilt,
            vec!["fn main() {", "    let x = 1;", "}"],
            "concatenated spans must equal each source line"
        );
        assert!(
            lines
                .iter()
                .flatten()
                .all(|s| s.font_kind == FontKind::Mono),
            "every code span renders in the mono font"
        );
    }

    #[test]
    fn javascript_code_fence_gets_visible_keyword_and_string_colors() {
        let theme = Theme::default_dark();
        let source = "import { x } from \"y\";";
        let lines = highlight_code_lines(Some("js"), source, &theme.colors, None);
        let spans: Vec<&StyledSpan> = lines.iter().flatten().collect();

        assert!(spans.iter().any(|span| {
            span.text == "import" && span.color == Some(theme.colors.syntax_keyword)
        }));
        assert!(spans.iter().any(|span| {
            span.text == "\"y\"" && span.color == Some(theme.colors.syntax_string)
        }));
    }

    #[test]
    fn unlabeled_code_fence_can_fall_back_to_review_file_path() {
        let theme = Theme::default_dark();
        let source = "import { x } from \"y\";";
        let lines = highlight_code_lines(None, source, &theme.colors, Some("src/app.ts"));
        let spans: Vec<&StyledSpan> = lines.iter().flatten().collect();

        assert!(spans.iter().any(|span| {
            span.text == "import" && span.color == Some(theme.colors.syntax_keyword)
        }));
    }

    #[test]
    fn typoed_javascript_fence_can_still_infer_from_source() {
        let theme = Theme::default_dark();
        let source = "import { x } from \"y\";";
        let lines = highlight_code_lines(Some("javascrpt"), source, &theme.colors, None);
        let spans: Vec<&StyledSpan> = lines.iter().flatten().collect();

        assert!(spans.iter().any(|span| {
            span.text == "import" && span.color == Some(theme.colors.syntax_keyword)
        }));
    }
}
