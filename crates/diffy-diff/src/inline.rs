//! Word-level changes inside a pair of changed lines.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::myers::{diff, intern};

/// Lines longer than this get no word diff by default. Long lines are
/// diffed in chunks (see [`paired_inline_diff`]), so this only bounds what
/// a caller chooses to bound: the default matches the 64 MiB a side may
/// hold elsewhere in the diff stack.
pub const MAX_INLINE_LINE_BYTES: usize = crate::limits::DETAIL_SIDE_BYTES;

/// Pairs whose lines together are at most this long are diffed token by
/// token in one go, as they always were; longer pairs take the chunked
/// path.
const DIRECT_PAIR_BYTES: usize = 4_000;

/// Below this share of unchanged bytes on either side the lines count as
/// rewritten by default, and highlighting scattered common words would be
/// noise.
pub const MIN_KEPT_SHARE: f32 = 0.3;

/// Changed byte ranges of an old and a new line. Ranges are sorted,
/// disjoint, on char boundaries, and never touch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InlineDiff {
    pub old: Vec<Range<u32>>,
    pub new: Vec<Range<u32>>,
}

/// The unit an inline diff compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InlineMode {
    /// No inline detail.
    Off,
    /// Unicode words (UAX #29): identifiers, numbers, and CJK runs whole;
    /// punctuation, spaces, and emoji on their own.
    #[default]
    Word,
    /// Extended grapheme clusters: user-visible characters, never
    /// splitting an emoji sequence or a combining mark from its base.
    Grapheme,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineOptions {
    pub mode: InlineMode,
    /// Longer lines get no inline detail.
    pub max_line_bytes: usize,
    /// Pairs keeping less than this share of either line are rewritten.
    pub min_kept_share: f32,
}

impl Default for InlineOptions {
    fn default() -> Self {
        Self {
            mode: InlineMode::Word,
            max_line_bytes: MAX_INLINE_LINE_BYTES,
            min_kept_share: MIN_KEPT_SHARE,
        }
    }
}

/// Why a [`PairedInlineDiff`] has the ranges it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InlineDetail {
    /// The ranges are the changed units; none when the lines are equal.
    Exact,
    /// Too little is shared to be worth highlighting: no ranges.
    Rewritten,
    /// A line exceeds `max_line_bytes`: no ranges.
    Limited,
    /// [`InlineMode::Off`]: no ranges.
    Off,
}

/// An inline diff with the reason behind its ranges, so an omitted
/// highlight never passes for an unchanged pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedInlineDiff {
    pub old: Vec<Range<u32>>,
    pub new: Vec<Range<u32>>,
    pub detail: InlineDetail,
}

/// The words that differ between `old` and `new`. Empty when the lines are
/// equal, too long, or too different to compare word by word.
pub fn inline_diff(old: &str, new: &str) -> InlineDiff {
    let d = paired_inline_diff(old, new, &InlineOptions::default());
    InlineDiff {
        old: d.old,
        new: d.new,
    }
}

/// The units of `options.mode` that differ between `old` and `new`, with
/// the reason when there are none to show. Ranges are UTF-8 byte ranges on
/// unit boundaries, merged across whitespace.
pub fn paired_inline_diff(old: &str, new: &str, options: &InlineOptions) -> PairedInlineDiff {
    let none = |detail| PairedInlineDiff {
        old: Vec::new(),
        new: Vec::new(),
        detail,
    };
    if options.mode == InlineMode::Off {
        return none(InlineDetail::Off);
    }
    if old.len() > options.max_line_bytes || new.len() > options.max_line_bytes {
        return none(InlineDetail::Limited);
    }
    if old == new {
        return none(InlineDetail::Exact);
    }
    let mut out = none(InlineDetail::Exact);
    if old.len() + new.len() <= DIRECT_PAIR_BYTES {
        diff_tokens(
            (old, 0..old.len()),
            (new, 0..new.len()),
            options.mode,
            &mut out,
        );
    } else {
        diff_long(old, new, options.mode, Chunking::DEFAULT, &mut out);
    }
    let changed = |ranges: &[Range<u32>]| ranges.iter().map(|r| r.len()).sum::<usize>();
    let kept = |line: &str, ranges: &[Range<u32>]| {
        if line.is_empty() {
            1.0
        } else {
            1.0 - changed(ranges) as f32 / line.len() as f32
        }
    };
    if kept(old, &out.old) < options.min_kept_share || kept(new, &out.new) < options.min_kept_share
    {
        return none(InlineDetail::Rewritten);
    }
    out
}

/// Diffs `old_part` of `old` against `new_part` of `new` token by token,
/// appending the changed ranges (in whole-line offsets) to `out`.
fn diff_tokens(
    (old, old_part): (&str, Range<usize>),
    (new, new_part): (&str, Range<usize>),
    mode: InlineMode,
    out: &mut PairedInlineDiff,
) {
    let (o, n) = (&old[old_part.clone()], &new[new_part.clone()]);
    let (old_tokens, new_tokens) = (tokens(o, mode), tokens(n, mode));
    let (a, b) = intern(
        old_tokens.iter().map(|t| t.1),
        new_tokens.iter().map(|t| t.1),
    );
    let byte_range = |tokens: &[(usize, &str)], part: &Range<usize>, r: &Range<u32>| {
        let at = |i: u32| tokens.get(i as usize).map_or(part.len(), |t| t.0);
        (part.start + at(r.start)) as u32..(part.start + at(r.end)) as u32
    };
    for change in diff(&a, &b) {
        let (ro, rn) = (
            byte_range(&old_tokens, &old_part, &change.old),
            byte_range(&new_tokens, &new_part, &change.new),
        );
        push_joined(&mut out.old, ro, old);
        push_joined(&mut out.new, rn, new);
    }
}

/// How a long pair is cut into chunks.
#[derive(Debug, Clone, Copy)]
struct Chunking {
    /// A run of changed chunks this long or shorter (both sides together)
    /// is diffed token by token; longer runs are cut again, finer.
    direct: usize,
    /// Mask of the rolling hash that picks cut points: about one cut per
    /// `mask + 1` bytes.
    mask: u64,
}

impl Chunking {
    /// Chunks of about 256 bytes, diffed directly up to 16 KiB.
    const DEFAULT: Self = Self {
        direct: 16 << 10,
        mask: 255,
    };
}

/// Diffs a pair too long to tokenize whole. The common prefix and suffix
/// are cut off at word boundaries; the middles are cut into chunks at
/// content-defined boundaries (a rolling hash of the bytes before a
/// boundary, so an edit moves only its own chunk's cuts), the chunk
/// sequences are diffed like lines, and only runs of changed chunks are
/// diffed token by token. Runs too long for that are cut again with
/// smaller chunks. The work is linear in the lines but for those runs,
/// which the token diff's own budget bounds.
fn diff_long(
    old: &str,
    new: &str,
    mode: InlineMode,
    chunking: Chunking,
    out: &mut PairedInlineDiff,
) {
    let (old_b, new_b) = (old.as_bytes(), new.as_bytes());
    let prefix = old_b.iter().zip(new_b).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old_b
        .iter()
        .rev()
        .zip(new_b.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    // Cuts strictly inside the common parts, where both lines read alike
    // on both sides of the cut.
    let start = prefix_cut(old, prefix);
    let end_back = suffix_cut(old, suffix);
    let (old_mid, new_mid) = (start..old.len() - end_back, start..new.len() - end_back);
    // (old range, new range) of changed runs still to diff, in order.
    let mut runs = vec![(old_mid, new_mid, chunking.mask)];
    let mut i = 0;
    while i < runs.len() {
        let (o, n, mask) = runs[i].clone();
        if o.len() + n.len() <= chunking.direct || mask == 0 {
            i += 1;
            continue;
        }
        let (oc, nc) = (cuts(&old[o.clone()], mask), cuts(&new[n.clone()], mask));
        let (a, b) = intern(
            oc.windows(2).map(|w| &old[o.start + w[0]..o.start + w[1]]),
            nc.windows(2).map(|w| &new[n.start + w[0]..n.start + w[1]]),
        );
        let finer: Vec<_> = diff(&a, &b)
            .into_iter()
            .map(|c| {
                let os = o.start + oc[c.old.start as usize]..o.start + oc[c.old.end as usize];
                let ns = n.start + nc[c.new.start as usize]..n.start + nc[c.new.end as usize];
                (os, ns, mask >> 2)
            })
            .collect();
        runs.splice(i..=i, finer);
    }
    for (o, n, _) in runs {
        if o.is_empty() || n.is_empty() {
            // One side gained or lost whole chunks: they are the change.
            push_joined(&mut out.old, o.start as u32..o.end as u32, old);
            push_joined(&mut out.new, n.start as u32..n.end as u32, new);
            continue;
        }
        diff_tokens((old, o), (new, n), mode, out);
    }
}

/// Whether a cut between bytes `i - 1` and `i` of `line` is a boundary of
/// every unit mode: both bytes ASCII, and one of them punctuation that
/// UAX #29 never joins to a neighbor (not `_`, `.`, `:`, `,`, `;`, `'`,
/// which join letters or digits, nor space, which joins space). Both ends
/// of the line count.
fn safe_cut(line: &[u8], i: usize) -> bool {
    if i == 0 || i >= line.len() {
        return true;
    }
    let (a, b) = (line[i - 1], line[i]);
    let breaker = |c: u8| b"(){}[]<>=+-*/%&|!?^~#$@\\\"`".contains(&c);
    a.is_ascii() && b.is_ascii() && (breaker(a) || breaker(b))
}

/// The last safe cut at or before `at`, strictly inside the common prefix
/// (the bytes on both sides of it belong to both lines) or at 0.
fn prefix_cut(line: &str, prefix: usize) -> usize {
    let bytes = line.as_bytes();
    (1..prefix).rev().find(|&i| safe_cut(bytes, i)).unwrap_or(0)
}

/// How many bytes from the end the first safe cut inside the common suffix
/// of length `suffix` lies (0 when there is none). Measured from the end,
/// it is the same cut in both lines.
fn suffix_cut(line: &str, suffix: usize) -> usize {
    let bytes = line.as_bytes();
    let len = bytes.len();
    (1..suffix)
        .rev()
        .find(|&back| safe_cut(bytes, len - back))
        .unwrap_or(0)
}

/// Chunk boundaries of `text`: 0, each safe cut where a rolling hash of the
/// preceding bytes has its `mask` bits clear, and the end.
fn cuts(text: &str, mask: u64) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut out = vec![0];
    let mut hash = 0u64;
    for (i, &byte) in bytes.iter().enumerate() {
        // A gear hash: each byte's influence shifts out after 64 bytes.
        hash = (hash << 1).wrapping_add(GEAR[byte as usize]);
        let at = i + 1;
        if hash & mask == 0 && at < bytes.len() && safe_cut(bytes, at) {
            out.push(at);
        }
    }
    out.push(bytes.len());
    out
}

/// Random 64-bit values per byte for [`cuts`], fixed so chunking is
/// deterministic.
static GEAR: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut i = 0;
    while i < 256 {
        // splitmix64
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
};

fn tokens(line: &str, mode: InlineMode) -> Vec<(usize, &str)> {
    match mode {
        InlineMode::Grapheme => line.grapheme_indices(true).collect(),
        _ => line.split_word_bound_indices().collect(),
    }
}

/// Appends `range` unless empty, merging it into the previous range when
/// only whitespace separates them, so `a b` replaced by `c d` reads as one
/// change.
fn push_joined(ranges: &mut Vec<Range<u32>>, range: Range<u32>, line: &str) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = ranges.last_mut() {
        let between = &line[last.end as usize..range.start as usize];
        if between.chars().all(char::is_whitespace) {
            last.end = range.end;
            return;
        }
    }
    ranges.push(range);
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use proptest::prelude::*;
    use unicode_segmentation::UnicodeSegmentation;

    use super::{
        Chunking, InlineDetail, InlineMode, InlineOptions, PairedInlineDiff, diff_long,
        inline_diff, paired_inline_diff,
    };

    /// Each side with changed ranges in brackets.
    fn marked(old: &str, new: &str) -> String {
        let d = inline_diff(old, new);
        bracket(old, new, &d.old, &d.new)
    }

    fn marked_by(old: &str, new: &str, mode: InlineMode) -> String {
        let options = InlineOptions {
            mode,
            ..InlineOptions::default()
        };
        let d = paired_inline_diff(old, new, &options);
        bracket(old, new, &d.old, &d.new)
    }

    fn bracket(
        old: &str,
        new: &str,
        old_ranges: &[Range<u32>],
        new_ranges: &[Range<u32>],
    ) -> String {
        let mark = |line: &str, ranges: &[std::ops::Range<u32>]| {
            let mut out = String::new();
            let mut at = 0;
            for r in ranges {
                out.push_str(&line[at..r.start as usize]);
                out.push('[');
                out.push_str(&line[r.start as usize..r.end as usize]);
                out.push(']');
                at = r.end as usize;
            }
            out.push_str(&line[at..]);
            out
        };
        format!("{} | {}", mark(old, old_ranges), mark(new, new_ranges))
    }

    #[test]
    fn changed_words_are_bracketed_on_both_sides() {
        let cases = [
            (
                "let old = 1;",
                "let new = 1;",
                "let [old] = 1; | let [new] = 1;",
            ),
            ("foo(a, b)", "foo(a, b, c)", "foo(a, b) | foo(a, b[, c])"),
            (
                "x = alpha beta;",
                "x = gamma delta;",
                "x = [alpha beta]; | x = [gamma delta];",
            ),
            ("naïve café", "naïve cafés", "naïve [café] | naïve [cafés]"),
            ("say 👋 hi", "say 🎉 hi", "say [👋] hi | say [🎉] hi"),
            ("same", "same", "same | same"),
            // Too different: no word highlights.
            (
                "fn parse(input)",
                "struct Token {",
                "fn parse(input) | struct Token {",
            ),
        ];
        for (old, new, expected) in cases {
            assert_eq!(marked(old, new), expected, "{old} -> {new}");
        }
    }

    #[test]
    fn grapheme_mode_marks_whole_user_visible_characters() {
        let family_a = "the family emoji \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} here";
        let family_b = "the family emoji \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F466} here";
        let cases = [
            (
                "let count = 1;",
                "let counter = 1;",
                "let count = 1; | let count[er] = 1;",
            ),
            ("colour", "color", "colo[u]r | color"),
            // The accent is a combining mark: it changes with its base.
            ("cafe\u{301}", "cafe", "caf[e\u{301}] | caf[e]"),
        ];
        for (old, new, expected) in cases {
            assert_eq!(
                marked_by(old, new, InlineMode::Grapheme),
                expected,
                "{old} -> {new}"
            );
        }
        let marked = marked_by(family_a, family_b, InlineMode::Grapheme);
        assert_eq!(
            marked,
            "the family emoji [\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}] here | \
             the family emoji [\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F466}] here"
        );
    }

    #[test]
    fn a_pair_without_ranges_says_why() {
        // Past a cap the caller lowered, detail is reported as limited.
        let long = "x".repeat(2_001);
        let cases = [
            ("a b", "a c", InlineMode::Off, InlineDetail::Off),
            (long.as_str(), "x", InlineMode::Word, InlineDetail::Limited),
            (
                "fn parse(input)",
                "struct Token {",
                InlineMode::Word,
                InlineDetail::Rewritten,
            ),
            ("same", "same", InlineMode::Grapheme, InlineDetail::Exact),
        ];
        for (old, new, mode, expected) in cases {
            let options = InlineOptions {
                mode,
                max_line_bytes: 2_000,
                ..InlineOptions::default()
            };
            let d = paired_inline_diff(old, new, &options);
            assert_eq!(
                (d.detail, d.old.len() + d.new.len()),
                (expected, 0),
                "{mode:?}"
            );
        }
    }

    /// The text of each changed range, old side then new.
    fn range_texts(old: &str, new: &str, d: &PairedInlineDiff) -> (Vec<String>, Vec<String>) {
        let texts = |line: &str, ranges: &[Range<u32>]| {
            ranges
                .iter()
                .map(|r| line[r.start as usize..r.end as usize].to_owned())
                .collect()
        };
        (texts(old, &d.old), texts(new, &d.new))
    }

    // Catches long lines losing their word highlights (they used to stop at
    // 2,000 bytes) or the chunked path marking whole chunks: two small
    // edits in a 200 KB line mark just their words.
    #[test]
    fn a_long_line_marks_only_its_edited_words() {
        let old: String = (0..20_000).map(|i| format!("call({i});")).collect();
        let new = old
            .replace("call(500);", "call(five);")
            .replace("call(12000);", "call(12000, x);");
        let d = paired_inline_diff(&old, &new, &InlineOptions::default());

        assert_eq!(d.detail, InlineDetail::Exact);
        assert_eq!(
            range_texts(&old, &new, &d),
            (
                vec!["500".to_owned()],
                vec!["five".to_owned(), ", x".to_owned()]
            )
        );
    }

    fn line() -> impl Strategy<Value = String> {
        let unit = prop::sample::select(vec![
            "a",
            "ab",
            " ",
            ",",
            "\u{e9}",
            "e\u{301}",
            "\u{1F468}\u{200D}\u{1F469}",
            "\u{1F44B}",
        ]);
        prop::collection::vec(unit, 0..12).prop_map(|units| units.concat())
    }

    proptest! {
        #[test]
        fn inline_ranges_are_ordered_apart_and_on_grapheme_boundaries(
            old in line(),
            new in line(),
            grapheme in any::<bool>(),
        ) {
            let mode = if grapheme { InlineMode::Grapheme } else { InlineMode::Word };
            let options = InlineOptions { mode, min_kept_share: 0.0, ..InlineOptions::default() };
            let d = paired_inline_diff(&old, &new, &options);
            for (line, ranges) in [(&old, &d.old), (&new, &d.new)] {
                let bounds: Vec<usize> = line
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain([line.len()])
                    .collect();
                let mut end = None;
                for r in ranges {
                    prop_assert!(r.start < r.end);
                    prop_assert!(end.is_none_or(|e| e < r.start), "{:?} touch", ranges);
                    prop_assert!(bounds.contains(&(r.start as usize)));
                    prop_assert!(bounds.contains(&(r.end as usize)));
                    end = Some(r.end);
                }
            }
        }

        // Catches chunk cuts that split a unit or ranges that overlap
        // across chunks: with tiny chunks every pair takes the chunked path,
        // and its ranges must still be ordered, apart, and on boundaries.
        #[test]
        fn chunked_ranges_are_ordered_apart_and_on_grapheme_boundaries(
            stem in line(),
            old_tail in line(),
            new_tail in line(),
            grapheme in any::<bool>(),
        ) {
            let mode = if grapheme { InlineMode::Grapheme } else { InlineMode::Word };
            let filler = "(x)=[y]+{z};".repeat(8);
            let old = format!("{filler}{stem}{old_tail}{filler}{stem}");
            let new = format!("{filler}{new_tail}{stem}{filler}{old_tail}");
            let mut d = PairedInlineDiff { old: Vec::new(), new: Vec::new(), detail: InlineDetail::Exact };
            diff_long(&old, &new, mode, Chunking { direct: 0, mask: 3 }, &mut d);
            for (line, ranges) in [(&old, &d.old), (&new, &d.new)] {
                let bounds: Vec<usize> = line
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .chain([line.len()])
                    .collect();
                let mut end = None;
                for r in ranges {
                    prop_assert!(r.start < r.end);
                    prop_assert!(end.is_none_or(|e| e < r.start), "{:?} touch", ranges);
                    prop_assert!(bounds.contains(&(r.start as usize)));
                    prop_assert!(bounds.contains(&(r.end as usize)));
                    end = Some(r.end);
                }
            }
            if old == new {
                prop_assert!(d.old.is_empty() && d.new.is_empty());
            }
        }
    }
}
