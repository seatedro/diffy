//! Read-only literal search over the sources the view holds.
//!
//! Search reads text stores, not rendered rows, so it finds lines inside
//! collapsed context, and moving to a match expands just enough context to
//! show it. By default it searches the new side, ignores case, and includes
//! unchanged lines. A file the view holds only as patch lines is searched
//! where those lines reach, and the summary says so. Matches are kept as
//! byte ranges of lines, never copies of the text, up to
//! [`FindOptions::max_matches`]; past that the count is a lower bound.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use diffy_diff::{BlockKind, DiffDocument, Side};

use super::DiffViewState;
use super::navigation::{RevealAlign, SourcePoint};
use super::prepared::{RowPaint, SearchMark};
use super::state::source_line;

/// Which sides a search reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SearchSides {
    Old,
    #[default]
    New,
    /// Old and new occurrences are separate matches.
    Both,
}

impl SearchSides {
    fn has(self, side: Side) -> bool {
        match self {
            Self::Old => side == Side::Old,
            Self::New => side == Side::New,
            Self::Both => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FindOptions {
    pub sides: SearchSides,
    pub case_sensitive: bool,
    /// Search unchanged lines too, not only added and removed ones.
    pub include_unchanged: bool,
    /// Most matches kept; past it the count reads "at least".
    pub max_matches: usize,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            sides: SearchSides::New,
            case_sensitive: false,
            include_unchanged: true,
            max_matches: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchDirection {
    Forward,
    Backward,
}

/// How much of the searched files' text the view holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SearchCoverage {
    /// Every searched file is held whole.
    #[default]
    Full,
    /// Some file is held only as its patch's lines: matches cover the
    /// available hunks.
    PatchOnly,
}

/// The state of the current search.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SearchSummary {
    pub matches: u32,
    pub old_matches: u32,
    pub new_matches: u32,
    /// The search stopped at the match limit: there are at least
    /// `matches`.
    pub at_least: bool,
    /// Zero-based index of the match last moved to.
    pub active: Option<u32>,
    pub coverage: SearchCoverage,
}

/// One match: a byte range of a store line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    seg: u32,
    generation: u64,
    file: u32,
    side: Side,
    index: u32,
    range: Range<u32>,
}

impl Hit {
    fn order(&self) -> (u32, u32, Side, u32) {
        (self.seg, self.file, self.side, self.index)
    }

    fn same(&self, other: &Self) -> bool {
        (
            self.generation,
            self.file,
            self.side,
            self.index,
            &self.range,
        ) == (
            other.generation,
            other.file,
            other.side,
            other.index,
            &other.range,
        )
    }
}

#[derive(Debug, Default)]
pub(crate) struct SearchState {
    query: String,
    options: FindOptions,
    hits: Vec<Hit>,
    at_least: bool,
    coverage: SearchCoverage,
    active: Option<usize>,
    complete: HashSet<u64>,
}

/// Calls `push` with each non-overlapping occurrence of `query` in
/// `line`, until it returns false.
fn find_in(
    line: &str,
    query: &str,
    case_sensitive: bool,
    mut push: impl FnMut(Range<usize>) -> bool,
) {
    if query.is_empty() {
        return;
    }
    if case_sensitive {
        for (at, _) in line.match_indices(query) {
            if !push(at..at + query.len()) {
                return;
            }
        }
    } else if query.is_ascii() {
        // ASCII bytes never occur inside a multibyte sequence, so every
        // match starts and ends on a char boundary.
        let (hay, needle) = (line.as_bytes(), query.as_bytes());
        let mut at = 0;
        while at + needle.len() <= hay.len() {
            if hay[at..at + needle.len()].eq_ignore_ascii_case(needle) {
                if !push(at..at + needle.len()) {
                    return;
                }
                at += needle.len();
            } else {
                at += 1;
            }
        }
    } else {
        // Fold both to lowercase, mapping folded bytes back to the
        // original char each came from.
        let mut folded = String::with_capacity(line.len());
        let mut origin = Vec::with_capacity(line.len() + 1);
        for (at, c) in line.char_indices() {
            for lower in c.to_lowercase() {
                let start = folded.len();
                folded.push(lower);
                origin.extend(std::iter::repeat_n(at, folded.len() - start));
            }
        }
        origin.push(line.len());
        let needle: String = query.chars().flat_map(char::to_lowercase).collect();
        for (at, _) in folded.match_indices(&needle) {
            let end = at + needle.len();
            // The end maps to the char after the last matched one.
            let end = origin[end..]
                .iter()
                .copied()
                .find(|&o| o > origin[end - 1])
                .unwrap_or(line.len());
            if !push(origin[at]..end) {
                return;
            }
        }
    }
}

/// Store lines of `file`'s `side` a search reads.
fn searched_lines(doc: &DiffDocument, file: u32, side: Side, unchanged: bool) -> Vec<Range<u32>> {
    if unchanged {
        return std::iter::once(0..doc.text(file, side).line_count()).collect();
    }
    let (h, b) = (doc.hunks(), doc.blocks());
    doc.files().hunks[file as usize]
        .clone()
        .flat_map(|hunk| h.blocks[hunk as usize].clone())
        .filter(|&block| b.kind[block as usize] == BlockKind::Change)
        .map(|block| {
            let bi = block as usize;
            match side {
                Side::Old => b.old_store[bi]..b.old_store[bi] + b.old_len[bi],
                Side::New => b.new_store[bi]..b.new_store[bi] + b.new_len[bi],
            }
        })
        .collect()
}

impl DiffViewState {
    /// Searches for `query` (literal text), replacing any earlier search.
    /// An empty query clears it.
    pub fn set_find_query(&mut self, query: &str, options: FindOptions) {
        if query == self.search.query && options == self.search.options {
            return;
        }
        self.search.query = query.to_owned();
        self.search.options = options;
        self.search.active = None;
        self.search.hits.clear();
        self.search.complete.clear();
        self.rerun_search();
    }

    /// Runs the current search again over the current sources, keeping the
    /// active match when it still exists.
    pub(crate) fn rerun_search(&mut self) {
        self.revision += 1;
        if self.search.query.is_empty() {
            self.search.hits.clear();
            self.search.complete.clear();
            self.search.active = None;
            self.search.at_least = false;
            self.search.coverage = SearchCoverage::Full;
            return;
        }
        let active = self.search.active.map(|i| self.search.hits[i].clone());
        let (query, options) = (&self.search.query, self.search.options);
        let limit = options.max_matches;
        let scanned = std::mem::take(&mut self.search.complete);
        let mut kept: HashMap<u64, Vec<Hit>> = HashMap::new();
        for hit in std::mem::take(&mut self.search.hits) {
            if scanned.contains(&hit.generation) {
                kept.entry(hit.generation).or_default().push(hit);
            }
        }
        let mut complete = HashSet::new();
        let mut hits = Vec::new();
        let mut at_least = false;
        let mut coverage = SearchCoverage::Full;
        'search: for (seg, segment) in self.segments.iter().enumerate() {
            let doc = &segment.doc;
            let generation = segment.generation;
            if (0..doc.file_count()).any(|file| doc.files().partial[file as usize]) {
                coverage = SearchCoverage::PatchOnly;
            }
            if scanned.contains(&generation) {
                for mut hit in kept.remove(&generation).unwrap_or_default() {
                    if hits.len() >= limit {
                        at_least = true;
                        break 'search;
                    }
                    hit.seg = seg as u32;
                    hits.push(hit);
                }
                complete.insert(generation);
                continue;
            }
            for file in 0..doc.file_count() {
                for side in [Side::Old, Side::New] {
                    if !options.sides.has(side) {
                        continue;
                    }
                    let store = doc.text(file, side);
                    for lines in searched_lines(doc, file, side, options.include_unchanged) {
                        for index in lines {
                            let line = store.display_line(index).unwrap_or("");
                            find_in(line, query, options.case_sensitive, |range| {
                                if hits.len() >= limit {
                                    at_least = true;
                                    return false;
                                }
                                hits.push(Hit {
                                    seg: seg as u32,
                                    generation,
                                    file,
                                    side,
                                    index,
                                    range: range.start as u32..range.end as u32,
                                });
                                true
                            });
                            if at_least {
                                break 'search;
                            }
                        }
                    }
                }
            }
            complete.insert(generation);
        }
        self.search.active = active.and_then(|a| hits.iter().position(|h| h.same(&a)));
        self.search.hits = hits;
        self.search.at_least = at_least;
        self.search.coverage = coverage;
        self.search.complete = complete;
    }

    pub fn search_summary(&self) -> SearchSummary {
        let s = &self.search;
        let old = s.hits.iter().filter(|h| h.side == Side::Old).count() as u32;
        SearchSummary {
            matches: s.hits.len() as u32,
            old_matches: old,
            new_matches: s.hits.len() as u32 - old,
            at_least: s.at_least,
            active: s.active.map(|i| i as u32),
            coverage: s.coverage,
        }
    }

    /// Moves to the next or previous match, wrapping around, expanding
    /// context to show it and scrolling it into view. Returns where it is.
    pub fn next_match(&mut self, direction: SearchDirection) -> Option<SourcePoint> {
        let count = self.search.hits.len();
        if count == 0 {
            return None;
        }
        let next = match (self.search.active, direction) {
            (None, SearchDirection::Forward) => 0,
            (None, SearchDirection::Backward) => count - 1,
            (Some(i), SearchDirection::Forward) => (i + 1) % count,
            (Some(i), SearchDirection::Backward) => (i + count - 1) % count,
        };
        self.search.active = Some(next);
        self.revision += 1;
        let hit = self.search.hits[next].clone();
        let seg = hit.seg as usize;
        let unit = self.segments[seg].unit(hit.file);
        self.unfold(unit);
        self.reveal_line(seg, hit.file, hit.side, hit.index);
        if let Some(index) = self.list_index_of_line(unit, hit.side, hit.index) {
            self.reveal_list_row(index);
        }
        self.reveal_byte(seg, hit.file, hit.side, hit.index, hit.range.start as usize);
        let segment = &self.segments[seg];
        Some(SourcePoint {
            file: self.file_id(unit),
            side: hit.side,
            line: source_line(&segment.doc, hit.file, hit.side, hit.index),
            byte: hit.range.start,
        })
    }

    fn reveal_list_row(&mut self, index: u32) {
        self.scroll_to_index(index, RevealAlign::Nearest);
        self.set_focus(Some(index));
    }

    /// Matches on each side of a line row, for its paint. Unified context
    /// rows show the new side's text, so old-side matches on them paint
    /// there too.
    pub(crate) fn search_marks(
        &self,
        seg: usize,
        row: u32,
        paint: &RowPaint,
    ) -> [Vec<SearchMark>; 2] {
        let mut marks: [Vec<SearchMark>; 2] = Default::default();
        let hits = &self.search.hits;
        if hits.is_empty() {
            return marks;
        }
        let p = &self.segments[seg].projection;
        let file = p.file[row as usize];
        for side in [Side::Old, Side::New] {
            let Some(index) = p.line(row, side) else {
                continue;
            };
            let slot = if paint.sides[side as usize].is_some() {
                side
            } else if paint.sides[Side::New as usize].is_some() {
                Side::New
            } else {
                continue;
            };
            // Line bytes the layout covers: all of a short line, the window
            // of a long one.
            let (start, len) = paint.sides[slot as usize].as_ref().map_or((0, 0), |l| {
                (l.window.map_or(0, |w| w.start), l.layout.text().len())
            });
            let key = (seg as u32, file, side, index);
            let first = hits.partition_point(|h| h.order() < key);
            for (i, hit) in hits[first..].iter().enumerate() {
                if hit.order() != key {
                    break;
                }
                let (from, to) = (hit.range.start as usize, hit.range.end as usize);
                let range =
                    from.clamp(start, start + len) - start..to.clamp(start, start + len) - start;
                if range.start < range.end && !marks[slot as usize].iter().any(|m| m.range == range)
                {
                    marks[slot as usize].push(SearchMark {
                        range,
                        active: self.search.active == Some(first + i),
                    });
                }
            }
        }
        marks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found<'a>(line: &'a str, query: &str, case_sensitive: bool) -> Vec<&'a str> {
        let mut out = Vec::new();
        find_in(line, query, case_sensitive, |r| {
            out.push(&line[r]);
            true
        });
        out
    }

    // Catches case folding that shifts byte ranges or splits characters:
    // each match is the original text it covers.
    #[test]
    fn literal_matches_cover_the_original_text() {
        let cases: &[(&str, &str, bool, &[&str])] = &[
            ("Foo foo FOO", "foo", false, &["Foo", "foo", "FOO"]),
            ("Foo foo FOO", "foo", true, &["foo"]),
            ("aaaa", "aa", false, &["aa", "aa"]),
            ("é ÉCOLE école", "école", false, &["ÉCOLE", "école"]),
            ("İx ix", "ix", false, &["ix"]),
            ("日本語 Ünïcode", "ünï", false, &["Ünï"]),
        ];
        for &(line, query, case, expected) in cases {
            assert_eq!(found(line, query, case), expected, "{line:?} / {query:?}");
        }
    }
}
