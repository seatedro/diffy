//! Measurements of the whole diff stack on 63 MiB sides: line diff, first
//! painted frame, syntax colors (with the JavaScript pack built, see
//! `diff_syntax.rs`), scrolling frames, settled-frame allocations, and
//! peak memory. Fixtures are generated from fixed seeds.
//!
//! Run in release with
//! `cargo test --release -p diffy-ui --test diff_large -- --ignored --nocapture`;
//! `DIFFY_DIFF_FIXTURE_MIB` picks another size, `DIFFY_DIFF_CASE` the
//! cases whose names contain it, and `DIFFY_DIFF_NO_SYNTAX` skips colors.

use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use diffy_diff::fixtures::{
    edit_line, every_nth, javascript, minified, one_block, peak_rss, repetitive, reset_peak_rss,
    scattered_edits,
};
use diffy_diff::{Side, diff_texts};
use diffy_syntax::{HighlightKind, testing};
use diffy_ui::diff_view::FindOptions;
use diffy_ui::diff_view::syntax::SyntaxStatus;
use diffy_ui::{DiffEvent, DiffOutcome, DiffViewState, diff_view};
use quark_app::quark_ui::element::AnyElement;
use quark_app::quark_ui::test_alloc::{self, Counting};
use quark_app::quark_ui::{Action, FocusId};
use quark_app::testing::UiTestHarness;
use quark_app::{UiApp, UiContext, ViewContext};
use quark_components::CollectionEnv;

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const FOCUS: FocusId = FocusId::from_key("large.diff");
const SIZE: (f32, f32) = (1200.0, 800.0);

struct Large {
    diff: DiffViewState,
}

impl UiApp for Large {
    type Action = DiffEvent;
    type Message = ();

    fn view(&mut self, cx: &mut ViewContext) -> AnyElement {
        let (width, height) = cx.frame.size();
        self.diff.set_viewport(width, height);
        let scale = cx.frame.scale_factor();
        let now_ms = cx.frame.elapsed().as_millis() as u64;
        let text = cx.frame.text();
        self.diff
            .prepare(&mut text.system, &mut text.layouts, scale, now_ms);
        let env = CollectionEnv {
            focused: cx.is_focused(FOCUS),
            accessible: cx.frame.accessibility_active(),
        };
        diff_view(&mut self.diff, cx.theme, env, Action::new)
    }

    fn update(&mut self, event: DiffEvent, _: &mut UiContext) {
        let _ = matches!(self.diff.handle(event), DiffOutcome::Copy(_));
    }
}

/// Whether a line on screen has syntax colors.
fn colored(ui: &UiTestHarness<Large>) -> bool {
    ui.app().diff.frame().is_some_and(|frame| {
        frame
            .rows
            .iter()
            .flat_map(|row| row.paint.sides.iter().flatten())
            .any(|line| line.tones.iter().any(|&k| k != HighlightKind::Normal))
    })
}

/// The file header's syntax status, from the frame.
fn status(ui: &UiTestHarness<Large>) -> Option<SyntaxStatus> {
    let frame = ui.app().diff.frame()?;
    frame
        .sticky_header
        .iter()
        .chain(&frame.rows)
        .find_map(|row| row.paint.syntax)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / f64::from(1 << 20)
}

/// Frame times of `steps` frames, each after `step` moved the view.
fn frames(
    ui: &mut UiTestHarness<Large>,
    steps: usize,
    mut step: impl FnMut(&mut UiTestHarness<Large>, usize),
) -> (Duration, Duration) {
    let (mut total, mut worst) = (Duration::ZERO, Duration::ZERO);
    for i in 0..steps {
        step(ui, i);
        let started = Instant::now();
        ui.frame();
        let took = started.elapsed();
        total += took;
        worst = worst.max(took);
    }
    (total / steps as u32, worst)
}

fn wait(woke: &Receiver<()>, timeout: Duration) -> bool {
    woke.recv_timeout(timeout).is_ok()
}

/// How far from its column the new side's long line paints, scrolled
/// to a fraction of a point short of its far end (hundreds of millions of
/// points in), where f32 offsets would round by tens of points.
fn far_end_error(ui: &mut UiTestHarness<Large>) -> f64 {
    let end = ui
        .app()
        .diff
        .frame()
        .map_or(0.0, |f| f.content_w[Side::New as usize]);
    let to = end - 2_000.5;
    ui.app()
        .diff
        .horizontal_scroll(Side::New)
        .set_offset(to, 0.0);
    ui.frame();
    ui.frame();
    let Some(frame) = ui.app().diff.frame().cloned() else {
        return f64::NAN;
    };
    let shown = frame.rows.iter().rev().find_map(|row| {
        let line = row.paint.sides[Side::New as usize].as_ref()?;
        Some((line.layout.text().to_owned(), line.window?.x))
    });
    let Some((text, x)) = shown else {
        return f64::NAN;
    };
    let Some(run) = ui.painted_texts().into_iter().find(|t| t.text == text) else {
        return f64::NAN;
    };
    let column = frame.columns.of(Side::New);
    let expected = f64::from(column.text_x + frame.metrics.text_pad) + x - to;
    (f64::from(run.bounds.x) - expected).abs()
}

#[test]
#[ignore = "measurement, prints a report"]
fn report_large_diffs() {
    let Some(store) = testing::store_with("javascript") else {
        return;
    };
    const MIB: usize = 1 << 20;
    let size = std::env::var("DIFFY_DIFF_FIXTURE_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
        * MIB;
    // A little under the size, so edits keep the new side within it.
    let size = size - size / 64;
    type Make = fn(usize) -> (String, String);
    let cases: [(&str, Make); 4] = [
        ("1% scattered", |size| {
            let base = javascript(1, size);
            let new = scattered_edits(&base, 4, 10);
            (base, new)
        }),
        ("one huge block", |size| {
            let base = javascript(1, size);
            let new = one_block(&base, 5, 0.5);
            (base, new)
        }),
        ("minified line", |size| {
            let line = minified(2, size);
            let new = edit_line(&line, 6, 5);
            (line, new)
        }),
        ("many small hunks", |size| {
            let old = repetitive(3, size);
            let new = every_nth(&old, 7, 8);
            (old, new)
        }),
    ];
    // `DIFFY_DIFF_CASE` picks the cases whose names contain it.
    let only = std::env::var("DIFFY_DIFF_CASE").unwrap_or_default();
    for (name, make) in cases.into_iter().filter(|(name, _)| name.contains(&*only)) {
        let (old, new) = make(size);
        reset_peak_rss();
        let inputs = peak_rss().unwrap_or(0);
        let started = Instant::now();
        let doc = diff_texts(Some("f.js"), Some("f.js"), Some(&old), Some(&new), 3);
        let diffed = started.elapsed();
        drop((old, new));
        // What the diff and its document take, before the view.
        let diff_peak = peak_rss().unwrap_or(0);
        let viewed = Instant::now();
        let mut diff = DiffViewState::new("large.diff", FOCUS, doc);
        let state_built = viewed.elapsed();
        let (woke_tx, woke) = channel();
        diff.set_syntax_wake(move || {
            let _ = woke_tx.send(());
        });
        // `DIFFY_DIFF_NO_SYNTAX` leaves colors off, for quicker runs.
        let syntax = std::env::var_os("DIFFY_DIFF_NO_SYNTAX").is_none();
        if syntax {
            diff.enable_syntax(store.clone());
        }
        let mut ui = UiTestHarness::new(Large { diff }, SIZE, 1.0);
        let first_paint = started.elapsed();
        // The view's own part: state and first frame, without the diff.
        let view_paint = viewed.elapsed();
        let rows = ui.app().diff.frame().map_or(0, |frame| frame.row_count);
        let paint_peak = peak_rss().unwrap_or(0);

        // Colors: the first on screen, then every part until both sides are
        // done.
        let (mut first_colors, mut full) = (None, None);
        let deadline = Instant::now() + Duration::from_secs(600);
        while syntax && full.is_none() && Instant::now() < deadline {
            if !wait(&woke, Duration::from_secs(120)) {
                break;
            }
            while woke.try_recv().is_ok() {}
            ui.frame();
            if colored(&ui) {
                first_colors.get_or_insert(started.elapsed());
            }
            match status(&ui) {
                Some(SyntaxStatus::Ready) => full = Some(started.elapsed()),
                Some(SyntaxStatus::Pending | SyntaxStatus::Streaming { .. }) | None => {}
                Some(other) => {
                    eprintln!("  status {other:?}");
                    break;
                }
            }
        }

        for _ in 0..3 {
            ui.frame();
        }
        let ((), settled_before) = test_alloc::count(|| {
            ui.frame();
        });
        eprintln!("  settled before scrolling: {settled_before} allocations");
        // Scrolling: wheel steps down, then sideways for the long line.
        let (avg, worst) = frames(&mut ui, 200, |ui, _| {
            ui.pointer_move((600.0, 400.0));
            ui.wheel(0.0, 400.0);
        });
        let wheeled = ui.app().diff.scroll_offset();
        let (jump_avg, jump_worst) = frames(&mut ui, 20, |ui, i| {
            let to = ui.app().diff.content_height() * (i as f32 / 20.0);
            ui.app_mut().diff.handle(DiffEvent::ScrollTo(to));
        });
        let sideways = if name == "minified line" {
            let (a, w) = frames(&mut ui, 200, |ui, i| {
                let x = i as f32 * 4_000.0;
                ui.app()
                    .diff
                    .horizontal_scroll(Side::New)
                    .set_offset(x, 0.0);
            });
            let error = far_end_error(&mut ui);
            format!(", sideways {a:.2?} avg / {w:.2?} worst, far end painted {error:.2} pt off")
        } else {
            String::new()
        };
        // Find over both whole sources, on the UI thread.
        let searched = Instant::now();
        ui.app_mut()
            .diff
            .set_find_query("compute(", FindOptions::default());
        let search = searched.elapsed();
        ui.app_mut().diff.set_find_query("", FindOptions::default());
        for _ in 0..30 {
            ui.frame();
        }

        let ((), settled) = test_alloc::count(|| {
            ui.frame();
        });
        if settled > 0 && std::env::var_os("DIFFY_DIFF_ALLOC_SITES").is_some() {
            let ((), sites) = test_alloc::profile(|| {
                ui.frame();
            });
            for (site, n) in sites.iter().take(8) {
                eprintln!("  {n:6}  {site}");
            }
        }
        let peak = peak_rss().unwrap_or(0);
        eprintln!(
            "{name}: {rows} rows; diff {diffed:.2?}, view state {state_built:.2?}, view to first paint \
             {view_paint:.2?}, first paint {first_paint:.2?}, first colors \
             {first_colors:.2?}, full syntax {full:.2?}; scroll {avg:.2?} avg / {worst:.2?} worst \
             (to {wheeled:.0} pt), \
             jumps {jump_avg:.2?} / {jump_worst:.2?}{sideways}; find {search:.2?}; settled frame \
             {settled} allocations; peak RSS {:.0} MiB ({:.0} after the diff, {:.0} at first paint, {:.0} with inputs)",
            mib(peak.saturating_sub(inputs)),
            mib(diff_peak.saturating_sub(inputs)),
            mib(paint_peak.saturating_sub(inputs)),
            mib(peak),
        );
    }
}
