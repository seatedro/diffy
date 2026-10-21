//! The diff view's elements: one cache boundary for the whole view, and
//! one per gutter cell, text cell, and header row inside it, so an
//! unchanged frame replays without building or allocating and a scrolled
//! frame builds only the rows entering the window.

use diffy_syntax::HighlightKind;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use accesskit::Role;
use diffy_diff::{FileStatus, GapId, Mode, Reveal, RowKind, Side, SlotState};
use quark::view;
use quark_render::scene::{Rect, RichTextPrimitive, ShapedText};
use quark_render::{FontKind, RectPrimitive, Scene};
use quark_text::{TextParams, TextStyle};
use quark_ui::Action;
use quark_ui::element::{
    AnyElement, Bounds, CacheKey, ClickEvent, CursorHint, DragHandler, DragReleaseResult, Element,
    ElementContext, GroupCondition, GroupId, IntoAnyElement, LayoutEngine, LayoutId,
    ScrollActionBuilder, cached, canvas, div, inputs_hash, svg_icon, text,
};
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::{Color, Theme};

use super::decorator::{
    AnnotationContext, DiffDecorator, DiffSources, DiffViewOptions, GutterContext, HeaderContext,
    HeaderSlot, HunkSource, LineContext, LineMark, LinePointer, SeparatorContext,
};
use super::interaction::AUTOSCROLL_FRAME_MS;
use super::paint::{self, Cue, LineColors};
use super::prepared::{
    ColumnBox, FileFact, FrameRow, LinePaint, Metrics, PreparedKind, RowPaint, SearchMark,
    ViewFrame,
};
use super::presentation::{
    DiffColors, DiffMarkers, DiffNumbers, DiffPresentation, EmptySideFill, FileHeaders,
    HunkSeparator, band_height,
};
use super::{DiffEvent, DiffKey, DiffViewState, REVEAL_STEP};
use quark_components::CollectionEnv;

pub(crate) const KEYS: &[(&str, DiffKey)] = &[
    ("n", DiffKey::NextHunk),
    ("p", DiffKey::PrevHunk),
    ("alt+arrowdown", DiffKey::NextHunk),
    ("alt+arrowup", DiffKey::PrevHunk),
    ("]", DiffKey::NextFile),
    ("[", DiffKey::PrevFile),
    ("arrowup", DiffKey::LineUp),
    ("arrowdown", DiffKey::LineDown),
    ("pageup", DiffKey::PageUp),
    ("pagedown", DiffKey::PageDown),
    ("home", DiffKey::Home),
    ("end", DiffKey::End),
    ("mod+c", DiffKey::Copy),
    ("mod+a", DiffKey::SelectAll),
];

/// What every cell build reads besides its row, copied into each cached
/// closure.
#[derive(Clone, Copy)]
struct Look {
    colors: DiffColors,
    /// Identifies `colors` in cache keys, so new local colors rebuild the
    /// cells they paint.
    colors_key: u64,
    presentation: DiffPresentation,
    env: CollectionEnv,
}

const CHANGE_BAR_W: f32 = 3.0;

/// The app's decorator and what identifies its output in cache keys.
#[derive(Clone)]
struct Deco {
    decorator: Option<Rc<dyn DiffDecorator>>,
    sources: Option<Rc<DiffSources>>,
    key: (usize, u64, usize),
}

impl Deco {
    fn new(decorator: Option<Rc<dyn DiffDecorator>>, sources: Option<Rc<DiffSources>>) -> Self {
        let (ptr, revision) = decorator.as_ref().map_or((0, 0), |d| {
            (Rc::as_ptr(d) as *const () as usize, d.revision())
        });
        let sources_ptr = sources.as_ref().map_or(0, |s| Rc::as_ptr(s) as usize);
        Self {
            decorator,
            sources,
            key: (ptr, revision, sources_ptr),
        }
    }

    fn file(&self, unit: u32) -> Option<super::decorator::FileSource> {
        self.sources.as_ref()?.file(unit)
    }

    fn mark(&self, paint: &RowPaint, kind: RowKind, side: Side) -> Option<LineMark> {
        let decorator = self.decorator.as_ref()?;
        let line = paint.source_lines[side as usize]?;
        decorator.line_mark(&LineContext {
            file: paint.file,
            source: self.file(paint.file),
            side,
            line,
            kind,
        })
    }
}

/// The diff in `state` at its viewport size, as of the last
/// [`DiffViewState::prepare`]. `on_event` wraps input into the app's
/// action type.
pub fn diff_view(
    state: &mut DiffViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    diff_view_with(state, theme, env, on_event, None)
}

/// [`diff_view`] with the app's `decorator` filling header slots,
/// annotation rows, and the focused row's gutter utility.
pub fn diff_view_with(
    state: &mut DiffViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
    decorator: Option<Rc<dyn DiffDecorator>>,
) -> AnyElement {
    diff_view_custom(
        state,
        theme,
        env,
        on_event,
        DiffViewOptions {
            decorator,
            ..DiffViewOptions::default()
        },
    )
}

pub fn diff_view_custom(
    state: &mut DiffViewState,
    theme: &Theme,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
    options: DiffViewOptions,
) -> AnyElement {
    let (width, height) = state.viewport;
    let Some(frame) = state.frame.clone() else {
        return view! { <div w={width} h={height} /> };
    };
    let colors = DiffColors::resolve(theme, &frame.appearance);
    let look = Look {
        colors,
        colors_key: colors.key(),
        presentation: frame.presentation,
        env,
    };
    let deco = Deco::new(options.decorator, options.sources);
    let keys = options.keys.unwrap_or(KEYS);
    let on_line = options.on_line;
    // The cache watches the sideways scroll handles itself.
    let hash = inputs_hash(&(
        state.frame_id,
        env,
        on_event as usize,
        on_line.map(|f| f as usize),
        look.colors_key,
        deco.key,
        keys.as_ptr() as usize,
        keys.len(),
    ));
    BoundsProbe {
        child: view! {
            <cached(state.id, hash, move || build(&frame, look, &deco, keys, on_event, on_line))
                w={width}
                h={height}
            />
        },
        bounds: state.bounds.clone(),
        autoscrolling: state.wants_frame(),
    }
    .into_any()
}

fn build(
    frame: &Rc<ViewFrame>,
    look: Look,
    deco: &Deco,
    keys: &'static [(&'static str, DiffKey)],
    on_event: fn(DiffEvent) -> Action,
    on_line: Option<fn(LinePointer) -> Action>,
) -> AnyElement {
    let (width, height) = frame.viewport;
    let colors = look.colors;
    let env = look.env;
    let first_top = frame.rows.first().map_or(0.0, |r| r.top);
    let columns = frame
        .columns
        .sides
        .iter()
        .enumerate()
        .filter_map(|(slot, column)| {
            let column = column.as_ref()?;
            let side = if slot == 0 { Side::Old } else { Side::New };
            // Unwrapped columns scroll sideways over content laid out from
            // an origin near the scroll: what lies before it is out of view.
            let (content_w, origin) = if frame.wrap {
                (f64::from(column.text_w), 0.0)
            } else {
                let content_w = frame.content_w[slot].max(f64::from(column.text_w));
                (content_w, frame.content_origin[slot].min(content_w))
            };
            let laid_w = (content_w - origin) as f32;
            Some((slot, column, side, content_w, origin, laid_w))
        });
    let split = frame.columns.mode == Mode::Split;
    let sticky = frame.sticky_header.as_ref().map(|row| row.key);
    let picks = [Side::Old, Side::New].map(|side| picked_rows(frame, side, deco));
    // Split rows with no line on a side: one fill across that side's
    // gutter and text, outside the sideways scroll.
    let empty_sides = frame
        .rows
        .iter()
        .filter(move |r| split && r.paint.kind.is_line())
        .flat_map(|r| {
            [Side::Old, Side::New]
                .into_iter()
                .filter(move |&side| r.paint.source_lines[side as usize].is_none())
                .map(move |side| (r, side))
        });
    let divider = split.then(|| {
        let old = frame.columns.of(Side::Old);
        (
            old.text_x + old.text_w,
            frame.columns.of(Side::New).gutter_x - (old.text_x + old.text_w),
        )
    });
    view! {
        <div
            w={width}
            h={height}
            bg={colors.surface}
            track_focus={frame.focus}
            // Its children are laid out from the scroll offset, so it
            // only feeds the wheel and the scrollbar.
            scroll_y={frame.scroll}
            scroll_total={frame.total}
            scroll_origin={(0.0, frame.scroll)}
            on:scroll={ScrollActionBuilder::new(move |lines| on_event(DiffEvent::Scroll(lines)))
                .with_to_px(move |px| on_event(DiffEvent::ScrollTo(px as f32)))}
            @when {frame.scrollbar_auto_hide} {
                scrollbar_visibility={&frame.scrollbar}
                class="scrollbar-auto-hide"
            }
            @for &(binding, key) in keys { on_key={(binding, on_event(DiffEvent::Key(key)))} }
            @when {env.accessible} {
                accessibility_id={frame.id}
                accessibility_role={Role::List}
                aria-label={frame.label}
            }
        >
            <div
                w={width}
                h={height}
                class="relative"
                @when {frame.link} { class="cursor-pointer" }
                @when {!frame.link} { class="cursor-text" }
                on:drag={move |press: ClickEvent| {
                    Box::new(SelectDrag { press, on_event }) as Box<dyn DragHandler>
                }}
            >
                for (slot, column, side, content_w, origin, laid_w) in columns {
                    <div
                        class="absolute"
                        left={column.gutter_x}
                        class="top-0"
                        w={column.gutter_w}
                        h={height}
                        class="overflow-clip"
                        bg={colors.gutter}
                        @when {let Some(on_line) = on_line} {
                            class="cursor-pointer"
                            on:drag={move |press: ClickEvent| {
                                Box::new(LineDrag { press, on_line }) as Box<dyn DragHandler>
                            }}
                        }
                    >
                        <div w={column.gutter_w} class="flex-col" translate={(0.0, first_top)}>
                            for (i, row) in frame.rows.iter().enumerate() {
                                {gutter_cell(frame, row, side, column.gutter_w, look, picks[slot][i])}
                            }
                        </div>
                    </div>
                    <div
                        class="absolute"
                        left={column.text_x}
                        class="top-0"
                        w={column.text_w}
                        h={height}
                        @when {frame.wrap} { class="overflow-clip" }
                        @when {!frame.wrap} {
                            track_scroll={&frame.hscroll[slot]}
                            class="overflow-x-scroll"
                            scroll_total_x={content_w}
                            scroll_origin={(origin, 0.0)}
                            class="scrollbar-auto-hide"
                        }
                    >
                        <div w={laid_w} class="flex-col" translate={(0.0, first_top)}>
                            for (i, row) in frame.rows.iter().enumerate() {
                                {text_cell(frame, row, side, laid_w, origin, look, picks[slot][i], on_event)}
                            }
                        </div>
                    </div>
                }
                for (row, side) in empty_sides {
                    {empty_side(frame, row, side, look)}
                }
                if let Some((x, w)) = divider {
                    <div class="absolute top-0" left={x} w={w} h={height}>
                        <canvas(
                            move |bounds, scene, cx| {
                                paint::vertical_hairline(
                                    scene,
                                    bounds.x,
                                    bounds.y,
                                    bounds.height,
                                    colors.border,
                                    cx.scale_factor,
                                );
                            }
                        )
                            w={w}
                            h={height}
                        />
                    </div>
                }
                for row in frame
                    .rows
                    .iter()
                    .filter(|r| !r.paint.kind.is_line() && r.height > 0.0 && Some(r.key) != sticky) {
                    <div
                        class="absolute left-0"
                        top={row.top}
                        w={width}
                        h={row.height}
                        @when {let Some(id) = band_id(frame, row, deco)} { test_id={id} }
                    >
                        {band(frame, row, look, deco, on_event)}
                    </div>
                }
                for row in frame.rows.iter().filter(|r| !r.focused && r.height > 0.0 && r.paint.kind.is_line()) {
                    if let Some(utility) = hover_utility(frame, row, deco) {
                        {utility}
                    }
                }
                for (i, row) in frame.rows.iter().enumerate().filter(|(_, r)| r.focused && r.height > 0.0) {
                    {focus_overlay(frame, row, look, deco, picks.iter().any(|p| p[i].on))}
                }
                if let Some(row) = &frame.sticky_header {
                    <div
                        class="absolute left-0 top-0"
                        w={width}
                        h={row.height}
                        @when {let Some(id) = band_id(frame, row, deco)} { test_id={id} }
                    >
                        {band(frame, row, look, deco, on_event)}
                    </div>
                }
            </div>
        </div>
    }
}

fn band_id(frame: &ViewFrame, row: &FrameRow, deco: &Deco) -> Option<String> {
    let paint = &row.paint;
    let sources = deco.sources.as_ref()?;
    let source = sources.file(paint.file)?;
    let (id, file, revision) = (frame.id, source.id.0, source.revision.0);
    let hunk = |global: u32| sources.hunk(paint.file, global).map(|h| h.hunk);
    match paint.kind.diff()? {
        RowKind::FileHeader => Some(format!("{id}.file.{file}.{revision}")),
        RowKind::HunkHeader => {
            let local = hunk((row.key & 0x3F_FFFF_FFFF) as u32)?;
            Some(format!("{id}.hunk.{file}.{revision}.{local}"))
        }
        RowKind::Gap => match paint.gap.and_then(|g| g.hunk) {
            Some(global) => Some(format!("{id}.gap.{file}.{revision}.{}", hunk(global)?)),
            None => Some(format!("{id}.gap.{file}.{revision}.end")),
        },
        _ => None,
    }
}

/// Cache key of one part of a row.
fn part_key(row: u64, part: u64) -> CacheKey {
    CacheKey(row ^ (part + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// The line `side`'s column shows for `row`: unified shows removed lines
/// from the old side and everything else from the new.
fn shown_side(mode: Mode, kind: RowKind, side: Side) -> Side {
    match mode {
        Mode::Split => side,
        Mode::Unified if kind == RowKind::Removed => Side::Old,
        Mode::Unified => Side::New,
    }
}

/// The change a line of `kind` on `side` shows, if any.
fn cue(kind: RowKind, side: Side) -> Option<Cue> {
    match (kind, side) {
        (RowKind::Removed, _) | (RowKind::Modified, Side::Old) => Some(Cue::Removed),
        (RowKind::Added | RowKind::Merged, _) | (RowKind::Modified, Side::New) => Some(Cue::Added),
        (RowKind::Moved, _) => Some(Cue::Moved),
        _ => None,
    }
}

/// The numbers a gutter shows, left to right, and how many columns.
fn gutter_numbers(
    numbers: DiffNumbers,
    mode: Mode,
    all: [u32; 2],
    side: Side,
    shown: Side,
) -> ([u32; 2], usize) {
    match (numbers, mode) {
        (DiffNumbers::None, _) => ([0; 2], 0),
        (DiffNumbers::Both, Mode::Unified) => (all, 2),
        (DiffNumbers::RelevantSide, Mode::Unified) => ([all[shown as usize], 0], 1),
        (_, Mode::Split) => ([all[side as usize], 0], 1),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
struct Pick {
    on: bool,
    top: bool,
    bottom: bool,
}

fn picked_rows(frame: &ViewFrame, side: Side, deco: &Deco) -> Vec<Pick> {
    let mode = frame.columns.mode;
    let on: Vec<bool> = frame
        .rows
        .iter()
        .map(|row| {
            row.paint
                .kind
                .diff()
                .filter(|k| k.is_line())
                .and_then(|kind| deco.mark(&row.paint, kind, shown_side(mode, kind, side)))
                .is_some()
        })
        .collect();
    (0..on.len())
        .map(|i| Pick {
            on: on[i],
            top: on[i] && (i == 0 || !on[i - 1]),
            bottom: on[i] && on.get(i + 1).is_none_or(|next| !next),
        })
        .collect()
}

fn pick_rules(scene: &mut Scene, bounds: Bounds, pick: Pick, color: Color, scale: f32) {
    let rule = paint::whole_pixels(1.0, scale);
    let mut draw = |y: f32| {
        scene.rect(RectPrimitive {
            rect: Rect {
                x: bounds.x,
                y,
                width: bounds.width,
                height: rule,
            },
            color,
        })
    };
    if pick.top {
        draw(bounds.y);
    }
    if pick.bottom {
        draw(bounds.y + bounds.height - rule);
    }
}

fn gutter_cell(
    frame: &ViewFrame,
    row: &FrameRow,
    side: Side,
    width: f32,
    look: Look,
    pick: Pick,
) -> AnyElement {
    let height = row.height;
    let Some(kind) = row.paint.kind.diff().filter(|k| k.is_line()) else {
        return view! { <div w={width} h={height} class="shrink-0" /> };
    };
    let mode = frame.columns.mode;
    if mode == Mode::Split && row.paint.source_lines[side as usize].is_none() {
        // The empty-side fill covers it.
        return view! { <div w={width} h={height} class="shrink-0" /> };
    }
    let shown = shown_side(mode, kind, side);
    let presentation = look.presentation;
    let (numbers, count) =
        gutter_numbers(presentation.numbers, mode, row.paint.numbers(), side, shown);
    let m = frame.metrics;
    let colors = look.colors;
    let hash = inputs_hash(&(
        row.paint.stamp,
        numbers,
        count,
        mode,
        height.to_bits(),
        width.to_bits(),
        look.colors_key,
        presentation.markers,
        pick,
    ));
    let quiet = row.paint.quiet;
    let build = move || {
        let bg = if pick.on {
            Some(colors.picked_gutter)
        } else if kind == RowKind::Moved {
            Some(colors.moved_gutter)
        } else if quiet {
            None
        } else {
            colors.line(kind, shown)
        };
        view! {
            <div
                w={width}
                h={height}
                class="shrink-0"
                @when {let Some(bg) = bg} { bg={bg} }
            >
                <canvas(
                    move |bounds, scene, cx| {
                        paint_gutter(
                            bounds,
                            scene,
                            cx,
                            &m,
                            if quiet {
                                DiffMarkers::None
                            } else {
                                presentation.markers
                            },
                            &numbers[..count],
                            cue(kind, shown).filter(|_| {
                                !quiet || matches!(kind, RowKind::Removed | RowKind::Moved)
                            }),
                            pick.on,
                            colors,
                        );
                        pick_rules(scene, bounds, pick, colors.accent, cx.scale_factor);
                    }
                )
                    w={width}
                    h={height}
                />
            </div>
        }
    };
    view! { <cached(part_key(row.key, side as u64), hash, build) w={width} h={height} /> }
}

/// A line's gutter: its bar at the leading edge, its number columns, and
/// its sign at the trailing edge, as the markers choose.
#[allow(clippy::too_many_arguments)]
fn paint_gutter(
    bounds: Bounds,
    scene: &mut Scene,
    cx: &mut ElementContext,
    m: &Metrics,
    markers: DiffMarkers,
    numbers: &[u32],
    cue: Option<Cue>,
    picked: bool,
    colors: DiffColors,
) {
    let style = TextStyle::new(m.font_size)
        .kind(FontKind::Mono)
        .line_height(m.line_h);
    let mut x = bounds.x;
    if markers == DiffMarkers::Bars {
        let color = match (picked, cue) {
            (true, _) => Some(colors.accent),
            (false, Some(Cue::Added)) => Some(colors.add_marker),
            (false, Some(Cue::Removed)) => Some(colors.del_marker),
            (false, Some(Cue::Moved)) => Some(colors.tone(HighlightKind::Keyword)),
            (false, None) => None,
        };
        if let Some(color) = color {
            let w = paint::whole_pixels(CHANGE_BAR_W, cx.scale_factor);
            scene.rect(RectPrimitive {
                rect: Rect {
                    x: bounds.x + bounds.width - w,
                    y: bounds.y,
                    width: w,
                    height: bounds.height,
                },
                color,
            });
        }
    }
    let number_color = match cue {
        _ if picked => colors.picked_number,
        Some(Cue::Added) => colors.add_number,
        Some(Cue::Removed) => colors.del_number,
        Some(Cue::Moved) => colors.tone(HighlightKind::Keyword),
        None => colors.gutter_text,
    };
    for &number in numbers {
        if number != 0
            && let Some(layout) = cx.layout_text(&TextParams::new(number.to_string(), style))
        {
            let w = layout.size().0;
            let right = x + m.number_w - m.char_w;
            scene.rich_text(RichTextPrimitive {
                rect: Rect {
                    x: right - w,
                    y: bounds.y,
                    width: w + 1.0,
                    height: m.line_h,
                },
                layout: ShapedText::new(layout),
                default_color: number_color,
                span_colors: Arc::from([]),
            });
        }
        x += m.number_w;
    }
    let sign = match cue {
        Some(Cue::Removed) => Some(("-", colors.del_marker)),
        Some(Cue::Added) => Some(("+", colors.add_marker)),
        Some(Cue::Moved) | None => None,
    };
    if markers == DiffMarkers::Signs
        && let Some((sign, color)) = sign
        && let Some(layout) = cx.layout_text(&TextParams::new(sign, style))
    {
        let x = bounds.x + bounds.width - m.sign_w + (m.sign_w - layout.size().0) / 2.0;
        scene.rich_text(RichTextPrimitive {
            rect: Rect {
                x,
                y: bounds.y,
                width: m.sign_w,
                height: m.line_h,
            },
            layout: ShapedText::new(layout),
            default_color: color,
            span_colors: Arc::from([]),
        });
    }
}

/// A line's text in a column `width` wide, laid out from content x
/// `origin`.
#[allow(clippy::too_many_arguments)]
fn text_cell(
    frame: &ViewFrame,
    row: &FrameRow,
    side: Side,
    width: f32,
    origin: f64,
    look: Look,
    pick: Pick,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let height = row.height;
    let mode = frame.columns.mode;
    let Some(kind) = row.paint.kind.diff().filter(|k| k.is_line()) else {
        return view! { <div w={width} h={height} class="shrink-0" /> };
    };
    let shown = shown_side(mode, kind, side);
    if row.paint.sides[shown as usize].is_none() {
        // No line here (split's empty-side fill covers it), or one that
        // could not be shaped.
        return view! { <div w={width} h={height} class="shrink-0" /> };
    }
    let paint = row.paint.clone();
    let selected = row.selected[shown as usize];
    let link = row.link[shown as usize];
    let search = row.search[shown as usize].clone();
    let pad = frame.metrics.text_pad;
    let line_h = frame.metrics.line_h;
    let font_size = frame.metrics.font_size;
    let position = (row.index + 1, frame.row_count);
    let colors = look.colors;
    let env = look.env;
    let hash = inputs_hash(&(
        paint.stamp,
        selected,
        link,
        &search,
        width.to_bits(),
        origin.to_bits(),
        height.to_bits(),
        env.accessible,
        mode,
        position,
        look.colors_key,
        pick,
    ));
    let build = move || {
        let line = paint.sides[shown as usize].as_ref().expect("shown line");
        let canvas_paint = paint.clone();
        let fold = paint.fold.zip(paint.source_lines[Side::New as usize]).map(
            |((count, open), line_index)| {
                let (end, top) = line
                    .layout
                    .lines()
                    .last()
                    .map_or((line.layout.size().0, 0.0), |last| (last.width, last.top));
                let x = pad + end + FOLD_GAP - origin as f32;
                let toggle = DiffEvent::ToggleReflow {
                    file: paint.file,
                    line: line_index,
                };
                (x, top, count, open, on_event(toggle))
            },
        );
        let whole = line.whole.then_some(match shown {
            Side::Old => RowKind::Removed,
            Side::New => RowKind::Added,
        });
        let line_bg = if kind == RowKind::Moved {
            Some(colors.moved_wash)
        } else if paint.quiet {
            None
        } else {
            colors.line(whole.unwrap_or(kind), shown)
        };
        let bg = match (pick.on, line_bg) {
            (true, Some(bg)) => Some(DiffColors::tint(bg, colors.accent)),
            (true, None) => Some(colors.picked_wash),
            (false, bg) => bg,
        };
        view! {
            <div
                w={width}
                h={height}
                class="shrink-0 relative"
                @when {let Some(bg) = bg} { bg={bg} }
                @when {let Some(hint) = paint.hint.clone()} { tooltip={hint.to_string()} }
                @when {env.accessible} {
                    accessibility_role={Role::ListItem}
                    aria-label={line.label()}
                    aria-description={kind.description()}
                    aria-valuetext={match (mode, paint.numbers()) {
                        (Mode::Unified, [o, n]) if o != 0 && n != 0 => {
                            format!("old line {o}, new line {n}")
                        }
                        (_, [o, _]) if shown == Side::Old => format!("old line {o}"),
                        (_, [_, n]) => format!("new line {n}"),
                    }}
                }
            >
                if let Some((x, top, count, open, toggle)) = fold.clone() {
                    <div
                        class="absolute flex-row items-center"
                        left={x}
                        top={top}
                        h={line_h}
                    >
                        <div
                            h={(line_h - 4.0).max(12.0)}
                            px={4.0}
                            rounded={4.0}
                            class="flex-row items-center cursor-pointer"
                            hover_bg={colors.hover}
                            on:click={toggle}
                            @when {env.accessible} {
                                accessibility_role={Role::Button}
                                aria-label={if open {
                                    "Fold reformatted lines".to_owned()
                                } else {
                                    format!("Show {count} reformatted lines")
                                }}
                            }
                        >
                            <icon
                                svg={if open { lucide::CHEVRON_UP } else { lucide::MORE_HORIZONTAL }}
                                size={(line_h * 0.6).round()}
                                color={colors.muted}
                            />
                        </div>
                    </div>
                }
                <canvas(
                    move |bounds, scene, cx| {
                        if let Some(line) = &canvas_paint.sides[shown as usize] {
                            paint_text(
                                bounds,
                                scene,
                                line,
                                shown,
                                bg,
                                match kind {
                                    RowKind::Reflowed => TextTone::Dim,
                                    RowKind::Removed if canvas_paint.quiet => TextTone::Removed,
                                    _ => TextTone::Normal,
                                },
                                &search,
                                selected,
                                link,
                                pad,
                                origin,
                                font_size,
                                colors,
                                cx.scale_factor,
                            );
                        }
                        pick_rules(scene, bounds, pick, colors.accent, cx.scale_factor);
                    }
                )
                    w={width}
                    h={height}
                />
            </div>
        }
    };
    view! { <cached(part_key(row.key, 2 + side as u64), hash, build) w={width} h={height} /> }
}

const FOLD_GAP: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextTone {
    Normal,
    Dim,
    Removed,
}

#[allow(clippy::too_many_arguments)]
fn paint_text(
    bounds: Bounds,
    scene: &mut Scene,
    line: &LinePaint,
    side: Side,
    bg: Option<Color>,
    tone: TextTone,
    search: &[SearchMark],
    selected: Option<(usize, usize)>,
    link: Option<(usize, usize)>,
    pad: f32,
    origin: f64,
    font_size: f32,
    colors: DiffColors,
    scale: f32,
) {
    let word = if side == Side::Old {
        colors.del_word
    } else {
        colors.add_word
    };
    // A long line's layout holds only its window, which starts this far in:
    // from the origin the cell is laid out at, in f64 until it is small.
    let window_x = (line.window.map_or(0.0, |w| w.x) - origin) as f32;
    let removed = |i: usize| line.removed_spans.get(i).copied().unwrap_or(false);
    paint::line(
        scene,
        (bounds.x + pad + window_x, bounds.y),
        &line.layout,
        {
            let tones = colors.tones_on(bg);
            line.tones
                .iter()
                .enumerate()
                .map(|(i, &k)| match tone {
                    _ if removed(i) => colors.del_text,
                    TextTone::Removed | TextTone::Dim => colors.muted,
                    TextTone::Normal => tones[k as usize],
                })
                .collect()
        },
        &line.words,
        &line.struck,
        &line.renamed,
        search,
        selected,
        link,
        LineColors {
            text: match tone {
                TextTone::Normal => colors.text,
                TextTone::Dim | TextTone::Removed => colors.muted,
            },
            word,
            strike: colors.del_marker,
            rename: colors.accent,
            search: colors.search_match,
            search_active: colors.search_active,
            search_outline: colors.search_outline,
            selection: colors.selection,
            link: colors.accent,
        },
        bounds.height,
        font_size,
        scale,
    );
}

/// The fill of a split row's side that has no line: across its gutter and
/// text, solid or hatched.
fn empty_side(frame: &ViewFrame, row: &FrameRow, side: Side, look: Look) -> AnyElement {
    let column = frame.columns.of(side);
    let (x, width, height) = (column.gutter_x, column.gutter_w + column.text_w, row.height);
    let fill = look.presentation.empty_side;
    // The row's top in the document, so hatches of stacked rows join.
    // In content coordinates, so the hatch scrolls with the row.
    let phase =
        (f64::from(row.top) + frame.scroll).rem_euclid(f64::from(paint::HATCH_SPACING)) as f32;
    let colors = look.colors;
    let hash = inputs_hash(&(
        fill,
        width.to_bits(),
        height.to_bits(),
        phase.to_bits(),
        look.colors_key,
    ));
    let build = move || {
        view! {
            <canvas(
                move |bounds, scene, cx| {
                    scene.rect(RectPrimitive {
                        rect: Rect {
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height
                        },
                        color: colors.empty_side,
                    });
                    if fill == EmptySideFill::Hatch {
                        let rect = Rect {
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height
                        };
                        paint::hatch(scene, rect, phase, colors.hatch, cx.scale_factor);
                    }
                }
            )
                w={width}
                h={height}
            />
        }
    };
    view! {
        <div class="absolute" left={x} top={row.top} w={width} h={height}>
            <cached(part_key(row.key, 6 + side as u64), hash, build) w={width} h={height} />
        </div>
    }
}

fn utility(frame: &ViewFrame, row: &FrameRow, deco: &Deco) -> Option<(ColumnBox, f32, AnyElement)> {
    let kind = row.paint.kind.diff().filter(|k| k.is_line())?;
    let decorator = deco.decorator.as_ref()?;
    let mode = frame.columns.mode;
    let side = match mode {
        Mode::Unified => shown_side(mode, kind, Side::New),
        // The new side's column, unless the row has only an old line.
        Mode::Split if row.paint.source_lines[1].is_some() => Side::New,
        Mode::Split => Side::Old,
    };
    let line = row.paint.source_lines[side as usize]?;
    let column = frame.columns.of(side);
    let size = row.height.min(frame.metrics.line_h);
    let cx = GutterContext {
        file: row.paint.file,
        source: deco.file(row.paint.file),
        side,
        line,
        kind,
        width: size,
        height: size,
    };
    Some((column, size, decorator.gutter_utility(&cx)?))
}

fn hover_utility(frame: &ViewFrame, row: &FrameRow, deco: &Deco) -> Option<AnyElement> {
    let group = GroupId::new("diff.line");
    let (column, size, element) = utility(frame, row, deco)?;
    Some(view! {
        <div
            class="absolute"
            left={column.text_x}
            top={row.top}
            w={column.text_w}
            h={row.height}
            interaction_group={group}
        >
            <div
                class="absolute top-0 left-0"
                w={size}
                h={size}
                show_when={GroupCondition::HoveredOrFocusWithin(group)}
            >
                {element}
            </div>
        </div>
    })
}

/// The keyboard focus outline over a row, painted after everything else
/// in it, with the decorator's gutter utility for a line row.
/// The keyboard focus outline over a row, painted after everything else
/// in it, with the decorator's gutter utility for a line row.
fn focus_overlay(
    frame: &ViewFrame,
    row: &FrameRow,
    look: Look,
    deco: &Deco,
    picked: bool,
) -> AnyElement {
    let (width, height) = (frame.viewport.0, row.height);
    let color = look.colors.focused_row;
    let utility = utility(frame, row, deco);
    let band = matches!(
        row.paint.kind.diff(),
        Some(RowKind::FileHeader | RowKind::MovedHere | RowKind::MovedAway)
    );
    let outline = look.env.focused && !band && !picked;
    view! {
        <div class="absolute left-0" top={row.top} w={width} h={height}>
            if outline {
                <canvas(
                    move |bounds, scene, cx| {
                        let rect = Rect {
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height
                        };
                        paint::focus_outline(scene, rect, color, cx.scale_factor);
                    }
                )
                    w={width}
                    h={height}
                />
            }
            if let Some((column, size, element)) = utility {
                <div class="absolute top-0" left={column.text_x} w={size} h={size}>
                    {element}
                </div>
            }
        </div>
    }
}

/// A row across the whole view: a file header, a hunk header or gap, a
/// file fact, an annotation, or a preview's remainder.
fn band(
    frame: &ViewFrame,
    row: &FrameRow,
    look: Look,
    deco: &Deco,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let (width, height) = (frame.viewport.0, row.height);
    let paint = row.paint.clone();
    let font_size = frame.metrics.font_size;
    let pad = frame.metrics.char_w * 2.0;
    let id = frame.id;
    let columns = frame.columns;
    let deco = deco.clone();
    let key = row.key;
    let focused = row.focused;
    let hash = inputs_hash(&(
        paint.stamp,
        focused,
        width.to_bits(),
        height.to_bits(),
        look.env,
        on_event as usize,
        look.colors_key,
        look.presentation,
        deco.key,
        &paint.kind,
    ));
    let header_h = band_height(RowKind::FileHeader, &frame.metrics, &look.presentation);
    let build = move || match &paint.kind {
        PreparedKind::Diff(RowKind::FileHeader) => file_header(
            &paint, id, width, height, pad, font_size, look, &deco, on_event,
        ),
        PreparedKind::Diff(RowKind::MovedHere | RowKind::MovedAway) => {
            let indent = columns.sides[Side::New as usize].map_or(pad, |c| c.text_x);
            move_note(
                &paint, width, height, indent, font_size, focused, look, on_event,
            )
        }
        PreparedKind::Pending { state, .. } => {
            let header_h = header_h.min(height);
            let header = file_header(
                &paint, id, width, header_h, pad, font_size, look, &deco, on_event,
            );
            pending_band(
                header,
                *state,
                width,
                height - header_h,
                pad,
                font_size,
                look,
            )
        }
        PreparedKind::Diff(_) => {
            let hunk = match paint.kind.diff() {
                Some(RowKind::Gap) => paint.gap.and_then(|g| g.hunk),
                _ => Some((key & 0x3F_FFFF_FFFF) as u32),
            };
            let sources = deco.sources.as_ref();
            let cx = SeparatorContext {
                file: paint.file,
                gap: paint.gap,
                source: deco.file(paint.file),
                hunk: hunk.and_then(|h| sources?.hunk(paint.file, h)),
                hidden: paint.hidden,
                title: &paint.title,
                columns,
                width,
                height,
            };
            let decorator = deco.decorator.as_ref();
            match decorator.and_then(|d| d.separator(&cx)) {
                Some(custom) => view! { <div w={width} h={height}>{custom}</div> },
                None => {
                    let scope = decorator.and_then(|d| d.hunk_scope(&cx));
                    let actions = decorator.and_then(|d| d.hunk_actions(&cx));
                    separator(&paint, &cx, scope, actions, pad, font_size, look, on_event)
                }
            }
        }
        PreparedKind::Fact(fact) => {
            let label = fact_label(fact);
            let colors = look.colors;
            view! {
                <div
                    w={width}
                    h={height}
                    class="flex-row items-center"
                    px={pad}
                    bg={colors.separator}
                >
                    <text size={font_size * 0.9} color={colors.muted}>{label}</text>
                </div>
            }
        }
        PreparedKind::Annotation(slot) => {
            let content = deco.decorator.as_ref().and_then(|d| {
                d.annotation(&AnnotationContext {
                    id: slot.id,
                    side: slot.side,
                    lines: &slot.lines,
                    outdated: slot.outdated,
                    width,
                })
            });
            view! {
                <div w={width} h={height} class="overflow-clip">
                    if let Some(content) = content {
                        {content}
                    }
                </div>
            }
        }
        PreparedKind::More { hidden_rows } => {
            let colors = look.colors;
            let env = look.env;
            let rows = if *hidden_rows == 1 { "row" } else { "rows" };
            let label = format!("{hidden_rows} more {rows}");
            view! {
                <div
                    w={width}
                    h={height}
                    class="flex-row items-center"
                    px={pad}
                    bg={colors.separator}
                    @when {env.accessible} {
                        accessibility_role={Role::Status}
                        aria-label={label.clone()}
                    }
                >
                    <text size={font_size * 0.9} color={colors.muted}>{label}</text>
                    <div class="flex-1" />
                    <div
                        class="flex-row items-center h-full cursor-pointer"
                        px={pad * 0.5}
                        hover_bg={colors.hover}
                        on:click={on_event(DiffEvent::OpenFull)}
                        @when {env.accessible} {
                            accessibility_role={Role::Button}
                            aria-label="Open full diff"
                        }
                    >
                        <text size={font_size * 0.9} color={colors.text}>"Open full diff"</text>
                    </div>
                </div>
            }
        }
    };
    view! { <cached(part_key(row.key, 4), hash, build) w={width} h={height} /> }
}

#[allow(clippy::too_many_arguments)]
fn pending_band(
    header: AnyElement,
    state: SlotState,
    width: f32,
    height: f32,
    pad: f32,
    font_size: f32,
    look: Look,
) -> AnyElement {
    let colors = look.colors;
    let env = look.env;
    let (label, tone) = match state {
        SlotState::Unloaded => ("Not loaded yet", colors.muted),
        SlotState::Loading => ("Loading\u{2026}", colors.muted),
        SlotState::Failed => ("Couldn\u{2019}t load this file", colors.del_text),
        SlotState::Evicted => (
            "Unloaded to save memory; scroll here to reload",
            colors.muted,
        ),
    };
    view! {
        <div w={width} class="flex-col">
            {header}
            <div
                w={width}
                h={height.max(0.0)}
                class="flex-row items-start justify-center"
                pt={pad}
                bg={colors.surface}
                @when {env.accessible} {
                    accessibility_role={Role::Status}
                    aria-label={label}
                }
            >
                <text size={font_size * 0.9} color={tone}>{label}</text>
            </div>
        </div>
    }
}

fn fact_label(fact: &FileFact) -> String {
    match fact {
        FileFact::Binary => "Binary file not shown".to_owned(),
        FileFact::ModeChange { old, new } => format!("File mode changed from {old} to {new}"),
        FileFact::RenameOnly => "Renamed without changes".to_owned(),
        FileFact::CopyOnly => "Copied without changes".to_owned(),
        FileFact::NoNewlineAtEof(Side::Old) => "No newline at end of the old file".to_owned(),
        FileFact::NoNewlineAtEof(Side::New) => "No newline at end of the new file".to_owned(),
    }
}

fn status_label(status: FileStatus) -> String {
    let name = status.name();
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn status_tone(status: FileStatus, colors: DiffColors) -> Color {
    match status {
        FileStatus::Added => colors.add_text,
        FileStatus::Deleted => colors.del_text,
        FileStatus::Modified => colors.warning,
        FileStatus::Renamed | FileStatus::Copied => colors.muted,
    }
}

fn path_segments(title: &str) -> Option<Vec<String>> {
    (!title.contains('\u{2192}')).then(|| title.split('/').map(str::to_owned).collect())
}

#[allow(clippy::too_many_arguments)]
fn file_header(
    paint: &RowPaint,
    id: &'static str,
    width: f32,
    height: f32,
    pad: f32,
    font_size: f32,
    look: Look,
    deco: &Deco,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let (adds, dels) = paint.stats;
    let cx = HeaderContext {
        file: paint.file,
        source: deco.file(paint.file),
        title: &paint.title,
        status: paint.status,
        additions: adds,
        deletions: dels,
        binary: paint.binary,
        width,
        height,
    };
    let decorator = deco.decorator.as_ref();
    if look.presentation.headers == FileHeaders::Custom
        && let Some(custom) = decorator.and_then(|d| d.header(&cx))
    {
        return view! { <div w={width} h={height}>{custom}</div> };
    }
    let slot = |slot| decorator.and_then(|d| d.header_slot(slot, &cx));
    let (prefix, metadata, actions, trailing) = (
        slot(HeaderSlot::Prefix),
        slot(HeaderSlot::Metadata),
        slot(HeaderSlot::Actions),
        slot(HeaderSlot::Trailing),
    );
    let collapsed_note = paint
        .collapsed
        .then(|| slot(HeaderSlot::Collapsed))
        .flatten();
    let status = paint.status.name();
    let state = decorator
        .and_then(|d| d.file_state(&cx))
        .unwrap_or_else(|| status_label(paint.status));
    let colors = look.colors;
    let env = look.env;
    let tone = status_tone(paint.status, colors);
    let small = font_size * 0.85;
    let open = !paint.collapsed;
    let toggle = on_event(DiffEvent::ToggleFile { file: paint.file });
    let chevron = if open {
        lucide::CHEVRON_DOWN
    } else {
        lucide::CHEVRON_RIGHT
    };
    let toggle_size = (height * 0.5).round();
    let title: AnyElement = match path_segments(&paint.title) {
        Some(segments) => {
            let last = segments.len() - 1;
            view! {
                <div class="flex-row items-center min-w-0 overflow-hidden" gap={pad * 0.25}>
                    for (i, segment) in segments.into_iter().enumerate() {
                        if i > 0 {
                            <text size={font_size} color={colors.faint}>"/"</text>
                        }
                        <text
                            size={font_size}
                            class="truncate"
                            color={if i == last { colors.strong } else { colors.muted }}
                            @when {i == last} { class="font-semibold" }
                        >
                            {segment}
                        </text>
                    }
                </div>
            }
        }
        None => view! {
            <text size={font_size} class="font-semibold truncate" color={colors.strong}>
                {&*paint.title}
            </text>
        },
    };
    view! {
        <div
            w={width}
            h={height}
            class="flex-row items-center"
            gap={pad * 0.5}
            pr={pad}
            pl={pad * 0.5}
            border_b={colors.border}
            bg={colors.file_header}
            @when {env.accessible} {
                accessibility_id={format!("{id}.file.{}", paint.file)}
                accessibility_role={Role::Heading}
                aria-label={format!("{}, {status}, {adds} added, {dels} removed", paint.title)}
            }
        >
            <div
                w={toggle_size}
                h={toggle_size}
                rounded={4.0}
                class="flex-row items-center justify-center shrink-0 cursor-pointer"
                hover_bg={colors.hover}
                on:click={toggle}
                @when {env.accessible} {
                    accessibility_role={Role::Button}
                    aria-label={if open { "Collapse file" } else { "Expand file" }}
                    aria-expanded={open}
                }
            >
                <icon svg={chevron} size={(toggle_size * 0.75).round()} color={colors.muted} />
            </div>
            if let Some(prefix) = prefix {
                {prefix}
            }
            {title}
            <div
                class="shrink-0"
                px={pad * 0.3}
                py={pad * 0.1}
                rounded={4.0}
                bg={DiffColors::tint(colors.file_header, tone)}
                test_id="diff.file.state"
            >
                <text size={small} class="font-semibold" color={tone}>{state}</text>
            </div>
            <div class="flex-row items-center shrink-0" gap={pad * 0.25}>
                <text size={small} class="font-mono" color={colors.add_text}>"+{adds}"</text>
                <text size={small} class="font-mono" color={colors.del_text}>"\u{2212}{dels}"</text>
            </div>
            if paint.binary {
                <text size={small} color={colors.muted}>"binary"</text>
            }
            if let Some(summary) = &paint.summary {
                <text size={small} class="truncate" color={colors.muted}>{&**summary}</text>
            }
            if let Some(metadata) = metadata {
                {metadata}
            }
            if let Some(note) = collapsed_note {
                {note}
            }
            <div class="flex-1" min_w={0.0} />
            if let Some(actions) = actions {
                <div class="shrink-0">{actions}</div>
            }
            if let Some(trailing) = trailing {
                <div class="shrink-0" w={1.0} h={(height * 0.45).round()} bg={colors.border} />
                <div class="shrink-0">{trailing}</div>
            }
        </div>
    }
}

#[allow(clippy::too_many_arguments)]
fn move_note(
    paint: &RowPaint,
    width: f32,
    height: f32,
    indent: f32,
    font_size: f32,
    focused: bool,
    look: Look,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let colors = look.colors;
    let env = look.env;
    let Some(note) = paint.note else {
        return view! { <div w={width} h={height} /> };
    };
    let jump = on_event(DiffEvent::JumpMove {
        file: paint.file,
        index: note.index,
        here: note.here,
    });
    let arrow = match note.up {
        Some(false) => lucide::ARROW_DOWN,
        _ => lucide::ARROW_UP,
    };
    let size = font_size * 0.9;
    let label = paint.title.to_string();
    let accent = colors.tone(HighlightKind::Keyword);
    let (bg, ink) = if focused {
        (Some(colors.hover), accent)
    } else {
        (None, colors.muted)
    };
    view! {
        <div w={width} h={height} class="flex-row items-center" pl={indent}>
            <div
                class="flex-row items-center cursor-pointer"
                gap={size * 0.4}
                px={size * 0.4}
                h={(height - 2.0).max(size)}
                rounded={4.0}
                @when {let Some(bg) = bg} { bg={bg} }
                hover_bg={colors.hover}
                on:click={jump}
                @when {env.accessible} {
                    accessibility_role={Role::Button}
                    aria-label={label.clone()}
                }
            >
                <icon svg={arrow} size={size.round()} color={accent} />
                <text size={size} color={ink}>{label}</text>
            </div>
        </div>
    }
}

fn hunk_range(hunk: &HunkSource) -> String {
    format!(
        "@@ \u{2212}{},{} +{},{} @@",
        hunk.old.start + 1,
        hunk.old.len(),
        hunk.new.start + 1,
        hunk.new.len()
    )
}

/// A hunk header or collapsed gap, drawn as the separator choice says.
#[allow(clippy::too_many_arguments)]
fn separator(
    paint: &RowPaint,
    cx: &SeparatorContext,
    scope: Option<Arc<[Arc<str>]>>,
    actions: Option<AnyElement>,
    pad: f32,
    font_size: f32,
    look: Look,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let (width, height) = (cx.width, cx.height);
    let colors = look.colors;
    let env = look.env;
    let gap = cx.gap;
    if look.presentation.separators == HunkSeparator::Compact {
        return match gap {
            Some(gap) => view! {
                <div
                    w={width}
                    h={height}
                    bg={colors.separator}
                    hover_bg={colors.hover}
                    class="cursor-pointer"
                    on:click={on_event(DiffEvent::Expand(gap, Reveal::All))}
                    @when {env.accessible} {
                        accessibility_role={Role::Button}
                        aria-label={"Show all unchanged lines"}
                        aria-description={&*paint.title}
                    }
                />
            },
            None => view! { <div w={width} h={height} bg={colors.separator} /> },
        };
    }
    let controls = gap.filter(|_| look.presentation.separators == HunkSeparator::ContextControls);
    let gutter_w = cx
        .columns
        .sides
        .iter()
        .flatten()
        .next()
        .map_or(pad * 4.0, |c| c.gutter_x + c.gutter_w);
    let (lead, section) = match (gap, paint.title.split_once("    ")) {
        (Some(_), Some((lead, rest))) => (Some(lead.to_owned()), rest.to_owned()),
        (Some(_), None) => (Some(paint.title.to_string()), String::new()),
        (None, _) => (None, paint.title.to_string()),
    };
    let range = cx.hunk.as_ref().map(hunk_range);
    let section = section
        .rsplit_once("@@")
        .map(|(_, text)| text.trim().to_owned())
        .filter(|text| !text.is_empty());
    let small = font_size * 0.9;
    let scope_label = scope.filter(|s| !s.is_empty()).map(|scopes| {
        view! {
            <div class="flex-row items-center min-w-0 overflow-hidden" gap={pad * 0.25}>
                <text size={small} color={colors.muted}>"in"</text>
                for (i, scope) in scopes.iter().enumerate() {
                    if i > 0 {
                        <text size={small} color={colors.muted}>"\u{203A}"</text>
                    }
                    <text size={small} class="font-medium truncate" color={colors.text}>
                        {scope.to_string()}
                    </text>
                }
            </div>
        }
    });
    let has_scope = scope_label.is_some();
    view! {
        <div
            w={width}
            h={height}
            class="flex-row items-center"
            gap={pad * 0.5}
            pr={pad}
            border_t={colors.border}
            border_b={colors.border}
            bg={colors.separator}
            @when {env.accessible} {
                accessibility_role={Role::Group}
                aria-label={paint.title.to_string()}
            }
        >
            <div min_w={gutter_w} h={height} px={pad * 0.5} class="flex-row items-center justify-center shrink-0" gap={pad * 0.25}>
                if let Some(gap) = controls {
                    for (reveal, icon, label) in gap_controls(gap) {
                        {expand_button(gap, reveal, icon, label, height, colors, env, on_event)}
                    }
                }
            </div>
            if let Some(scope) = scope_label {
                {scope}
            }
            if let Some(lead) = lead.filter(|_| !has_scope) {
                <text size={small} color={colors.muted}>{lead}</text>
            }
            if let Some(range) = range {
                <text size={small * 0.95} class="font-mono" color={colors.faint}>{range}</text>
            }
            if let Some(section) = section.filter(|_| !has_scope) {
                <text size={small * 0.95} class="font-mono truncate" color={colors.faint}>{section}</text>
            }
            <div class="flex-1" min_w={0.0} />
            if let Some(actions) = actions {
                <div class="shrink-0">{actions}</div>
            }
        </div>
    }
}

const UNFOLD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22v-6"/><path d="M12 8V2"/><path d="M4 12H2"/><path d="M10 12H8"/><path d="M16 12h-2"/><path d="M22 12h-2"/><path d="m15 19-3 3-3-3"/><path d="m15 5-3-3-3 3"/></svg>"#;

/// The expand controls of a gap: up and down between hunks, down only
/// after the last hunk, and all.
fn gap_controls(gap: GapId) -> Vec<(Reveal, &'static str, String)> {
    let mut controls = Vec::with_capacity(3);
    if gap.hunk.is_some() {
        controls.push((
            Reveal::Up,
            lucide::CHEVRON_UP,
            format!("Show {REVEAL_STEP} more lines above the next change"),
        ));
    }
    controls.push((
        Reveal::Down,
        lucide::CHEVRON_DOWN,
        format!("Show {REVEAL_STEP} more lines below the previous change"),
    ));
    controls.push((Reveal::All, UNFOLD, "Show all unchanged lines".to_owned()));
    controls
}

#[allow(clippy::too_many_arguments)]
fn expand_button(
    gap: GapId,
    reveal: Reveal,
    icon: &'static str,
    label: String,
    height: f32,
    colors: DiffColors,
    env: CollectionEnv,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let size = (height - 6.0).max(12.0);
    view! {
        <div
            w={size}
            h={size}
            rounded={4.0}
            class="flex-row items-center justify-center"
            hover_bg={colors.hover}
            cursor={CursorHint::Pointer}
            tooltip={label.clone()}
            on:click={on_event(DiffEvent::Expand(gap, reveal))}
            @when {env.accessible} { accessibility_role={Role::Button} aria-label={label} }
        >
            <icon svg={icon} size={(size * 0.7).round()} color={colors.muted} />
        </div>
    }
}

struct LineDrag {
    press: ClickEvent,
    on_line: fn(LinePointer) -> Action,
}

impl DragHandler for LineDrag {
    fn on_press(&mut self) -> Vec<Action> {
        vec![(self.on_line)(LinePointer {
            x: self.press.x,
            y: self.press.y,
            drag: false,
        })]
    }

    fn on_move(&mut self, x: f32, y: f32) -> Vec<Action> {
        vec![(self.on_line)(LinePointer { x, y, drag: true })]
    }

    fn on_release(&mut self) -> DragReleaseResult {
        DragReleaseResult {
            actions: Vec::new(),
        }
    }

    fn on_cancel(&mut self) -> Vec<Action> {
        Vec::new()
    }

    fn cursor(&self) -> CursorHint {
        CursorHint::Pointer
    }
}

/// A drag that selects text. The press is reported at press time so the
/// anchor lands on what was under the pointer.
struct SelectDrag {
    press: ClickEvent,
    on_event: fn(DiffEvent) -> Action,
}

impl SelectDrag {
    fn follows(&self) -> bool {
        let mods = self.press.modifiers;
        if cfg!(target_os = "macos") {
            mods.cmd
        } else {
            mods.ctrl
        }
    }
}

impl DragHandler for SelectDrag {
    fn on_press(&mut self) -> Vec<Action> {
        let press = (self.on_event)(DiffEvent::Press {
            x: self.press.x,
            y: self.press.y,
        });
        if !self.follows() {
            return vec![press];
        }
        vec![
            press,
            (self.on_event)(DiffEvent::Release),
            (self.on_event)(DiffEvent::Key(DiffKey::Definition)),
        ]
    }

    fn on_move(&mut self, x: f32, y: f32) -> Vec<Action> {
        if self.follows() {
            return Vec::new();
        }
        vec![(self.on_event)(DiffEvent::Drag { x, y })]
    }

    fn on_release(&mut self) -> DragReleaseResult {
        let actions = if self.follows() {
            Vec::new()
        } else {
            vec![(self.on_event)(DiffEvent::Release)]
        };
        DragReleaseResult { actions }
    }

    /// The selection made so far stays; the release only ends the drag.
    fn on_cancel(&mut self) -> Vec<Action> {
        self.on_release().actions
    }

    fn cursor(&self) -> CursorHint {
        CursorHint::Text
    }
}

/// Records its child's bounds every frame, replayed or not, so pointer
/// events map into the view.
struct BoundsProbe {
    child: AnyElement,
    bounds: Rc<Cell<Rect>>,
    /// A drag autoscrolls: ask for the next frame.
    autoscrolling: bool,
}

impl Element for BoundsProbe {
    type LayoutState = ();
    type PrepaintState = ();

    fn request_layout(
        &mut self,
        engine: &mut LayoutEngine,
        cx: &mut ElementContext,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(engine, cx), ())
    }

    fn prepaint(
        &mut self,
        bounds: Bounds,
        _layout: &mut (),
        engine: &LayoutEngine,
        cx: &mut ElementContext,
    ) {
        self.bounds.set(bounds);
        self.child.prepaint(engine, cx);
    }

    fn paint(
        &mut self,
        _bounds: Bounds,
        _layout: &mut (),
        _prepaint: &mut (),
        engine: &LayoutEngine,
        scene: &mut Scene,
        cx: &mut ElementContext,
    ) {
        if self.autoscrolling {
            cx.request_frame_at_ms(cx.clock_ms + AUTOSCROLL_FRAME_MS);
        }
        self.child.paint(engine, scene, cx);
    }
}

impl IntoAnyElement for BoundsProbe {
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}
