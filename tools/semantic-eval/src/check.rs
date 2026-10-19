use std::ops::Range;

use diffy_diff::{AlignedRow, BlockKind, ChangeIntensity, DiffDocument, Side};
use serde::{Deserialize, Serialize};

pub type Step = (Option<u32>, Option<u32>);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub hunks: u32,
    pub changed_old: u32,
    pub changed_new: u32,
    pub changed_bytes: u64,
    pub novel_bytes: u64,
    pub word_bytes: u64,
    pub marked_lines: u32,
    pub fragments: u32,
    pub unmarked_lines: u32,
    pub untiled_blocks: u32,
}

pub struct ChangeBlock {
    pub hunk: u32,
    pub old: Range<u32>,
    pub new: Range<u32>,
}

pub fn change_blocks(doc: &DiffDocument, file: u32) -> Vec<ChangeBlock> {
    let (h, b) = (doc.hunks(), doc.blocks());
    let mut out = Vec::new();
    for hunk in doc.files().hunks[file as usize].clone() {
        for block in h.blocks[hunk as usize].clone() {
            let bi = block as usize;
            if b.kind[bi] != BlockKind::Change {
                continue;
            }
            let old = b.old_line[bi] - 1;
            let new = b.new_line[bi] - 1;
            out.push(ChangeBlock {
                hunk,
                old: old..old + b.old_len[bi],
                new: new..new + b.new_len[bi],
            });
        }
    }
    out
}

pub fn tile(rows: &[AlignedRow], olds: Range<u32>, news: Range<u32>) -> Option<Vec<Step>> {
    let (mut o, mut n) = (olds.start, news.start);
    let mut steps = Vec::new();
    for row in rows {
        let ro = row.old.filter(|line| olds.contains(line));
        let rn = row.new.filter(|line| news.contains(line));
        if ro.is_none() && rn.is_none() {
            continue;
        }
        if ro.is_some_and(|line| line != o) || rn.is_some_and(|line| line != n) {
            return None;
        }
        steps.push((ro, rn));
        o += u32::from(ro.is_some());
        n += u32::from(rn.is_some());
    }
    (o == olds.end && n == news.end).then_some(steps)
}

pub fn check(doc: &DiffDocument) -> (Metrics, Vec<String>) {
    let mut metrics = Metrics::default();
    let mut violations = Vec::new();
    if let Err(error) = doc.verify_integrity() {
        violations.push(format!("integrity: {error}"));
        return (metrics, violations);
    }
    if doc.file_count() == 0 {
        return (metrics, violations);
    }
    let structural = doc.prefers_structural(0);
    let hunks = doc.files().hunks[0].clone();
    metrics.hunks = hunks.len() as u32;
    if structural {
        for hunk in hunks {
            if !monotonic(doc.aligned_rows(hunk)) {
                violations.push(format!("hunk {hunk}: rows are not strictly increasing"));
            }
        }
    }
    for block in change_blocks(doc, 0) {
        metrics.changed_old += block.old.len() as u32;
        metrics.changed_new += block.new.len() as u32;
        if structural
            && tile(
                doc.aligned_rows(block.hunk),
                block.old.clone(),
                block.new.clone(),
            )
            .is_none()
        {
            metrics.untiled_blocks += 1;
            violations.push(format!(
                "rows do not tile change block old {}..{} new {}..{}",
                block.old.start + 1,
                block.old.end + 1,
                block.new.start + 1,
                block.new.end + 1
            ));
        }
        for (side, lines) in [(Side::Old, block.old), (Side::New, block.new)] {
            for line in lines {
                line_metrics(doc, side, line, structural, &mut metrics);
            }
        }
    }
    (metrics, violations)
}

fn monotonic(rows: &[AlignedRow]) -> bool {
    let (mut old, mut new) = (None, None);
    rows.iter().all(|row| {
        let ok = row.old.is_none_or(|l| old.is_none_or(|p| l > p))
            && row.new.is_none_or(|l| new.is_none_or(|p| l > p));
        old = row.old.or(old);
        new = row.new.or(new);
        ok
    })
}

fn line_metrics(doc: &DiffDocument, side: Side, line: u32, structural: bool, m: &mut Metrics) {
    let text = doc.text(0, side).display_line(line).unwrap_or("");
    m.changed_bytes += solid(text) as u64;
    if !structural {
        return;
    }
    let mut marked = false;
    let mut end = None;
    for span in doc.semantic_spans(0, side, line) {
        let bytes = solid(&text[span.start as usize..span.end as usize]) as u64;
        match span.intensity {
            ChangeIntensity::Novel => m.novel_bytes += bytes,
            ChangeIntensity::NovelWord => m.word_bytes += bytes,
            ChangeIntensity::UnchangedContext => continue,
        }
        marked = true;
        let joined = end.is_some_and(|e: u32| solid(&text[e as usize..span.start as usize]) == 0);
        if !joined {
            m.fragments += 1;
        }
        end = Some(span.end);
    }
    if marked {
        m.marked_lines += 1;
    } else if solid(text) > 0 {
        m.unmarked_lines += 1;
    }
}

fn solid(text: &str) -> usize {
    text.bytes().filter(|b| !b.is_ascii_whitespace()).count()
}
