use std::ops::Range;

use crate::model::Side;
use crate::semantic::{ChangeIntensity, SemanticSpan};

pub const MAX_MERGE_BYTES: usize = 400;

pub const MIN_MERGE_KEPT: f32 = 0.4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePiece {
    pub side: Side,
    pub range: Range<u32>,
    pub changed: bool,
    pub renamed: bool,
}

pub fn merge_pair(
    old: &str,
    new: &str,
    old_spans: &[SemanticSpan],
    new_spans: &[SemanticSpan],
) -> Option<Vec<MergePiece>> {
    if old.len() > MAX_MERGE_BYTES || new.len() > MAX_MERGE_BYTES {
        return None;
    }
    let (old_marks, new_marks) = (marked(old_spans), marked(new_spans));
    let (old_kept, new_kept) = (kept(old, &old_marks), kept(new, &new_marks));
    if old_kept.len() != new_kept.len()
        || old_kept
            .iter()
            .zip(&new_kept)
            .any(|((_, a), (_, b))| a != b)
    {
        return None;
    }
    let renamed = renamed(new_spans);
    let renamed_len: usize = renamed.iter().map(|r| solid(&new[r.clone()])).sum();
    let longest = solid(old).max(solid(new));
    if longest > 0 && ((new_kept.len() + renamed_len) as f32) < MIN_MERGE_KEPT * longest as f32 {
        return None;
    }
    let mut removed: Vec<(usize, Range<usize>)> = Vec::new();
    for range in old_marks
        .into_iter()
        .filter(|r| !renamed_at(old_spans, r.start))
    {
        let index = old_kept.partition_point(|&(at, _)| at < range.start);
        match removed.last_mut() {
            Some((last, prev))
                if *last == index && old[prev.end..range.start].trim().is_empty() =>
            {
                prev.end = range.end;
            }
            _ => removed.push((index, range)),
        }
    }
    let mut pieces = Vec::new();
    let mut at = 0;
    for (index, range) in removed {
        let after = match index.checked_sub(1).and_then(|i| new_kept.get(i)) {
            Some(&(byte, c)) => byte + c.len_utf8(),
            None => 0,
        };
        let insert = skip_space(new, after).max(at);
        push_new(&mut pieces, at..insert, &new_marks, &renamed);
        pieces.push(MergePiece {
            side: Side::Old,
            range: range.start as u32..range.end as u32,
            changed: true,
            renamed: false,
        });
        at = insert;
    }
    push_new(&mut pieces, at..new.len(), &new_marks, &renamed);
    Some(pieces)
}

fn renamed(spans: &[SemanticSpan]) -> Vec<Range<usize>> {
    spans
        .iter()
        .filter(|span| span.intensity == ChangeIntensity::Renamed)
        .map(|span| span.start as usize..span.end as usize)
        .collect()
}

fn renamed_at(spans: &[SemanticSpan], at: usize) -> bool {
    spans
        .iter()
        .any(|s| s.intensity == ChangeIntensity::Renamed && s.start as usize == at)
}

fn marked(spans: &[SemanticSpan]) -> Vec<Range<usize>> {
    spans
        .iter()
        .filter(|span| span.intensity != ChangeIntensity::UnchangedContext)
        .map(|span| span.start as usize..span.end as usize)
        .collect()
}

fn kept(text: &str, marks: &[Range<usize>]) -> Vec<(usize, char)> {
    let mut mark = 0;
    text.char_indices()
        .filter(|&(at, c)| {
            while marks.get(mark).is_some_and(|m| m.end <= at) {
                mark += 1;
            }
            !c.is_whitespace() && !marks.get(mark).is_some_and(|m| m.contains(&at))
        })
        .collect()
}

fn solid(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn skip_space(text: &str, from: usize) -> usize {
    from + text[from..].len() - text[from..].trim_start().len()
}

fn push_new(
    pieces: &mut Vec<MergePiece>,
    range: Range<usize>,
    marks: &[Range<usize>],
    renamed: &[Range<usize>],
) {
    let mut at = range.start;
    for mark in marks {
        let (start, end) = (mark.start.max(range.start), mark.end.min(range.end));
        if start >= end {
            continue;
        }
        if at < start {
            pieces.push(new_piece(at..start, false, false));
        }
        let rename = renamed.iter().any(|r| r.start == mark.start);
        pieces.push(new_piece(start..end, true, rename));
        at = end;
    }
    if at < range.end {
        pieces.push(new_piece(at..range.end, false, false));
    }
}

fn new_piece(range: Range<usize>, changed: bool, renamed: bool) -> MergePiece {
    MergePiece {
        side: Side::New,
        range: range.start as u32..range.end as u32,
        changed,
        renamed,
    }
}

#[cfg(test)]
mod tests {
    use super::{MergePiece, merge_pair};
    use crate::model::Side;
    use crate::semantic::{ChangeIntensity, SemanticSpan};

    fn novel(start: u32, end: u32) -> SemanticSpan {
        SemanticSpan {
            start,
            end,
            intensity: ChangeIntensity::Novel,
        }
    }

    fn render(old: &str, new: &str, pieces: &[MergePiece]) -> String {
        pieces
            .iter()
            .map(|p| {
                let text = match p.side {
                    Side::Old => &old[p.range.start as usize..p.range.end as usize],
                    Side::New => &new[p.range.start as usize..p.range.end as usize],
                };
                match (p.side, p.changed) {
                    (Side::Old, _) => format!("[-{text}-]"),
                    (Side::New, true) => format!("{{+{text}+}}"),
                    (Side::New, false) => text.to_owned(),
                }
            })
            .collect()
    }

    #[test]
    fn removed_pieces_sit_where_they_were() {
        let (old, new) = ("    limit = 10;", "        limit = 20;");
        let pieces = merge_pair(old, new, &[novel(12, 14)], &[novel(16, 18)]).unwrap();
        assert_eq!(render(old, new, &pieces), "        limit = [-10-]{+20+};");
        let (old, new) = ("step(first, second);", "step(first, extra, second);");
        let pieces = merge_pair(old, new, &[], &[novel(12, 18)]).unwrap();
        assert_eq!(render(old, new, &pieces), "step(first, {+extra,+} second);");
        let (old, new) = ("call(a, b)", "call(a)");
        let pieces = merge_pair(old, new, &[novel(6, 7), novel(8, 9)], &[]).unwrap();
        assert_eq!(render(old, new, &pieces), "call(a[-, b-])");
    }

    #[test]
    fn pairs_that_do_not_line_up_stay_apart() {
        assert!(merge_pair("let a = 1;", "let b = 1;", &[], &[novel(4, 5)]).is_none());
        assert!(merge_pair("first()", "second()", &[novel(0, 7)], &[novel(0, 8)]).is_none());
    }
}
