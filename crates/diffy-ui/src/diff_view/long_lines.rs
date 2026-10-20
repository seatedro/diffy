//! Long lines: which part of a line too long to shape whole the view
//! shapes, and word diffs of long line pairs, computed off the UI thread.
//!
//! A line longer than [`WINDOW_LINE_BYTES`] (without wrap) is shaped only
//! around the columns its column scrolls to: a [`LineWindow`] a few hundred
//! columns either side of the view, snapped to a grid so scrolling within
//! it reshapes nothing. Where the window sits is estimated from column
//! counts (exact for ASCII in a monospace font; wide and combining
//! characters are estimated), and the glyphs inside it come from real
//! shaping. Selection, copy, and search use line byte offsets, so they see
//! the whole line wherever the window is.
//!
//! The word diff of a pair over [`SYNC_PAIR_BYTES`] runs on a thread of
//! its own and is kept per pair and options; until it lands the pair
//! shows no word highlights, and its arrival bumps
//! [`LongLines::generation`] so rows repaint.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use diffy_diff::{
    InlineDetail, InlineMode, InlineOptions, LineDetail, line_detail, paired_inline_diff,
};

use super::prepared::LineWindow;

/// Lines longer than this are shaped window by window (without wrap).
pub(crate) const WINDOW_LINE_BYTES: usize = 4_096;

/// Windows start and end on multiples of this many columns, one grid step
/// beyond the view on each side.
pub(crate) const GRID_COLUMNS: usize = 256;

/// Pairs whose lines together are at most this long are word diffed on the
/// spot, as short lines always were.
pub(crate) const SYNC_PAIR_BYTES: usize = 64 << 10;

/// Column maps kept, most recent first.
const KEPT_MAPS: usize = 16;

/// Word diffs of long pairs kept.
const KEPT_DIFFS: usize = 64;

/// A line's identity for caching: its text's address and where it starts.
type LineKey = (usize, usize);

/// A pair's identity for caching: both lines and the options' bits.
type PairKey = (LineKey, LineKey, (InlineMode, u32, usize));

fn options_key(options: &InlineOptions) -> (InlineMode, u32, usize) {
    (
        options.mode,
        options.min_kept_share.to_bits(),
        options.max_line_bytes,
    )
}

/// Changed word ranges of a pair, old side then new, in line bytes, and
/// why they are what they are.
pub(crate) type Words = Arc<([Vec<Range<usize>>; 2], InlineDetail)>;

/// A pair's word diff as the view keeps it.
fn words_of(d: diffy_diff::PairedInlineDiff) -> Words {
    let bytes = |v: Vec<Range<u32>>| -> Vec<Range<usize>> {
        v.into_iter()
            .map(|r| r.start as usize..r.end as usize)
            .collect()
    };
    Arc::new(([bytes(d.old), bytes(d.new)], d.detail))
}

/// Where columns fall in a long line.
enum ColumnMap {
    /// Every byte is one column.
    Ascii,
    /// `(byte, column)` every [`CHECKPOINT_BYTES`] or so, from `(0, 0)`.
    Checkpoints { at: Vec<(u32, u32)>, total: usize },
}

/// Bytes between [`ColumnMap::Checkpoints`].
const CHECKPOINT_BYTES: usize = 4_096;

impl ColumnMap {
    fn new(line: &str) -> Self {
        if line.is_ascii() {
            return Self::Ascii;
        }
        let mut at = vec![(0, 0)];
        let (mut column, mut next) = (0usize, CHECKPOINT_BYTES);
        for (byte, c) in line.char_indices() {
            if byte >= next {
                at.push((byte as u32, column as u32));
                next = byte + CHECKPOINT_BYTES;
            }
            column += width(c);
        }
        Self::Checkpoints { at, total: column }
    }

    fn total(&self, line: &str) -> usize {
        match self {
            Self::Ascii => line.len(),
            Self::Checkpoints { total, .. } => *total,
        }
    }

    /// The column where byte `byte` (a char boundary) starts.
    fn column_of(&self, line: &str, byte: usize) -> usize {
        match self {
            Self::Ascii => byte,
            Self::Checkpoints { at, .. } => {
                let k = at.partition_point(|&(b, _)| b as usize <= byte) - 1;
                let (from, column) = (at[k].0 as usize, at[k].1 as usize);
                column + line[from..byte].chars().map(width).sum::<usize>()
            }
        }
    }

    /// The first char at or after column `column` that takes a column (so
    /// never a combining mark), or the end.
    fn byte_of(&self, line: &str, column: usize) -> usize {
        match self {
            Self::Ascii => column.min(line.len()),
            Self::Checkpoints { at, .. } => {
                let k = at.partition_point(|&(_, c)| c as usize <= column) - 1;
                let (from, mut at_column) = (at[k].0 as usize, at[k].1 as usize);
                for (offset, c) in line[from..].char_indices() {
                    if at_column >= column && width(c) > 0 {
                        return from + offset;
                    }
                    at_column += width(c);
                }
                line.len()
            }
        }
    }
}

/// Estimated columns of `c` in a monospace font: none for combining marks
/// and joiners, two for wide East Asian characters and most emoji.
fn width(c: char) -> usize {
    match u32::from(c) {
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200B..=0x200F
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F => 0,
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// The last grapheme boundary of `line` at or before `byte`.
fn floor_grapheme(line: &str, byte: usize) -> usize {
    match line_detail(line, byte) {
        LineDetail::Complete => line.len(),
        LineDetail::Prefix { shown_bytes, .. } => shown_bytes as usize,
    }
}

/// A word diff waiting for the thread.
struct Job {
    key: PairKey,
    old: (Arc<str>, Range<usize>),
    new: (Arc<str>, Range<usize>),
    options: InlineOptions,
}

/// Called on the word thread after each result.
type Wake = Arc<dyn Fn() + Send + Sync>;

struct Thread {
    jobs: Sender<Job>,
    done: Receiver<(PairKey, Words)>,
    /// Read for each result, so a wake set after the thread started counts.
    wake: Arc<Mutex<Option<Wake>>>,
}

/// See the [module docs](self).
#[derive(Default)]
pub(crate) struct LongLines {
    maps: RefCell<Vec<(LineKey, Arc<ColumnMap>)>>,
    diffs: RefCell<HashMap<PairKey, Words>>,
    asked: RefCell<HashSet<PairKey>>,
    thread: RefCell<Option<Thread>>,
    generation: Cell<u64>,
    /// Per column: a sideways scroll requested from the handle, which it
    /// applies only while painting, and the frame it was requested in.
    expected: Cell<[Option<(f64, u64)>; 2]>,
}

impl LongLines {
    /// The window of `line` (starting at byte `start` of `text`) to shape
    /// when its column is scrolled `scroll_x` points and shows `view_w`
    /// points, with characters `char_w` wide; `None` for a line short
    /// enough to shape whole.
    pub(crate) fn window(
        &self,
        text: &Arc<str>,
        line: Range<usize>,
        scroll_x: f64,
        view_w: f32,
        char_w: f32,
    ) -> Option<LineWindow> {
        if line.len() <= WINDOW_LINE_BYTES || char_w <= 0.0 {
            return None;
        }
        let key = (Arc::as_ptr(text) as *const u8 as usize, line.start);
        let line_text = &text[line];
        let map = self.map(key, line_text);
        let total = map.total(line_text);
        let char_w64 = f64::from(char_w);
        let first = (scroll_x.max(0.0) / char_w64) as usize;
        let last = ((scroll_x.max(0.0) + f64::from(view_w)) / char_w64).ceil() as usize;
        let from = ((first / GRID_COLUMNS).saturating_sub(1) * GRID_COLUMNS).min(total);
        let to = ((last / GRID_COLUMNS + 2) * GRID_COLUMNS).min(total);
        let start = floor_grapheme(line_text, map.byte_of(line_text, from));
        let end = floor_grapheme(line_text, map.byte_of(line_text, to)).max(start);
        Some(LineWindow {
            start,
            end,
            x: map.column_of(line_text, start) as f64 * char_w64,
            width: total as f64 * char_w64,
        })
    }

    /// Where byte `byte` of a long line (as for [`Self::window`]) sits from
    /// the line's start, in points.
    pub(crate) fn x_of(
        &self,
        text: &Arc<str>,
        line: Range<usize>,
        byte: usize,
        char_w: f32,
    ) -> f64 {
        let key = (Arc::as_ptr(text) as *const u8 as usize, line.start);
        let line_text = &text[line];
        let byte = line_text.floor_char_boundary(byte);
        self.map(key, line_text).column_of(line_text, byte) as f64 * f64::from(char_w)
    }

    fn map(&self, key: LineKey, line: &str) -> Arc<ColumnMap> {
        let mut maps = self.maps.borrow_mut();
        if let Some(at) = maps.iter().position(|(k, _)| *k == key) {
            let entry = maps.remove(at);
            let map = entry.1.clone();
            maps.insert(0, entry);
            return map;
        }
        let map = Arc::new(ColumnMap::new(line));
        maps.insert(0, (key, map.clone()));
        maps.truncate(KEPT_MAPS);
        map
    }

    /// Records that column `slot` was asked to scroll to `x` in frame
    /// `frame`: windows use it until a later frame has painted it.
    pub(crate) fn expect_scroll(&self, slot: usize, x: f64, frame: u64) {
        let mut expected = self.expected.get();
        expected[slot] = Some((x, frame));
        self.expected.set(expected);
    }

    /// The scroll [`Self::expect_scroll`] recorded for `slot`, until a
    /// frame after `frame`'s next has been built.
    pub(crate) fn expected_scroll(&self, slot: usize, frame: u64) -> Option<f64> {
        let (x, at) = self.expected.get()[slot]?;
        (frame <= at + 1).then_some(x)
    }

    /// Bumped whenever a word diff lands, so rows showing it repaint.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// The changed words of a long pair of display lines (byte ranges of
    /// their stores' texts) under `options`, from the word thread: `None`
    /// while it works. `wake` is called when a result lands.
    pub(crate) fn words(
        &self,
        old: (&Arc<str>, Range<usize>),
        new: (&Arc<str>, Range<usize>),
        options: &InlineOptions,
        wake: Option<Wake>,
    ) -> Option<Words> {
        let key = (
            (Arc::as_ptr(old.0) as *const u8 as usize, old.1.start),
            (Arc::as_ptr(new.0) as *const u8 as usize, new.1.start),
            options_key(options),
        );
        if let Some(words) = self.diffs.borrow().get(&key) {
            return Some(words.clone());
        }
        if self.asked.borrow_mut().insert(key) {
            let mut thread = self.thread.borrow_mut();
            let thread = thread.get_or_insert_with(spawn);
            *thread.wake.lock().unwrap_or_else(PoisonError::into_inner) = wake;
            let _ = thread.jobs.send(Job {
                key,
                old: (old.0.clone(), old.1),
                new: (new.0.clone(), new.1),
                options: *options,
            });
        }
        None
    }

    /// Takes the word diffs that landed; true when any did.
    pub(crate) fn poll(&self) -> bool {
        let thread = self.thread.borrow();
        let Some(thread) = thread.as_ref() else {
            return false;
        };
        let mut landed = false;
        let mut diffs = self.diffs.borrow_mut();
        while let Ok((key, words)) = thread.done.try_recv() {
            if diffs.len() >= KEPT_DIFFS {
                // Rows on screen ask again; nothing else needs the rest.
                diffs.clear();
                self.asked.borrow_mut().clear();
            }
            self.asked.borrow_mut().insert(key);
            diffs.insert(key, words);
            landed = true;
        }
        if landed {
            self.generation.set(self.generation.get() + 1);
        }
        landed
    }
}

/// The word diff thread: it ends when its job sender is dropped with the
/// view.
fn spawn() -> Thread {
    let (jobs, job_rx) = channel::<Job>();
    let (done_tx, done) = channel();
    let wake: Arc<Mutex<Option<Wake>>> = Arc::default();
    let wakes = wake.clone();
    let _ = std::thread::Builder::new()
        .name("diffy-diff-words".to_owned())
        .spawn(move || {
            for job in job_rx {
                let (old, new) = (&job.old.0[job.old.1], &job.new.0[job.new.1]);
                let words = words_of(paired_inline_diff(old, new, &job.options));
                if done_tx.send((job.key, words)).is_err() {
                    return;
                }
                let wake = wakes.lock().unwrap_or_else(PoisonError::into_inner).clone();
                if let Some(wake) = wake {
                    wake();
                }
            }
        });
    Thread { jobs, done, wake }
}

/// `ranges` (line bytes) inside `window`, as bytes of the window's text.
pub(crate) fn clip_to_window(ranges: &[Range<usize>], window: &LineWindow) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter(|r| r.end > window.start && r.start < window.end)
        .map(|r| r.start.max(window.start) - window.start..r.end.min(window.end) - window.start)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's text and its x, for a line scrolled `scroll_x` points
    /// with 10-point characters and a 100-point view.
    fn shown(line: &str, scroll_x: f64) -> (String, f64, f64) {
        let text: Arc<str> = Arc::from(line);
        let w = LongLines::default()
            .window(&text, 0..line.len(), scroll_x, 100.0, 10.0)
            .unwrap();
        (line[w.start..w.end].to_owned(), w.x, w.width)
    }

    // Catches a window placed by bytes instead of columns (wide and
    // multibyte characters shift it), or cutting a grapheme: windows of a
    // line of two-column characters start on a character at the column
    // the view needs, and the whole width counts every column.
    #[test]
    fn windows_follow_columns_and_whole_characters() {
        let ascii = "abcdefghij".repeat(1_000);
        let wide = "\u{4E2D}".repeat(5_000);
        let accented = "e\u{301}".repeat(5_000);
        // (name, line, scroll, (columns shown, window x, line width))
        type Case<'a> = (&'a str, &'a str, f64, (usize, f64, f64));
        let cases: [Case; 4] = [
            // Columns 0..10 in view: grid steps 0 and 1, then one more.
            ("ascii at the start", &ascii, 0.0, (512, 0.0, 100_000.0)),
            // Columns 9,000..9,010 in view (grid step 35): steps 34..37.
            (
                "ascii scrolled",
                &ascii,
                90_000.0,
                (768, 87_040.0, 100_000.0),
            ),
            ("wide scrolled", &wide, 30_000.0, (768, 25_600.0, 100_000.0)),
            (
                "accent pairs",
                &accented,
                20_000.0,
                (768, 15_360.0, 50_000.0),
            ),
        ];
        for (name, line, scroll, (columns, x, width)) in cases {
            let (text, at, total) = shown(line, scroll);
            let measured: usize = text.chars().map(width_of).sum();
            assert_eq!((measured, at, total), (columns, x, width), "{name}");
            assert!(!text.starts_with('\u{301}'), "{name}: split grapheme");
        }
    }

    fn width_of(c: char) -> usize {
        width(c)
    }
}
