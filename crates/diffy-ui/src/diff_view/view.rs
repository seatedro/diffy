//! The diff view's elements: one cache boundary for the whole view, and
//! one per gutter cell, text cell, and header row inside it, so an
//! unchanged frame replays without building or allocating and a scrolled
//! frame builds only the rows entering the window.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use accesskit::Role;
use diffy_diff::{GapId, Mode, Reveal, RowKind, Side};
use quark::view;
use quark_render::scene::{Rect, RichTextPrimitive, ShapedText};
use quark_render::{FontKind, RectPrimitive, Scene};
use quark_text::{TextParams, TextStyle};
use quark_ui::Action;
use quark_ui::element::{
    AnyElement, Bounds, CacheKey, ClickEvent, CursorHint, DragHandler, DragReleaseResult, Element,
    ElementContext, IntoAnyElement, LayoutEngine, LayoutId, ScrollActionBuilder, cached, canvas,
    div, inputs_hash, svg_icon, text,
};
use quark_ui::icons::lucide;
use quark_ui::style::Styled;
use quark_ui::theme::Theme;

use super::decorator::{
    AnnotationContext, DiffDecorator, DiffSources, DiffViewOptions, GutterContext, HeaderContext,
    HeaderSlot, LineContext, LineMark, SeparatorContext,
};
use super::interaction::AUTOSCROLL_FRAME_MS;
use super::paint::{self, Cue, LineColors};
use super::prepared::{
    FileFact, FrameRow, LinePaint, MARKER_BAR_W, Metrics, PreparedKind, RowPaint, SearchMark,
    ViewFrame,
};
use super::presentation::{
    DiffColors, DiffMarkers, DiffNumbers, DiffPresentation, EmptySideFill, FileHeaders,
    HunkSeparator,
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

pub(crate) fn diff_view_custom(
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
    // The cache watches the sideways scroll handles itself.
    let hash = inputs_hash(&(
        state.frame_id,
        env,
        on_event as usize,
        look.colors_key,
        deco.key,
        keys.as_ptr() as usize,
        keys.len(),
    ));
    BoundsProbe {
        child: view! {
            <cached(state.id, hash, move || build(&frame, look, &deco, keys, on_event))
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
                class="cursor-text"
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
                    >
                        <div w={column.gutter_w} class="flex-col" translate={(0.0, first_top)}>
                            for row in &frame.rows {
                                {gutter_cell(frame, row, side, column.gutter_w, look, deco)}
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
                            for row in &frame.rows {
                                {text_cell(frame, row, side, laid_w, origin, look)}
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
                for row in frame.rows.iter().filter(|r| r.focused && r.height > 0.0) {
                    {focus_overlay(frame, row, look, deco)}
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
        (RowKind::Added, _) | (RowKind::Modified, Side::New) => Some(Cue::Added),
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

fn gutter_cell(
    frame: &ViewFrame,
    row: &FrameRow,
    side: Side,
    width: f32,
    look: Look,
    deco: &Deco,
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
    let mark = deco.mark(&row.paint, kind, shown);
    let hash = inputs_hash(&(
        row.paint.stamp,
        numbers,
        count,
        mode,
        height.to_bits(),
        width.to_bits(),
        look.colors_key,
        presentation.markers,
        mark,
    ));
    let build = move || {
        let bg = match mark {
            Some(LineMark::Selected) => Some(colors.selection),
            None => colors.line(kind, shown),
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
                            presentation.markers,
                            &numbers[..count],
                            cue(kind, shown),
                            colors,
                        );
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
    colors: DiffColors,
) {
    let style = TextStyle::new(m.font_size)
        .kind(FontKind::Mono)
        .line_height(m.line_h);
    let mut x = bounds.x;
    if markers == DiffMarkers::Bars {
        if let Some(cue) = cue {
            let color = match cue {
                Cue::Added => colors.add_marker,
                Cue::Removed => colors.del_marker,
            };
            paint::marker(
                scene,
                x,
                bounds.y,
                MARKER_BAR_W,
                bounds.height,
                cue,
                color,
                cx.scale_factor,
            );
        }
        x += MARKER_BAR_W;
    }
    let number_color = match cue {
        Some(Cue::Added) => colors.add_number,
        Some(Cue::Removed) => colors.del_number,
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
        None => None,
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
fn text_cell(
    frame: &ViewFrame,
    row: &FrameRow,
    side: Side,
    width: f32,
    origin: f64,
    look: Look,
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
    let search = row.search[shown as usize].clone();
    let pad = frame.metrics.text_pad;
    let font_size = frame.metrics.font_size;
    let position = (row.index + 1, frame.row_count);
    let colors = look.colors;
    let env = look.env;
    let hash = inputs_hash(&(
        paint.stamp,
        selected,
        &search,
        width.to_bits(),
        origin.to_bits(),
        height.to_bits(),
        env.accessible,
        mode,
        position,
        look.colors_key,
    ));
    let build = move || {
        let line = paint.sides[shown as usize].as_ref().expect("shown line");
        let canvas_paint = paint.clone();
        view! {
            <div
                w={width}
                h={height}
                class="shrink-0"
                @when {let Some(bg) = colors.line(kind, shown)} { bg={bg} }
                @when {env.accessible} {
                    accessibility_role={Role::ListItem}
                    aria-label={line.layout.text().to_string()}
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
                <canvas(
                    move |bounds, scene, cx| {
                        if let Some(line) = &canvas_paint.sides[shown as usize] {
                            paint_text(
                                bounds,
                                scene,
                                line,
                                shown,
                                &search,
                                selected,
                                pad,
                                origin,
                                font_size,
                                colors,
                                cx.scale_factor,
                            );
                        }
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

#[allow(clippy::too_many_arguments)]
fn paint_text(
    bounds: Bounds,
    scene: &mut Scene,
    line: &LinePaint,
    side: Side,
    search: &[SearchMark],
    selected: Option<(usize, usize)>,
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
    paint::line(
        scene,
        (bounds.x + pad + window_x, bounds.y),
        &line.layout,
        line.tones.iter().map(|&k| colors.tone(k)).collect(),
        &line.words,
        search,
        selected,
        LineColors {
            text: colors.text,
            word,
            search: colors.search_match,
            search_active: colors.search_active,
            search_outline: colors.search_outline,
            selection: colors.selection,
        },
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

/// The keyboard focus outline over a row, painted after everything else
/// in it, with the decorator's gutter utility for a line row.
fn focus_overlay(frame: &ViewFrame, row: &FrameRow, look: Look, deco: &Deco) -> AnyElement {
    let (width, height) = (frame.viewport.0, row.height);
    let color = look.colors.focused_row;
    let utility = row
        .paint
        .kind
        .diff()
        .filter(|k| k.is_line())
        .and_then(|kind| {
            let decorator = deco.decorator.as_ref()?;
            let mode = frame.columns.mode;
            let side = match mode {
                Mode::Unified => shown_side(mode, kind, Side::New),
                // The new side's gutter, unless the row has only an old line.
                Mode::Split if row.paint.source_lines[1].is_some() => Side::New,
                Mode::Split => Side::Old,
            };
            let line = row.paint.source_lines[side as usize]?;
            let column = frame.columns.of(side);
            let cx = GutterContext {
                file: row.paint.file,
                source: deco.file(row.paint.file),
                side,
                line,
                kind,
                width: column.gutter_w,
                height,
            };
            Some((column, decorator.gutter_utility(&cx)?))
        });
    view! {
        <div class="absolute left-0" top={row.top} w={width} h={height}>
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
            if let Some((column, element)) = utility {
                <div class="absolute top-0" left={column.gutter_x} w={column.gutter_w} h={height}>
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
    let hash = inputs_hash(&(
        paint.stamp,
        width.to_bits(),
        height.to_bits(),
        look.env,
        on_event as usize,
        look.colors_key,
        look.presentation,
        deco.key,
        &paint.kind,
    ));
    let build = move || match &paint.kind {
        PreparedKind::Diff(RowKind::FileHeader) => {
            file_header(&paint, id, width, height, pad, font_size, look, &deco)
        }
        PreparedKind::Diff(_) => {
            let custom = deco.decorator.as_ref().and_then(|d| {
                let hunk = match paint.kind.diff() {
                    Some(RowKind::Gap) => paint.gap.and_then(|g| g.hunk),
                    _ => Some((key & 0x3F_FFFF_FFFF) as u32),
                };
                let sources = deco.sources.as_ref();
                d.separator(&SeparatorContext {
                    file: paint.file,
                    gap: paint.gap,
                    source: deco.file(paint.file),
                    hunk: hunk.and_then(|h| sources?.hunk(paint.file, h)),
                    hidden: paint.hidden,
                    title: &paint.title,
                    columns,
                    width,
                    height,
                })
            });
            match custom {
                Some(custom) => view! { <div w={width} h={height}>{custom}</div> },
                None => separator(&paint, width, height, pad, font_size, look, on_event),
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
    let (prefix, metadata, actions) = (
        slot(HeaderSlot::Prefix),
        slot(HeaderSlot::Metadata),
        slot(HeaderSlot::Actions),
    );
    let status = paint.status.name();
    let colors = look.colors;
    let env = look.env;
    view! {
        <div
            w={width}
            h={height}
            class="flex-row items-center"
            gap={pad * 0.5}
            px={pad}
            border_b={colors.border}
            bg={colors.file_header}
            @when {env.accessible} {
                accessibility_id={format!("{id}.file.{}", paint.file)}
                accessibility_role={Role::Heading}
                aria-label={format!("{}, {status}, {adds} added, {dels} removed", paint.title)}
            }
        >
            if let Some(prefix) = prefix {
                {prefix}
            }
            <text size={font_size * 0.85} color={colors.muted}>{status}</text>
            <text size={font_size} class="font-semibold truncate" color={colors.text}>
                {&*paint.title}
            </text>
            if let Some(metadata) = metadata {
                {metadata}
            }
            <div class="flex-1" />
            if paint.binary {
                <text size={font_size * 0.85} color={colors.muted}>"binary"</text>
            }
            <text size={font_size} color={colors.add_text}>"+{adds}"</text>
            <text size={font_size} color={colors.del_text}>"-{dels}"</text>
            if let Some(actions) = actions {
                {actions}
            }
        </div>
    }
}

/// A hunk header or collapsed gap, drawn as the separator choice says.
fn separator(
    paint: &RowPaint,
    width: f32,
    height: f32,
    pad: f32,
    font_size: f32,
    look: Look,
    on_event: fn(DiffEvent) -> Action,
) -> AnyElement {
    let colors = look.colors;
    let env = look.env;
    let gap = (paint.kind.diff() == Some(RowKind::Gap)).then(|| paint.gap.expect("gap row"));
    match (look.presentation.separators, gap) {
        (HunkSeparator::Compact, Some(gap)) => view! {
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
        (HunkSeparator::Compact, None) => view! {
            <div w={width} h={height} bg={colors.separator} />
        },
        (separators, gap) => {
            let controls = gap.filter(|_| separators == HunkSeparator::ContextControls);
            view! {
                <div
                    w={width}
                    h={height}
                    class="flex-row items-center"
                    gap={pad * 0.5}
                    px={pad}
                    border_b={colors.border}
                    bg={colors.separator}
                >
                    if let Some(gap) = controls {
                        for (reveal, icon, label) in gap_controls(gap) {
                            {expand_button(gap, reveal, icon, label, height, colors, env, on_event)}
                        }
                    }
                    <text size={font_size * 0.9} class="font-mono" color={colors.muted}>
                        {&*paint.title}
                    </text>
                </div>
            }
        }
    }
}

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
    controls.push((
        Reveal::All,
        lucide::LIST,
        "Show all unchanged lines".to_owned(),
    ));
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
            on:click={on_event(DiffEvent::Expand(gap, reveal))}
            @when {env.accessible} { accessibility_role={Role::Button} aria-label={label} }
        >
            <icon svg={icon} size={(size * 0.7).round()} color={colors.muted} />
        </div>
    }
}

/// A drag that selects text. The press is reported at press time so the
/// anchor lands on what was under the pointer.
struct SelectDrag {
    press: ClickEvent,
    on_event: fn(DiffEvent) -> Action,
}

impl DragHandler for SelectDrag {
    fn on_press(&mut self) -> Vec<Action> {
        vec![(self.on_event)(DiffEvent::Press {
            x: self.press.x,
            y: self.press.y,
        })]
    }

    fn on_move(&mut self, x: f32, y: f32) -> Vec<Action> {
        vec![(self.on_event)(DiffEvent::Drag { x, y })]
    }

    fn on_release(&mut self) -> DragReleaseResult {
        DragReleaseResult {
            actions: vec![(self.on_event)(DiffEvent::Release)],
        }
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
