use std::fmt::Write;

use diffy_diff::{BlockKind, ChangeIntensity, DiffDocument, Side};

use crate::check::{Step, tile};

pub fn render(doc: &DiffDocument, fallback: Option<&str>) -> String {
    let mut out = String::new();
    if doc.file_count() == 0 || doc.files().hunks[0].is_empty() {
        out.push_str("no changes\n");
        return out;
    }
    let structural = doc.prefers_structural(0);
    if !structural {
        match fallback {
            Some(reason) => _ = writeln!(out, "line diff ({reason})"),
            None => out.push_str("line diff\n"),
        }
    }
    let (h, b) = (doc.hunks(), doc.blocks());
    for hunk in doc.files().hunks[0].clone() {
        let hi = hunk as usize;
        _ = writeln!(
            out,
            "@@ -{},{} +{},{} @@",
            h.old_start[hi], h.old_len[hi], h.new_start[hi], h.new_len[hi]
        );
        let rows = doc.aligned_rows(hunk);
        for block in h.blocks[hunk as usize].clone() {
            let bi = block as usize;
            let (o, n) = (b.old_line[bi] - 1, b.new_line[bi] - 1);
            let (olds, news) = (o..o + b.old_len[bi], n..n + b.new_len[bi]);
            if b.kind[bi] == BlockKind::Context {
                for (o, n) in olds.zip(news) {
                    row(&mut out, doc, ' ', (Some(o), Some(n)));
                }
                continue;
            }
            let steps = match structural.then(|| tile(rows, olds.clone(), news.clone())) {
                Some(Some(steps)) => steps,
                tiled => {
                    if tiled.is_some() {
                        out.push_str("! rows do not tile this block\n");
                    }
                    olds.map(|o| (Some(o), None))
                        .chain(news.map(|n| (None, Some(n))))
                        .collect()
                }
            };
            for step in steps {
                match step {
                    (Some(o), Some(n)) => {
                        row(&mut out, doc, '<', (Some(o), None));
                        row(&mut out, doc, '>', (None, Some(n)));
                    }
                    (Some(_), None) => row(&mut out, doc, '-', step),
                    (None, Some(_)) => row(&mut out, doc, '+', step),
                    (None, None) => {}
                }
            }
        }
    }
    out
}

fn row(out: &mut String, doc: &DiffDocument, sign: char, (old, new): Step) {
    let number = |line: Option<u32>| line.map_or(String::new(), |l| (l + 1).to_string());
    _ = write!(out, "{sign} {:>4} {:>4} |", number(old), number(new));
    let (side, line) = match (old, new) {
        (Some(o), _) => (Side::Old, o),
        (None, Some(n)) => (Side::New, n),
        (None, None) => return,
    };
    let text = doc.text(0, side).display_line(line).unwrap_or("");
    if sign == ' ' {
        out.push_str(text);
    } else {
        marked(out, text, doc, side, line);
    }
    out.push('\n');
}

fn marked(out: &mut String, text: &str, doc: &DiffDocument, side: Side, line: u32) {
    let mut runs: Vec<(usize, usize, ChangeIntensity)> = Vec::new();
    for span in doc.semantic_spans(0, side, line) {
        if span.intensity == ChangeIntensity::UnchangedContext {
            continue;
        }
        let (start, end) = (span.start as usize, span.end as usize);
        match runs.last_mut() {
            Some(last) if last.1 == start && last.2 == span.intensity => last.1 = end,
            _ => runs.push((start, end, span.intensity)),
        }
    }
    let mut at = 0;
    for (start, end, intensity) in runs {
        let (open, close) = match intensity {
            ChangeIntensity::NovelWord => ('⟨', '⟩'),
            ChangeIntensity::Renamed => ('⟪', '⟫'),
            _ => ('⟦', '⟧'),
        };
        out.push_str(&text[at..start]);
        out.push(open);
        out.push_str(&text[start..end]);
        out.push(close);
        at = end;
    }
    out.push_str(&text[at..]);
}
