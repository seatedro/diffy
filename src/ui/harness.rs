//! Shared dev/test substrate for the GPU-UI "browser devtools": deterministic
//! fixtures plus a no-GPU path to render the review-thread card into a scene and
//! capture its selectable-text regions, accessibility tree, and hit regions.

use std::collections::HashMap;

use crate::core::review::{
    ReviewAnchor, ReviewComment, ReviewCommentId, ReviewLineRange, ReviewResolution, ReviewSide,
    ReviewThread, ReviewThreadId, ReviewThreadPermissions, ReviewThreadStatus,
};
use crate::render::Scene;
use crate::ui::accessibility::AccessibilityFrame;
use crate::ui::editor::review::{build_review_thread_card, measure_review_thread_card_height};
use crate::ui::element::{ElementContext, HitRegion, SelectableTextRegion, render_element_at};
use crate::ui::state::CardTextSelection;
use crate::ui::theme::Theme;
use halogen::reactive::SignalStore;

/// Physical card width for the harness render. Wide enough that the header line
/// reference doesn't truncate at `HARNESS_UI_SCALE` (a real review column is wider
/// still); the body bodies wrap to several lines for selection coverage.
pub const HARNESS_CARD_WIDTH: f32 = 1040.0;
/// Must be != 1.0 so the avatar/icon `ui_scale` multiply is exercised — the avatar
/// double-scale bug is invisible at 1.0 (bug and fix both yield the base size).
pub const HARNESS_UI_SCALE: f32 = 2.0;

fn fixture_comment(
    thread_id: &ReviewThreadId,
    backend_id: i64,
    author: &str,
    created_at: &str,
    body: &str,
) -> ReviewComment {
    ReviewComment {
        id: ReviewCommentId::github(backend_id),
        backend_id: Some(backend_id),
        backend_node_id: Some(format!("node-{backend_id}")),
        thread_id: thread_id.clone(),
        in_reply_to: None,
        in_reply_to_node_id: None,
        author_login: Some(author.to_owned()),
        author_avatar_url: None,
        body: body.to_owned(),
        anchor: None,
        html_url: None,
        created_at: Some(created_at.to_owned()),
        updated_at: None,
        outdated: false,
        state: None,
        viewer_can_update: false,
        viewer_can_delete: false,
        reactions: Vec::new(),
    }
}

/// Neutral fixture thread: comment 0 plain (drag/copy guard), comment 1 inline
/// markdown, comment 2 prose + a fenced code block.
pub fn sample_review_thread() -> ReviewThread {
    let id = ReviewThreadId::github_node("harness-thread-1");
    let anchor = ReviewAnchor::inline(
        "src/widget.rs",
        ReviewSide::New,
        ReviewLineRange::new(120, 1),
    );
    let comments = vec![
        fixture_comment(
            &id,
            1001,
            "alpha",
            "2026-01-01T00:00:00Z",
            "This recomputes the layout every frame even when nothing changed, which \
             makes scrolling heavier than it should be on larger documents.",
        ),
        fixture_comment(
            &id,
            1002,
            "bravo",
            "2026-01-01T00:05:00Z",
            "`memoize_height()` over recompute keeps it **stable** while *pinned*; \
             see [the notes](https://example.com/perf).",
        ),
        fixture_comment(
            &id,
            1003,
            "alpha",
            "2026-01-01T00:10:00Z",
            "Cache it behind a dirty flag:\n```rust\nfn cached_height(w: u32) -> u16 {\n    cache.entry(w).or_insert(measure(w))\n}\n```",
        ),
    ];
    ReviewThread {
        id,
        backend_node_id: Some("harness-thread-1".to_owned()),
        anchor: Some(anchor),
        comments,
        status: ReviewThreadStatus {
            resolution: ReviewResolution::Unresolved,
            outdated: false,
            collapsed: false,
        },
        permissions: ReviewThreadPermissions {
            can_reply: true,
            can_resolve: true,
            can_unresolve: false,
        },
    }
}

/// A selection over comment 1 spanning the code run into normal text, for
/// eyeballing the highlight across a style boundary.
pub fn sample_card_selection() -> Option<CardTextSelection> {
    let thread = sample_review_thread();
    let key = crate::ui::editor::review::card_source_key(&thread.id, 1);
    let card = render_review_card(&thread, true, None);
    let region = card.selectable.iter().find(|r| r.source_key == key)?;
    let anchor = region.text.find("memoize")?;
    let after_code = region.text.find("while")? + "while".len();
    Some(CardTextSelection {
        source_key: key,
        text: region.text.clone(),
        anchor,
        focus: after_code,
    })
}

/// A painted text run with its pen position, for numeric geometry checks.
#[derive(Debug, Clone)]
pub struct TextPiece {
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub font_size: f32,
    pub font_kind: crate::render::FontKind,
    pub font_weight: crate::render::FontWeight,
    pub italic: bool,
    pub text: String,
}

/// Every `TextRun`/`RichTextRun` in the scene as a [`TextPiece`], sorted top-to-
/// bottom then left-to-right (reading order).
pub fn text_pieces(scene: &Scene) -> Vec<TextPiece> {
    use crate::render::{FontStyle, Primitive};

    let mut out = Vec::new();
    for primitive in &scene.primitives {
        match primitive {
            Primitive::TextRun(t) => out.push(TextPiece {
                x: t.rect.x,
                y: t.rect.y,
                height: t.rect.height,
                font_size: t.font_size,
                font_kind: t.font_kind,
                font_weight: t.font_weight,
                italic: false,
                text: t.text.to_string(),
            }),
            Primitive::RichTextRun(r) => {
                let text: String = r.spans.iter().map(|s| s.text.as_ref()).collect();
                let italic = r
                    .spans
                    .iter()
                    .any(|s| s.font_style == Some(FontStyle::Italic));
                let weight = r
                    .spans
                    .iter()
                    .find_map(|s| s.font_weight)
                    .unwrap_or(r.font_weight);
                out.push(TextPiece {
                    x: r.rect.x,
                    y: r.rect.y,
                    height: r.rect.height,
                    font_size: r.font_size,
                    font_kind: r.font_kind,
                    font_weight: weight,
                    italic,
                    text,
                });
            }
            _ => {}
        }
    }
    out.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    out
}

/// [`text_pieces`] grouped into visual lines (by `y` band), each sorted by `x`.
pub fn text_lines(scene: &Scene) -> Vec<Vec<TextPiece>> {
    let mut lines: Vec<Vec<TextPiece>> = Vec::new();
    for piece in text_pieces(scene) {
        match lines.last_mut() {
            Some(line) if (piece.y - line[0].y).abs() < line[0].height * 0.6 => line.push(piece),
            _ => lines.push(vec![piece]),
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    lines
}

/// Geometry dump: one line per text piece with its measured advance and gap to the
/// next on the line. Gap `+N` = an N-px space, `+0` = abutting, negative = overlap.
pub fn dump_text_layout(scene: &Scene, font_system: &mut glyphon::FontSystem) -> String {
    let mut out = String::new();
    for line in text_lines(scene) {
        for (i, p) in line.iter().enumerate() {
            let adv = crate::ui::element::measure_text_advance(
                font_system,
                &p.text,
                p.font_size,
                p.font_kind,
                p.font_weight,
            );
            let gap = line
                .get(i + 1)
                .map(|q| format!("{:+.0}", q.x - (p.x + adv)))
                .unwrap_or_else(|| "·".to_owned());
            let kind = match p.font_kind {
                crate::render::FontKind::Ui => "ui",
                crate::render::FontKind::Mono => "mono",
            };
            let style = format!(
                "{kind}/{:?}{}",
                p.font_weight,
                if p.italic { "+i" } else { "" }
            );
            out.push_str(&format!(
                "y={:.0} x={:.0} adv={adv:.0} gap={gap} {style} {:?}\n",
                p.y, p.x, p.text
            ));
        }
    }
    out
}

/// Outputs captured from rendering a review-thread card without a GPU.
pub struct RenderedCard {
    pub scene: Scene,
    pub selectable: Vec<SelectableTextRegion>,
    pub accessibility: AccessibilityFrame,
    pub hits: Vec<HitRegion>,
    pub width: f32,
    pub height: f32,
}

/// Build a dark-theme `ElementContext`, render the review-thread card at a fixed
/// width/scale with an empty avatar map and the given selection, and return the
/// resulting scene plus the captured devtools outputs.
pub fn render_review_card(
    thread: &ReviewThread,
    expanded: bool,
    selection: Option<&CardTextSelection>,
) -> RenderedCard {
    render_review_card_sized(
        thread,
        expanded,
        selection,
        HARNESS_CARD_WIDTH,
        HARNESS_UI_SCALE,
    )
}

/// Render at a caller-chosen physical `width` and `scale`. The example uses a small
/// size so the PNG comes back full-resolution; tests use the constants.
pub fn render_review_card_sized(
    thread: &ReviewThread,
    expanded: bool,
    selection: Option<&CardTextSelection>,
    width: f32,
    scale: f32,
) -> RenderedCard {
    render_review_card_with_avatars(thread, expanded, selection, &HashMap::new(), width, scale)
}

/// Same as [`render_review_card_sized`] but with a caller-supplied avatar map, so
/// tests can exercise the fetched-image avatar path (keyed by `avatar_cache_key`).
fn render_review_card_with_avatars(
    thread: &ReviewThread,
    expanded: bool,
    selection: Option<&CardTextSelection>,
    avatars: &HashMap<u64, crate::ui::components::avatar::AvatarImage>,
    width: f32,
    scale: f32,
) -> RenderedCard {
    // Scale the theme's metrics like the live app does (`Theme::with_ui_scale`), so
    // elements which read `cx.theme.metrics.ui_scale()` (avatars, icons) scale
    // consistently with spacing/fonts that take the explicit `scale` — otherwise the
    // render mixes 1x icons/avatars with 2x spacing.
    let theme = Theme::default_dark().with_ui_scale(scale);
    let mut font_system = crate::fonts::new_font_system();
    let store = SignalStore::new();

    let height = {
        let mut measure_cx = ElementContext::new(&theme, scale, &mut font_system, None, &store);
        f32::from(measure_review_thread_card_height(
            thread,
            expanded,
            &theme,
            scale,
            width,
            &mut measure_cx,
        ))
    };

    let mut scene = Scene::default();
    let mut cx = ElementContext::new(&theme, scale, &mut font_system, None, &store);
    let mut card =
        build_review_thread_card(thread, expanded, &theme, scale, width, avatars, selection);
    render_element_at(&mut card, &mut scene, &mut cx, 0.0, 0.0, width, height);

    RenderedCard {
        scene,
        selectable: std::mem::take(&mut cx.selectable_text_runs),
        accessibility: std::mem::take(&mut cx.accessibility),
        hits: std::mem::take(&mut cx.hits),
        width,
        height,
    }
}

/// Render the review-comment composer with a sample open draft, for eyeballing.
pub fn render_review_composer(width: f32, scale: f32) -> RenderedCard {
    use crate::core::forge::github::{CreatePullRequestReviewComment, GitHubReviewSide};
    use crate::render::Rect;
    use crate::ui::state::{
        AppState, AsyncStatus, FocusTarget, ReviewCommentComposerState, ReviewCommentDraft,
    };

    let theme = Theme::default_dark().with_ui_scale(scale);
    let mut font_system = crate::fonts::new_font_system();
    let mut state = AppState::default();
    let small = theme.metrics.ui_small_font_size;

    state.github.pull_request.review_composer.set(
        &state.store,
        ReviewCommentComposerState {
            draft: Some(ReviewCommentDraft {
                key: ("owner".to_owned(), "repo".to_owned(), 1),
                request: CreatePullRequestReviewComment {
                    body: String::new(),
                    commit_id: "deadbeef".to_owned(),
                    path: "src/widget.rs".to_owned(),
                    line: 263,
                    side: GitHubReviewSide::Right,
                    start_line: None,
                    start_side: None,
                },
            }),
            status: AsyncStatus::Ready,
            message: None,
            reply_target: None,
            edit_target: None,
        },
    );
    state
        .focus
        .set(&state.store, Some(FocusTarget::ReviewCommentEditor));

    state
        .review_comment_editor
        .set_font_size(&mut font_system, small);
    let inner_w = (width - 64.0 * scale).max(50.0);
    state
        .review_comment_editor
        .set_size(&mut font_system, inner_w, 120.0 * scale);
    state
        .review_comment_editor
        .set_text("Consider memoizing this height so we don't recompute it every frame.");

    let height = (248.0 * scale).round();
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width,
        height,
    };
    let mut scene = Scene::default();
    let mut cx = ElementContext::new(&theme, scale, &mut font_system, None, &state.store);
    let mut element = crate::ui::shell::build_review_composer(&state, &theme, scale, rect);
    render_element_at(&mut element, &mut scene, &mut cx, 0.0, 0.0, width, height);

    RenderedCard {
        scene,
        selectable: std::mem::take(&mut cx.selectable_text_runs),
        accessibility: std::mem::take(&mut cx.accessibility),
        hits: std::mem::take(&mut cx.hits),
        width,
        height,
    }
}

/// A no-GPU input driver: owns the application state, an `InputSystem`, a CPU
/// `FontSystem`, and a minimal `UiFrame` whose `selectable_text_runs` come from
/// rendering the fixture review card. It feeds synthetic pointer/key events
/// through the real input layer and applies every emitted action to `state`, so
/// tests can exercise selection + clipboard behaviour end to end without a window.
#[cfg(test)]
pub struct UiHarness {
    pub state: crate::ui::state::AppState,
    input: crate::input::InputSystem,
    font_system: glyphon::FontSystem,
    editor: crate::ui::editor::element::EditorElement,
    tooltip: crate::ui::components::TooltipState,
    launch_at: std::time::Instant,
    ui_frame: crate::ui::shell::UiFrame,
    pub card: RenderedCard,
}

#[cfg(test)]
impl UiHarness {
    /// Build a harness around the sample review-thread card. The minimal
    /// `UiFrame` carries only the card's selectable-text runs and scene (no
    /// viewport), which is all the card-selection input paths read.
    pub fn new(thread: &ReviewThread) -> Self {
        let card = render_review_card(thread, true, None);
        let ui_frame = crate::ui::shell::UiFrame {
            selectable_text_runs: card.selectable.clone(),
            scene: card.scene.clone(),
            accessibility: card.accessibility.clone(),
            hits: card.hits.clone(),
            ..crate::ui::shell::UiFrame::default()
        };
        Self {
            state: crate::ui::state::AppState::default(),
            input: crate::input::InputSystem::default(),
            font_system: crate::fonts::new_font_system(),
            editor: crate::ui::editor::element::EditorElement::default(),
            tooltip: crate::ui::components::TooltipState::default(),
            launch_at: std::time::Instant::now(),
            ui_frame,
            card,
        }
    }

    /// The selectable region for comment `index`, matched by its source key.
    pub fn region_for_comment(&self, thread: &ReviewThread, index: usize) -> &SelectableTextRegion {
        let key = crate::ui::editor::review::card_source_key(&thread.id, index);
        self.card
            .selectable
            .iter()
            .find(|region| region.source_key == key)
            .expect("selectable region for comment index")
    }

    fn feed(&mut self, event: crate::input::InputEvent) -> crate::input::InputOutcome {
        let outcome = self.input.handle_input_event_for_test(
            &mut self.state,
            &mut self.ui_frame,
            &self.editor,
            Some(&mut self.font_system),
            None,
            &mut self.tooltip,
            self.launch_at,
            event,
        );
        for action in &outcome.actions {
            self.state.apply_action(action.clone());
        }
        outcome
    }

    /// Move the pointer to `(x, y)`, applying any emitted actions.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> crate::input::InputOutcome {
        self.feed(crate::input::InputEvent::PointerMoved { x, y })
    }

    fn left_button(&mut self, pressed: bool) -> crate::input::InputOutcome {
        let state = if pressed {
            winit::event::ElementState::Pressed
        } else {
            winit::event::ElementState::Released
        };
        self.feed(crate::input::InputEvent::PointerButton {
            button: winit::event::MouseButton::Left,
            state,
        })
    }

    /// Press the left button at `(x, y)` (moves the pointer there first so the
    /// click reads the current position).
    pub fn mouse_down(&mut self, x: f32, y: f32) -> crate::input::InputOutcome {
        self.pointer_move(x, y);
        self.left_button(true)
    }

    /// Release the left button, ending any in-progress drag.
    pub fn mouse_up(&mut self) -> crate::input::InputOutcome {
        self.left_button(false)
    }

    /// A full press-and-release click at `(x, y)`.
    pub fn click(&mut self, x: f32, y: f32) {
        self.mouse_down(x, y);
        self.mouse_up();
    }

    /// Press down at `from`, drag through each waypoint, then release. Actions
    /// are applied after every event so the in-progress selection is visible to
    /// subsequent move events.
    pub fn drag(&mut self, from: (f32, f32), waypoints: &[(f32, f32)]) {
        self.mouse_down(from.0, from.1);
        for &(x, y) in waypoints {
            self.pointer_move(x, y);
        }
        self.mouse_up();
    }

    /// Route a key chord through the keyboard layer and return the emitted
    /// actions (the harness does not apply them, so callers can inspect e.g. a
    /// `CopyText` payload).
    pub fn key(&mut self, chord: crate::input::KeyChord) -> Vec<crate::actions::Action> {
        self.input
            .handle_input_event_for_test(
                &mut self.state,
                &mut self.ui_frame,
                &self.editor,
                Some(&mut self.font_system),
                None,
                &mut self.tooltip,
                self.launch_at,
                crate::input::InputEvent::KeyPress(chord),
            )
            .actions
    }

    /// A Cmd/Super + key chord (e.g. `cmd_key("c")` for copy).
    pub fn cmd_key(key: &str) -> crate::input::KeyChord {
        crate::input::KeyChord {
            logical: crate::input::KeyKind::Character(key.to_owned()),
            physical: None,
            modifiers: winit::keyboard::ModifiersState::SUPER,
            repeat: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::ui::accessibility::dump_accessibility;

    #[test]
    fn render_review_card_emits_selectable_text_and_a11y() {
        let thread = sample_review_thread();
        let rendered = render_review_card(&thread, true, None);

        assert!(
            !rendered.selectable.is_empty(),
            "expected at least one selectable text region"
        );

        let dump = dump_accessibility(&rendered.accessibility);
        assert!(!dump.is_empty(), "accessibility dump should be non-empty");
        assert!(
            dump.contains("selectable-text:"),
            "dump should include a selectable-text node:\n{dump}"
        );
    }

    /// (a) Every selectable comment body has a corresponding `Label` a11y node
    /// whose label text is exactly the body string.
    #[test]
    fn every_selectable_body_has_matching_label_node() {
        let thread = sample_review_thread();
        let rendered = render_review_card(&thread, true, None);
        let dump = dump_accessibility(&rendered.accessibility);

        let want: HashSet<&str> = rendered
            .selectable
            .iter()
            .map(|region| region.text.as_str())
            .collect();
        assert_eq!(want.len(), 3, "fixture has three comment bodies");

        let mut got: HashSet<&str> = HashSet::new();
        for line in dump.lines().filter(|l| l.starts_with("selectable-text:")) {
            let parts: Vec<&str> = line.split(" | ").collect();
            assert_eq!(
                parts[1], "Label",
                "selectable body must be a Label:\n{line}"
            );
            got.insert(parts[2]);
        }

        assert_eq!(
            got, want,
            "every selectable body must map to a Label node with that text:\n{dump}"
        );
    }

    /// (b) The card exposes its summary node (the inline-comment line reference)
    /// and its actionable role node (the Resolve button).
    #[test]
    fn card_exposes_summary_and_role_nodes() {
        let thread = sample_review_thread();
        let rendered = render_review_card(&thread, true, None);
        let dump = dump_accessibility(&rendered.accessibility);

        assert!(
            dump.contains("Comment on line R120"),
            "expected the inline summary node:\n{dump}"
        );
        let has_resolve_button = dump
            .lines()
            .any(|l| l.contains("| Button |") && l.contains("Resolve"));
        assert!(
            has_resolve_button,
            "expected a Resolve button role node:\n{dump}"
        );
    }

    /// (c) Regression guard for the duplicate-node-id bug (commit 9c88722).
    #[test]
    fn node_ids_are_unique_across_the_frame() {
        let thread = sample_review_thread();
        let rendered = render_review_card(&thread, true, None);

        // Post-dedup uniqueness in the shipped tree (cheap invariant).
        let update = rendered.accessibility.tree_update(None);
        let ids: Vec<_> = update.nodes.iter().map(|(id, _)| *id).collect();
        let unique: HashSet<_> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "accessibility NodeIds must be unique; {} duplicate(s) found",
            ids.len() - unique.len()
        );

        // The real guard: the card's NATURAL author keys are already unique, so
        // `ensure_unique_id` never had to disambiguate. It suffixes collisions with
        // `#N`; if two card elements ever collide (the 9c88722 bug) this fires —
        // unlike (1), which `ensure_unique_id` makes pass unconditionally.
        let dump = dump_accessibility(&rendered.accessibility);
        let disambiguated: Vec<&str> = dump
            .lines()
            .filter(|line| {
                line.split(" | ")
                    .next()
                    .and_then(|author_id| author_id.rsplit_once('#'))
                    .is_some_and(|(_, suffix)| {
                        !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
                    })
            })
            .collect();
        assert!(
            disambiguated.is_empty(),
            "no card node should need id disambiguation, but these did:\n{}",
            disambiguated.join("\n")
        );
    }

    /// Regression guard for the avatar double-scale bug: a fetched avatar image must
    /// render at `REVIEW_AVATAR_PX * ui_scale`, NOT a multiple of it. `RasterImage`
    /// scales its size by `ui_scale` internally, so passing an already-scaled size
    /// (the original bug) produced a ~4x-oversized avatar. No GPU needed — assert on
    /// the emitted `ImagePrimitive` rect in the scene.
    #[test]
    fn fetched_avatar_renders_at_review_avatar_px() {
        use std::sync::Arc;

        use crate::render::Primitive;
        use crate::ui::components::avatar::AvatarImage;
        use crate::ui::editor::review::{REVIEW_AVATAR_FETCH_PX, REVIEW_AVATAR_PX};
        use crate::ui::state::{avatar_cache_key, avatar_url_sized};

        let mut thread = sample_review_thread();
        let raw = "https://avatars.example.com/u/1.png";
        thread.comments[0].author_avatar_url = Some(raw.to_owned());

        let sized = avatar_url_sized(raw, REVIEW_AVATAR_FETCH_PX).expect("sized url");
        let key = avatar_cache_key(&sized);
        let mut avatars = HashMap::new();
        avatars.insert(
            key,
            AvatarImage {
                rgba: Arc::new(vec![255u8; 4]),
                width: 1,
                height: 1,
                cache_key: key,
            },
        );

        let rendered = render_review_card_with_avatars(
            &thread,
            true,
            None,
            &avatars,
            HARNESS_CARD_WIDTH,
            HARNESS_UI_SCALE,
        );
        // The card also rasterizes the chevron/menu SVGs to Image primitives, so
        // identify the avatar by its cache_key rather than assuming it's the only one.
        let avatar_img = rendered
            .scene
            .primitives
            .iter()
            .find_map(|p| match p {
                Primitive::Image(img) if img.cache_key == key => Some(img),
                _ => None,
            })
            .expect("the fetched comment-0 avatar must emit an ImagePrimitive");

        let expected = REVIEW_AVATAR_PX * HARNESS_UI_SCALE;
        assert!(
            (avatar_img.rect.width - expected).abs() <= 1.5
                && (avatar_img.rect.height - expected).abs() <= 1.5,
            "avatar must render at REVIEW_AVATAR_PX*scale = {expected}px; got {}x{} \
             (a double-scale regression would be ~{}px)",
            avatar_img.rect.width,
            avatar_img.rect.height,
            expected * HARNESS_UI_SCALE
        );
    }

    /// Strengthens the drag test: drive the drag to a KNOWN interior x (the second
    /// word boundary on line 0) so the byte resolution goes through the glyph-distance
    /// loop, not the `x<=0` / far-right saturation guards. Catches off-by-one / sign
    /// errors in `card_text_byte_at` that the full-line drag cannot see.
    #[test]
    fn interior_drag_maps_x_to_the_word_under_the_cursor() {
        let thread = sample_review_thread();
        let mut harness = UiHarness::new(&thread);
        let region = harness.region_for_comment(&thread, 0).clone();

        let line0 = &region.text[region.runs[0].start..region.runs[0].end];
        let first_space = line0.find(' ').expect("line 0 has a first space");
        let second_space = line0[first_space + 1..]
            .find(' ')
            .expect("line 0 has a second word")
            + first_space
            + 1;
        let target_byte = region.runs[0].start + second_space;

        let prefix = &region.text[region.runs[0].start..target_byte];
        let mut fs = crate::fonts::new_font_system();
        let target_x = region.text_origin.0
            + crate::ui::element::measure_text_width(
                &mut fs,
                prefix,
                region.font_size,
                region.font_kind,
                region.font_weight,
            );
        let y = region.text_origin.1 + region.line_height * 0.5;
        harness.drag((region.text_origin.0, y), &[(target_x, y)]);

        let selection = harness
            .state
            .github
            .pull_request
            .card_text_selection
            .get(&harness.state.store)
            .expect("card selection after interior drag");
        let selected = selection.selected_text().expect("non-empty selection");

        // Must land in the interior: past the first word (not the x<=0 guard) and
        // short of the whole line (not the saturation guard), near the target.
        assert!(
            selected.len() > first_space,
            "interior drag must select past the first word; got {selected:?}"
        );
        assert!(
            selected.len() < line0.len(),
            "interior drag must not select the whole line; got {selected:?}"
        );
        assert!(
            (selected.len() as i32 - target_byte as i32).abs() <= 4,
            "selection should end near the targeted word boundary (~{target_byte}); got {}",
            selected.len()
        );
        assert!(
            region.text.starts_with(&selected),
            "selection must be a prefix of the comment body"
        );
    }

    /// Inline styles render as distinct scene pieces; drag+Cmd+C copies marker-free text.
    #[test]
    fn rich_comment_body_renders_styled_pieces_and_copies_plain_text() {
        use crate::actions::AppAction;
        use crate::render::{FontKind, FontStyle, FontWeight, Primitive};

        let thread = sample_review_thread();
        let mut harness = UiHarness::new(&thread);
        let region = harness.region_for_comment(&thread, 1).clone();

        // The plain body (what selection/copy see) has the markup markers stripped.
        assert!(
            region.text.contains("memoize_height()"),
            "plain body should contain the code text: {:?}",
            region.text
        );
        assert!(
            !region.text.contains('`') && !region.text.contains('*'),
            "copy source must not contain markdown markers: {:?}",
            region.text
        );

        let scene = &harness.card.scene;
        let has_mono_code = scene.primitives.iter().any(|p| {
            matches!(p, Primitive::TextRun(t)
                if t.font_kind == FontKind::Mono && t.text.as_ref() == "memoize_height()")
        });
        assert!(has_mono_code, "inline code must render as a mono piece");

        let has_bold = scene.primitives.iter().any(|p| {
            matches!(p, Primitive::TextRun(t)
                if t.font_weight == FontWeight::Semibold && t.text.as_ref() == "stable")
        });
        assert!(has_bold, "**bold** must render as a semibold piece");

        let has_italic = scene.primitives.iter().any(|p| {
            matches!(p, Primitive::RichTextRun(r)
                if r.spans.iter().any(|s|
                    s.font_style == Some(FontStyle::Italic) && s.text.as_ref() == "pinned"))
        });
        assert!(has_italic, "*italic* must render as an italic span");

        // Drag the first visual line to the saturation edge; Cmd+C copies exactly
        // that line, markers-free, even though it mixes mono and UI glyphs.
        let first_line_end = region.runs[0].end;
        let expected = region.text[..first_line_end].to_owned();
        let y = region.text_origin.1 + region.line_height * 0.5;
        harness.drag(
            (region.text_origin.0, y),
            &[(region.bounds.x + region.bounds.width + 200.0, y)],
        );
        let actions = harness.key(UiHarness::cmd_key("c"));
        assert_eq!(
            actions,
            vec![AppAction::CopyText(expected.clone()).into()],
            "Cmd+C must copy the dragged plain-text line"
        );
        assert!(
            !expected.contains('`'),
            "copied text must not contain backticks: {expected:?}"
        );
    }

    /// No two text pieces on the same visual line overlap (computed gap, not eyeballed).
    #[test]
    fn text_pieces_do_not_overlap_on_a_line() {
        let thread = sample_review_thread();
        let card = render_review_card(&thread, true, None);
        let mut fs = crate::fonts::new_font_system();

        for line in text_lines(&card.scene) {
            for pair in line.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                let adv = crate::ui::element::measure_text_advance(
                    &mut fs,
                    &a.text,
                    a.font_size,
                    a.font_kind,
                    a.font_weight,
                );
                let gap = b.x - (a.x + adv);
                assert!(
                    gap >= -2.0,
                    "text pieces overlap on a line (gap {gap:.1}px): {:?} then {:?}\n{}",
                    a.text,
                    b.text,
                    dump_text_layout(&card.scene, &mut fs)
                );
            }
        }
    }

    /// Styled pieces keep a real space across the mono/bold/italic boundaries.
    #[test]
    fn rich_body_pieces_are_spaced_across_style_boundaries() {
        let thread = sample_review_thread();
        let card = render_review_card(&thread, true, None);
        let mut fs = crate::fonts::new_font_system();
        let pieces = text_pieces(&card.scene);

        // The code/bold/italic pieces and the normal text immediately after them.
        for (left, right) in [("memoize_height()", "over"), ("stable", "while")] {
            let li = pieces
                .iter()
                .position(|p| p.text.trim() == left)
                .unwrap_or_else(|| panic!("missing piece {left:?}"));
            let a = &pieces[li];
            let b = pieces[li + 1..]
                .iter()
                .find(|p| (p.y - a.y).abs() < a.height * 0.5)
                .unwrap_or_else(|| panic!("no piece after {left:?} on its line"));
            assert!(
                b.text.trim_start().starts_with(right),
                "expected {right:?} after {left:?}, got {:?}",
                b.text
            );
            let adv = crate::ui::element::measure_text_advance(
                &mut fs,
                &a.text,
                a.font_size,
                a.font_kind,
                a.font_weight,
            );
            let space = crate::ui::element::measure_text_advance(
                &mut fs,
                " ",
                a.font_size,
                crate::render::FontKind::Ui,
                crate::render::FontWeight::Normal,
            );
            let gap = b.x - (a.x + adv);
            assert!(
                gap >= space * 0.5,
                "{left:?}→{right:?} should have ~a space between them (gap {gap:.1}px, \
                 space {space:.1}px)\n{}",
                dump_text_layout(&card.scene, &mut fs)
            );
        }
    }

    /// (d) The dump is deterministic: two independent renders of the same fixture
    /// produce byte-identical accessibility snapshots.
    #[test]
    fn dump_is_stable_across_renders() {
        let thread = sample_review_thread();
        let first = dump_accessibility(&render_review_card(&thread, true, None).accessibility);
        let second = dump_accessibility(&render_review_card(&thread, true, None).accessibility);
        assert_eq!(first, second, "accessibility dump must be deterministic");
    }

    /// Reading order: a comment's author/header node precedes its body node, so a
    /// screen reader announces who is speaking before what they said.
    #[test]
    fn reading_order_places_header_before_body() {
        let thread = sample_review_thread();
        let rendered = render_review_card(&thread, true, None);
        let dump = dump_accessibility(&rendered.accessibility);

        let lines: Vec<&str> = dump.lines().collect();
        let first_author = lines
            .iter()
            .position(|l| l.contains("@alpha"))
            .expect("an author header node");
        let first_body = lines
            .iter()
            .position(|l| l.starts_with("selectable-text:"))
            .expect("a comment body node");
        assert!(
            first_author < first_body,
            "author header must come before the comment body:\n{dump}"
        );
    }

    /// End-to-end proof of the selectable comment-body feature with NO GPU:
    /// a drag across comment 0's first wrapped line produces a non-collapsed
    /// `CardTextSelection` whose substring is comment 0's first visual line, that
    /// drag clears any pre-existing viewport selection (mutual exclusivity), and
    /// Cmd+C copies exactly the dragged substring. A subsequent viewport-text
    /// selection clears the card selection (the reverse exclusivity direction).
    #[test]
    fn drag_selects_comment_text_and_cmd_c_copies_it() {
        use crate::actions::AppAction;
        use crate::ui::state::FocusTarget;

        let thread = sample_review_thread();
        let mut harness = UiHarness::new(&thread);

        // Pre-seed a viewport text selection; the card drag must clear it.
        harness.state.editor.text_selection.set(
            &harness.state.store,
            Some(crate::ui::editor::state::ViewportTextSelection {
                generation: 1,
                anchor: crate::ui::editor::state::ViewportTextPoint {
                    line_index: 0,
                    side: crate::ui::editor::state::ViewportTextSide::Right,
                    byte_offset: 0,
                },
                focus: crate::ui::editor::state::ViewportTextPoint {
                    line_index: 0,
                    side: crate::ui::editor::state::ViewportTextSide::Right,
                    byte_offset: 3,
                },
            }),
        );

        let region = harness.region_for_comment(&thread, 0).clone();
        assert!(
            region.runs.len() >= 2,
            "fixture comment 0 must wrap to >= 2 visual lines; got {}",
            region.runs.len()
        );
        let first_line_end = region.runs[0].end;
        let expected = region.text[..first_line_end].to_owned();

        // Mouse down at the text origin (x maps to byte 0), drag past the right
        // edge of the first visual line, release.
        let start = (
            region.text_origin.0,
            region.text_origin.1 + region.line_height * 0.5,
        );
        let line0_y = region.text_origin.1 + region.line_height * 0.5;
        harness.drag(
            start,
            &[
                (region.bounds.x + region.bounds.width * 0.5, line0_y),
                (region.bounds.x + region.bounds.width + 200.0, line0_y),
            ],
        );

        // Mutual exclusivity: the card drag cleared the viewport selection.
        assert!(
            harness
                .state
                .editor
                .text_selection
                .get(&harness.state.store)
                .is_none(),
            "card drag must clear the viewport text selection"
        );

        // The card selection is present, non-collapsed, and equals line 0's text.
        let selection = harness
            .state
            .github
            .pull_request
            .card_text_selection
            .get(&harness.state.store)
            .expect("card text selection after drag");
        assert!(
            !selection.is_collapsed(),
            "dragged selection must be non-collapsed"
        );
        let selected = selection.selected_text().expect("non-empty selected text");
        assert_eq!(
            selected, expected,
            "drag across the first visual line must select exactly that line"
        );
        assert!(
            region.text.starts_with(&selected),
            "selection must be a prefix of the comment body"
        );

        // The card mousedown emitted FocusViewport (applied by the harness), so
        // input owner resolves to Editor and Cmd+C takes the card-copy branch.
        assert_eq!(
            harness.state.focus.get(&harness.state.store),
            Some(FocusTarget::Editor),
            "card mousedown must focus the viewport/editor"
        );
        let actions = harness.key(UiHarness::cmd_key("c"));
        assert_eq!(
            actions,
            vec![AppAction::CopyText(selected.clone()).into()],
            "Cmd+C must copy exactly the dragged substring"
        );

        // Reverse exclusivity: starting a viewport text selection clears the card
        // selection.
        harness
            .state
            .apply_action(crate::actions::EditorAction::BeginViewportTextSelection {
                point: crate::ui::editor::state::ViewportTextPoint {
                    line_index: 0,
                    side: crate::ui::editor::state::ViewportTextSide::Right,
                    byte_offset: 0,
                },
                generation: 1,
            });
        assert!(
            harness
                .state
                .github
                .pull_request
                .card_text_selection
                .get(&harness.state.store)
                .is_none(),
            "beginning a viewport text selection must clear the card selection"
        );
    }
}
