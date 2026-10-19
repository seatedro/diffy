use proptest::prelude::*;

use crate::{
    BlockKind, Comparison, ComparisonOptions, ContextLen, ContextPolicy, DiffDocument, Expansion,
    FileSources, GapId, HydrationError, Mode, PairingMode, PatchError, Projection, Reveal, RowKind,
    Side, SourceCoverage, TextStore, WhitespaceMode, apply, diff_texts, parse_unified,
    write_unified,
};

/// Files, hunks, and lines of `doc` as text: a summary line per file, then
/// each hunk's header and lines with their diff tag. A line that ends its
/// store without a newline is marked `$`.
fn dump(doc: &DiffDocument) -> String {
    let mut out = String::new();
    let (h, b) = (doc.hunks(), doc.blocks());
    for s in doc.summaries() {
        let from = s
            .old_path
            .as_deref()
            .map(|p| format!("{p} -> "))
            .unwrap_or_default();
        let binary = if s.binary { " binary" } else { "" };
        out.push_str(&format!(
            "{from}{} {}{binary} +{} -{}\n",
            s.path,
            s.status.name(),
            s.additions,
            s.deletions
        ));
        let (old, new) = (doc.text(s.index, Side::Old), doc.text(s.index, Side::New));
        let line = |out: &mut String, tag: char, store: &crate::TextStore, i: u32| {
            let eol = if i + 1 == store.line_count() && store.no_newline_at_eof() {
                "$"
            } else {
                ""
            };
            out.push_str(&format!("  {tag}{}{eol}\n", store.line(i).unwrap()));
        };
        for hunk in doc.files().hunks[s.index as usize].clone() {
            let hi = hunk as usize;
            out.push_str(&format!(
                "  @@ -{},{} +{},{} @@{}\n",
                h.old_start[hi], h.old_len[hi], h.new_start[hi], h.new_len[hi], h.section[hi]
            ));
            for block in h.blocks[hi].clone() {
                let bi = block as usize;
                let (os, ns) = (b.old_store[bi], b.new_store[bi]);
                match b.kind[bi] {
                    BlockKind::Context => {
                        (0..b.old_len[bi]).for_each(|k| line(&mut out, ' ', old, os + k));
                    }
                    BlockKind::Change => {
                        (0..b.old_len[bi]).for_each(|k| line(&mut out, '-', old, os + k));
                        (0..b.new_len[bi]).for_each(|k| line(&mut out, '+', new, ns + k));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn parser_reads_git_and_plain_patches() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "modified with section and context",
            "diff --git a/src/lib.rs b/src/lib.rs\n\
             index 1111111..2222222 100644\n\
             --- a/src/lib.rs\n\
             +++ b/src/lib.rs\n\
             @@ -10,3 +10,3 @@ fn main() {\n \
             a\n\
             -b\n\
             +c\n \
             d\n",
            "src/lib.rs modified +1 -1\n  @@ -10,3 +10,3 @@fn main() {\n   a\n  -b\n  +c\n   d\n",
        ),
        (
            "rename without content change",
            "diff --git a/old name.txt b/new name.txt\n\
             similarity index 100%\n\
             rename from old name.txt\n\
             rename to new name.txt\n",
            "old name.txt -> new name.txt renamed +0 -0\n",
        ),
        (
            "rename with a hunk",
            "diff --git a/a.rs b/b.rs\n\
             similarity index 90%\n\
             rename from a.rs\n\
             rename to b.rs\n\
             --- a/a.rs\n\
             +++ b/b.rs\n\
             @@ -1 +1 @@\n\
             -x\n\
             +y\n",
            "a.rs -> b.rs renamed +1 -1\n  @@ -1,1 +1,1 @@\n  -x\n  +y\n",
        ),
        (
            "binary markers, both kinds",
            "diff --git a/logo.png b/logo.png\n\
             index 1111111..2222222 100644\n\
             Binary files a/logo.png and b/logo.png differ\n\
             diff --git a/icon.ico b/icon.ico\n\
             new file mode 100644\n\
             GIT binary patch\n\
             literal 4\n\
             LcmZQzWMT#Y01f~L\n\
             \n\
             literal 0\n\
             HcmV?d00001\n",
            "logo.png modified binary +0 -0\nicon.ico added binary +0 -0\n",
        ),
        (
            "no newline at end of file on both sides",
            "--- a/a.txt\n\
             +++ b/a.txt\n\
             @@ -1,2 +1,2 @@\n \
             keep\n\
             -old\n\
             \\ No newline at end of file\n\
             +new\n\
             \\ Kein Zeilenumbruch am Dateiende\n",
            "a.txt modified +1 -1\n  @@ -1,2 +1,2 @@\n   keep\n  -old$\n  +new$\n",
        ),
        (
            "plain diff -u, two files, new and deleted",
            "--- /dev/null\t2024-01-01 00:00:00\n\
             +++ b/new.txt\t2024-01-01 00:00:00\n\
             @@ -0,0 +1,2 @@\n\
             +one\n\
             +two\n\
             --- a/gone.txt\n\
             +++ /dev/null\n\
             @@ -1 +0,0 @@\n\
             -bye\n",
            "new.txt added +2 -0\n  @@ -0,0 +1,2 @@\n  +one\n  +two\n\
             gone.txt deleted +0 -1\n  @@ -1,1 +0,0 @@\n  -bye\n",
        ),
        (
            "a removed line that looks like a header stays in the hunk",
            "--- a/x\n\
             +++ b/x\n\
             @@ -1,2 +1 @@\n\
             --- a/y\n\
             -+++ b/y\n\
             +z\n",
            "x modified +1 -2\n  @@ -1,2 +1,1 @@\n  --- a/y\n  -+++ b/y\n  +z\n",
        ),
    ];
    for (name, patch, expected) in cases {
        let doc = parse_unified(patch).unwrap_or_else(|e| panic!("{name}: {e}"));
        doc.verify_integrity().unwrap();
        assert_eq!(dump(&doc), *expected, "{name}");
    }
}

#[test]
fn parser_rejects_malformed_hunks() {
    let cases: &[(&str, PatchError)] = &[
        ("", PatchError::Empty),
        (
            "--- a/x\n+++ b/x\n@@ -1 +1 @\n",
            PatchError::InvalidHunkHeader { line: 3 },
        ),
        (
            "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n a\n",
            PatchError::TruncatedHunk { line: 5 },
        ),
        (
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n*a\n",
            PatchError::MalformedHunkLine { line: 4 },
        ),
        (
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n\u{e9}a\n",
            PatchError::MalformedHunkLine { line: 4 },
        ),
        (
            "--- a/x\n+++ b/x\n@@ -5 +5 @@\n-a\n+b\n@@ -1 +1 @@\n-c\n+d\n",
            PatchError::HunkOutOfOrder { line: 6 },
        ),
        // A no-newline marker after an empty line cannot happen.
        (
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-\n\\ No newline\n+a\n",
            PatchError::MalformedHunkLine { line: 5 },
        ),
    ];
    for (patch, expected) in cases {
        assert_eq!(
            parse_unified(patch).err().as_ref(),
            Some(expected),
            "{patch:?}"
        );
    }
}

/// Text from a small alphabet of lines, so diffs have common lines.
fn text() -> impl Strategy<Value = String> {
    let line = prop::sample::select(vec!["a", "b", "c", "fn x() {", "}", "", "\tz\r"]);
    (prop::collection::vec(line, 0..30), any::<bool>()).prop_map(|(lines, eol)| {
        let mut text = lines.join("\n");
        if eol && !text.is_empty() {
            text.push('\n');
        }
        text
    })
}

proptest! {
    #[test]
    fn a_written_patch_parses_back_and_applies_to_the_old_text(
        old in text(),
        new in text(),
        context in 0u32..4,
    ) {
        let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), context);
        doc.verify_integrity().unwrap();
        let patch = write_unified(&doc);
        let parsed = parse_unified(&patch).unwrap();
        parsed.verify_integrity().unwrap();
        prop_assert_eq!(apply(&parsed, 0, &old).unwrap(), new.clone());
        prop_assert_eq!(apply(&doc, 0, &old).unwrap(), new);
        prop_assert_eq!(write_unified(&parsed), patch);
    }

    #[test]
    fn projections_cover_every_line_once_when_fully_expanded(
        old in text(),
        new in text(),
        split in any::<bool>(),
    ) {
        let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1);
        let mut expansion = Expansion::new(&doc);
        for hunk in 0..doc.hunk_count() {
            expansion.reveal(&doc, GapId { file: 0, hunk: Some(hunk) }, Reveal::All, 0);
        }
        expansion.reveal(&doc, GapId { file: 0, hunk: None }, Reveal::All, 0);
        let mode = if split { Mode::Split } else { Mode::Unified };
        let p = Projection::new(&doc, mode, &expansion);
        p.verify_integrity(&doc).unwrap();
        prop_assert!(!p.kind.contains(&RowKind::Gap));
        for side in [Side::Old, Side::New] {
            let lines = doc.text(0, side).line_count();
            let shown = (0..p.len()).filter(|&r| p.line(r, side).is_some()).count();
            prop_assert_eq!(shown as u32, lines);
        }
    }
}

/// Projection rows as text: kind tag, old and new line numbers, text.
fn rows(doc: &DiffDocument, p: &Projection) -> String {
    let mut out = String::new();
    for r in 0..p.len() {
        let side_text = |side| {
            p.line(r, side)
                .map(|i| (i + 1, doc.text(p.file[r as usize], side).line(i).unwrap()))
        };
        let (old, new) = (side_text(Side::Old), side_text(Side::New));
        let num = |l: Option<(u32, &str)>| l.map_or("-".to_owned(), |(n, _)| n.to_string());
        let line = match p.kind[r as usize] {
            RowKind::FileHeader => format!("file {}", doc.path(p.file[r as usize])),
            RowKind::HunkHeader => "@@".to_owned(),
            RowKind::Gap => format!("... {} hidden", p.gap(r).unwrap().hidden),
            kind => {
                let tag = match kind {
                    RowKind::Removed => '-',
                    RowKind::Added => '+',
                    RowKind::Modified => '~',
                    _ => ' ',
                };
                let text = match (old, new) {
                    (Some((_, o)), Some((_, n))) if o != n => format!("{o} | {n}"),
                    (Some((_, t)), _) | (_, Some((_, t))) => t.to_owned(),
                    _ => String::new(),
                };
                format!("{tag} {} {} {text}", num(old), num(new))
            }
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn numbered(lines: std::ops::Range<u32>) -> String {
    lines.map(|i| format!("l{i}\n")).collect()
}

#[test]
fn side_by_side_pairs_changed_lines_and_pads_the_shorter_side() {
    let old = "a\nb\nc\nd\n";
    let new = "a\nB\nC\nC2\nd\n";
    let doc = diff_texts(Some("f"), Some("f"), Some(old), Some(new), 1);
    let p = Projection::new(&doc, Mode::Split, &Expansion::new(&doc));
    assert_eq!(
        rows(&doc, &p),
        "file f\n  1 1 a\n~ 2 2 b | B\n~ 3 3 c | C\n+ - 4 C2\n  4 5 d\n"
    );
    let p = Projection::new(&doc, Mode::Unified, &Expansion::new(&doc));
    assert_eq!(
        rows(&doc, &p),
        "file f\n  1 1 a\n- 2 - b\n- 3 - c\n+ - 2 B\n+ - 3 C\n+ - 4 C2\n  4 5 d\n"
    );
}

#[test]
fn expanding_a_gap_reveals_lines_from_the_chosen_end() {
    // Changes at lines 2 and 20 of 30, one line of context.
    let old = numbered(1..31);
    let new = old.replace("l2\n", "x2\n").replace("l20\n", "x20\n");
    let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1);
    let mut expansion = Expansion::new(&doc);
    let shown = |e: &Expansion| {
        let p = Projection::new(&doc, Mode::Unified, e);
        p.verify_integrity(&doc).unwrap();
        rows(&doc, &p)
            .lines()
            .filter(|l| l.starts_with("...") || l.starts_with("  "))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(
        shown(&expansion),
        "  1 1 l1,  3 3 l3,... 15 hidden,  19 19 l19,  21 21 l21,... 9 hidden"
    );
    let between = GapId {
        file: 0,
        hunk: Some(1),
    };
    assert!(expansion.reveal(&doc, between, Reveal::Up, 2));
    assert!(expansion.reveal(&doc, between, Reveal::Down, 1));
    assert_eq!(
        shown(&expansion),
        "  1 1 l1,  3 3 l3,  4 4 l4,... 12 hidden,  17 17 l17,  18 18 l18,  19 19 l19,  21 21 l21,... 9 hidden"
    );
    // Revealing all but a few hidden lines shows the rest too.
    assert!(expansion.reveal(&doc, between, Reveal::Down, 10));
    assert!(!shown(&expansion).contains("... 12"));
    assert!(!expansion.reveal(&doc, between, Reveal::Down, 1));
}

#[test]
fn file_list_reports_paths_status_and_stats() {
    let mut doc = diff_texts(
        Some("a.rs"),
        Some("b.rs"),
        Some("x\ny\n"),
        Some("x\nz\nw\n"),
        3,
    );
    doc.append(diff_texts(None, Some("new.md"), None, Some("hi\n"), 3));
    doc.verify_integrity().unwrap();
    let list: Vec<String> = doc
        .summaries()
        .map(|s| {
            format!(
                "{:?} {} {} +{} -{}",
                s.old_path,
                s.path,
                s.status.name(),
                s.additions,
                s.deletions
            )
        })
        .collect();
    assert_eq!(
        list,
        [
            r#"Some("a.rs") b.rs renamed +2 -1"#,
            "None new.md added +1 -0"
        ]
    );
    assert_eq!(doc.totals(), (3, 1));
}

#[test]
fn a_hundred_thousand_line_diff_projects_every_row() {
    let old = numbered(0..100_000);
    let new = old.replace("l500\n", "x500\n");
    let doc = diff_texts(Some("big"), Some("big"), Some(&old), Some(&new), 3);
    let mut expansion = Expansion::new(&doc);
    expansion.reveal(
        &doc,
        GapId {
            file: 0,
            hunk: Some(0),
        },
        Reveal::All,
        0,
    );
    expansion.reveal(
        &doc,
        GapId {
            file: 0,
            hunk: None,
        },
        Reveal::All,
        0,
    );
    let p = Projection::new(&doc, Mode::Unified, &expansion);
    // A header, 99,999 unchanged lines, and the changed pair.
    assert_eq!(p.len(), 100_002);
    assert_eq!(p.row_of(0, Side::New, 99_999), Some(100_001));
}

/// Diff times of large inputs. Run with `--ignored --nocapture` in release
/// mode for meaningful numbers.
#[test]
#[ignore = "measurement, prints a report"]
fn report_diff_times() {
    let old = numbered(0..100_000);
    let sparse: String = old
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i % 100 == 7 {
                format!("edit {i}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    let rewritten: String = (0..20_000).map(|i| format!("other {i}\n")).collect();
    let small_old = numbered(0..20_000);
    let shuffled: String = (0..20_000)
        .map(|i| format!("l{}\n", (i * 7_919) % 20_000))
        .collect();
    for (name, a, b) in [
        ("100k lines, 1% edited", &old, &sparse),
        ("20k lines, all rewritten", &small_old, &rewritten),
        ("20k lines, reordered", &small_old, &shuffled),
    ] {
        let started = std::time::Instant::now();
        let doc = diff_texts(Some("f"), Some("f"), Some(a), Some(b), 3);
        let p = Projection::new(&doc, Mode::Split, &Expansion::new(&doc));
        eprintln!(
            "{name}: {:?}, {} hunks, {} rows",
            started.elapsed(),
            doc.hunk_count(),
            p.len()
        );
    }
}

// Regression: a file with full text but no hunks (identical sides) skipped
// its trailing gap, so expanding it showed none of its lines.
#[test]
fn an_unchanged_file_expands_to_all_of_its_lines() {
    let doc = diff_texts(Some("f"), Some("f"), Some("a\nb\n"), Some("a\nb\n"), 1);
    let mut expansion = Expansion::new(&doc);
    expansion.reveal(
        &doc,
        GapId {
            file: 0,
            hunk: None,
        },
        Reveal::All,
        0,
    );
    let p = Projection::new(&doc, Mode::Unified, &expansion);
    let shown = (0..p.len())
        .filter(|&r| p.line(r, Side::New).is_some())
        .count();
    assert_eq!(shown, 2);
}

/// Rows of `old` -> `new` with change blocks aligned per `options`.
fn compared(old: &str, new: &str, mode: Mode, options: ComparisonOptions) -> String {
    let doc = diff_texts(Some("f"), Some("f"), Some(old), Some(new), 0);
    let comparison = Comparison::new(&doc, options);
    let p = Projection::with_comparison(&doc, mode, &Expansion::new(&doc), &comparison);
    p.verify_integrity(&doc).unwrap();
    rows(&doc, &p)
}

#[test]
fn similarity_pairing_shifts_an_unequal_change_onto_the_lines_it_resembles() {
    let similar = ComparisonOptions::default();
    let positional = ComparisonOptions {
        pairing: PairingMode::Positional,
        ..similar
    };
    let cases = [
        (
            "a line inserted above an edited one",
            "return total;\n",
            "log(total);\nreturn total + tax;\n",
            similar,
            "file f\n+ - 1 log(total);\n~ 1 2 return total; | return total + tax;\n",
        ),
        (
            "positional mode keeps the old pairing",
            "return total;\n",
            "log(total);\nreturn total + tax;\n",
            positional,
            "file f\n~ 1 1 return total; | log(total);\n+ - 2 return total + tax;\n",
        ),
        (
            "an import removed above edited ones",
            "use a::x;\nuse b::{y, z};\nuse c::w;\n",
            "use b::{y};\nuse c::{w, v};\n",
            similar,
            "file f\n- 1 - use a::x;\n~ 2 1 use b::{y, z}; | use b::{y};\n~ 3 2 use c::w; | use c::{w, v};\n",
        ),
        (
            "unrelated lines stay positional",
            "alpha\n",
            "one\ntwo\n",
            similar,
            "file f\n~ 1 1 alpha | one\n+ - 2 two\n",
        ),
        (
            "an ambiguous shift stays positional",
            "x = 1;\n",
            "x = 2;\nx = 3;\n",
            similar,
            "file f\n~ 1 1 x = 1; | x = 2;\n+ - 2 x = 3;\n",
        ),
    ];
    for (name, old, new, options, expected) in cases {
        assert_eq!(compared(old, new, Mode::Split, options), expected, "{name}");
    }
}

#[test]
fn unified_rows_of_a_shifted_pair_point_at_their_partner() {
    let doc = diff_texts(
        Some("f"),
        Some("f"),
        Some("return total;\n"),
        Some("log(total);\nreturn total + tax;\n"),
        0,
    );
    let comparison = Comparison::new(&doc, ComparisonOptions::default());
    let p = Projection::with_comparison(&doc, Mode::Unified, &Expansion::new(&doc), &comparison);
    let pairs: Vec<_> = (0..p.len())
        .map(|r| p.line_pair(r).map(|pair| (pair.old, pair.new)))
        .collect();
    // Header, the removed line, then the inserted and the edited line.
    assert_eq!(pairs, [None, Some((0, 1)), None, Some((0, 1))]);
}

// Catches the pairing budget staying fixed (it used to give up past 4,096
// comparisons per change): 1 old line against 5,000 new means 5,000 shifts
// to score, and the old line still finds the new line it resembles.
#[test]
fn similarity_pairing_scales_its_budget_with_the_change() {
    let new: String = (0..5_000).map(|i| format!("n{i}\n")).collect();
    let doc = diff_texts(Some("f"), Some("f"), Some("n4999 edited\n"), Some(&new), 0);
    let comparison = Comparison::new(&doc, ComparisonOptions::default());
    let p = Projection::with_comparison(&doc, Mode::Split, &Expansion::new(&doc), &comparison);
    let modified = (0..p.len()).find(|&r| p.kind[r as usize] == RowKind::Modified);
    assert_eq!(modified.and_then(|r| p.line(r, Side::New)), Some(4_999));
    assert_eq!(comparison.limited_pairings(0), 0);
}

#[test]
fn whitespace_policies_hide_only_space_and_tab_changes() {
    use WhitespaceMode::*;
    let modes = [Exact, IgnoreEdgeSpace, IgnoreSpaceChange, IgnoreAllSpace];
    // Hidden line pairs under each mode, in the order above.
    let cases: &[(&str, &str, &str, [u32; 4])] = &[
        ("leading indentation", "  a\n", "a\n", [0, 1, 0, 1]),
        ("trailing space", "a \t\n", "a\n", [0, 1, 1, 1]),
        ("internal run of spaces", "a   b\n", "a b\n", [0, 0, 1, 1]),
        ("tab for a space", "a\tb\n", "a b\n", [0, 0, 1, 1]),
        ("space added inside", "ab\n", "a b\n", [0, 0, 0, 1]),
        ("no-break space", "a\u{a0}b\n", "a b\n", [0, 0, 0, 0]),
        ("crlf", "a\r\n", "a\n", [0, 0, 0, 0]),
        ("missing final newline", "a", "a\n", [0, 0, 0, 0]),
    ];
    for (name, old, new, expected) in cases {
        let doc = diff_texts(Some("f"), Some("f"), Some(old), Some(new), 3);
        let hidden = modes.map(|whitespace| {
            let options = ComparisonOptions {
                whitespace,
                ..ComparisonOptions::default()
            };
            Comparison::new(&doc, options).hidden_whitespace_changes(0)
        });
        assert_eq!(hidden, *expected, "{name}");
    }
}

#[test]
fn a_hidden_whitespace_change_shows_as_context_between_real_changes() {
    let options = ComparisonOptions {
        whitespace: WhitespaceMode::IgnoreAllSpace,
        ..ComparisonOptions::default()
    };
    let (old, new) = ("x\n  a\ny\n", "X\na\nY\n");
    assert_eq!(
        compared(old, new, Mode::Unified, options),
        "file f\n- 1 - x\n+ - 1 X\n  2 2   a | a\n- 3 - y\n+ - 3 Y\n"
    );
    assert_eq!(
        compared(old, new, Mode::Split, options),
        "file f\n~ 1 1 x | X\n  2 2   a | a\n~ 3 3 y | Y\n"
    );
}

/// Pairs of texts as a patch of their diff, parsed back: a patch-only
/// document of `old` -> `new`.
fn patch_of(old: &str, new: &str, context: u32) -> DiffDocument {
    let doc = diff_texts(Some("f"), Some("f"), Some(old), Some(new), context);
    parse_unified(&write_unified(&doc)).unwrap()
}

fn sources(old: &str, new: &str) -> FileSources {
    FileSources {
        old: Some(TextStore::new(old)),
        new: Some(TextStore::new(new)),
    }
}

/// Up to three edits (replace, insert, or delete a line) of `lines`.
fn edited(lines: Vec<String>) -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec((0u8..3, any::<prop::sample::Index>()), 0..3).prop_map(move |edits| {
        let mut lines = lines.clone();
        for (i, (kind, at)) in edits.into_iter().enumerate() {
            let at = at.index(lines.len() + 1);
            match kind {
                0 if at < lines.len() => lines[at] = format!("x{i}"),
                1 => lines.insert(at, format!("y{i}")),
                _ if at < lines.len() => drop(lines.remove(at)),
                _ => {}
            }
        }
        lines
    })
}

/// A file, an edit of it, and an edit of that (often none), with long
/// unchanged runs between edits so patches have several hunks and gaps.
fn revisions() -> impl Strategy<Value = (String, String, String)> {
    // The last edit may also flip the final newline.
    (
        0usize..40,
        any::<bool>(),
        any::<bool>(),
        prop::bool::weighted(0.2),
    )
        .prop_flat_map(|(n, crlf, eol, flip)| {
            let old: Vec<String> = (0..n).map(|i| format!("l{i}")).collect();
            (Just(old.clone()), edited(old), Just((crlf, eol, flip)))
        })
        .prop_flat_map(|(old, new, ends)| (Just(old), Just(new.clone()), edited(new), Just(ends)))
        .prop_map(|(old, new, other, (crlf, eol, flip))| {
            let join = |lines: Vec<String>, eol: bool| {
                let sep = if crlf { "\r\n" } else { "\n" };
                let mut text = lines.join(sep);
                if eol && !text.is_empty() {
                    text.push_str(sep);
                }
                text
            };
            (join(old, eol), join(new, eol), join(other, eol != flip))
        })
}

proptest! {
    // The patch and the old text determine the new text, so hydration
    // must accept exactly the real new text and refuse every other.
    #[test]
    fn hydration_accepts_only_the_sources_the_patch_came_from(
        (old, new, other) in revisions(),
        context in 0u32..3,
    ) {
        let patch = patch_of(&old, &new, context);
        let hydrated = patch.hydrate_file(0, sources(&old, &other));
        prop_assert_eq!(hydrated.is_ok(), other == new, "{:?}", hydrated.err());
    }

    #[test]
    fn a_hydrated_patch_expands_to_the_whole_file_and_exports_the_same_patch(
        (old, new, _) in revisions(),
    ) {
        let patch = patch_of(&old, &new, 1);
        let doc = patch.hydrate_file(0, sources(&old, &new)).unwrap();
        prop_assert_eq!(doc.coverage(0), SourceCoverage::Full);
        let mut expansion = Expansion::new(&doc);
        expansion.reveal_all(&doc);
        let p = Projection::new(&doc, Mode::Unified, &expansion);
        let shown: String = (0..p.len())
            .filter_map(|r| p.line(r, Side::New))
            .map(|i| format!("{}\n", doc.text(0, Side::New).line(i).unwrap()))
            .collect();
        let store = TextStore::new(new.as_str());
        let expected: String = (0..store.line_count())
            .map(|i| format!("{}\n", store.line(i).unwrap()))
            .collect();
        prop_assert_eq!(shown, expected);
        prop_assert_eq!(write_unified(&doc), write_unified(&patch));
    }

    #[test]
    fn comparisons_cover_every_changed_line_once_in_order(
        old in text(),
        new in text(),
        whitespace in prop::sample::select(vec![
            WhitespaceMode::Exact,
            WhitespaceMode::IgnoreEdgeSpace,
            WhitespaceMode::IgnoreSpaceChange,
            WhitespaceMode::IgnoreAllSpace,
        ]),
        similar in any::<bool>(),
        split in any::<bool>(),
    ) {
        let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1);
        let pairing = if similar { PairingMode::Similarity } else { PairingMode::Positional };
        let comparison = Comparison::new(&doc, ComparisonOptions { whitespace, pairing });
        prop_assert_eq!(comparison.verify_integrity(&doc), Ok(()));
        let mut expansion = Expansion::new(&doc);
        expansion.reveal_all(&doc);
        let mode = if split { Mode::Split } else { Mode::Unified };
        let p = Projection::with_comparison(&doc, mode, &expansion, &comparison);
        prop_assert_eq!(p.verify_integrity(&doc), Ok(()));
        for side in [Side::Old, Side::New] {
            let shown = (0..p.len()).filter(|&r| p.line(r, side).is_some()).count();
            prop_assert_eq!(shown as u32, doc.text(0, side).line_count());
        }
    }
}

#[test]
fn hydration_names_why_it_refuses_sources() {
    let added = parse_unified("--- /dev/null\n+++ b/n\n@@ -0,0 +1 @@\n+hi\n").unwrap();
    let binary =
        parse_unified("diff --git a/p.png b/p.png\nBinary files a/p.png and b/p.png differ\n")
            .unwrap();
    let modified = patch_of("a\nb\n", "a\nc\n", 1);
    let cases = [
        (
            &added,
            sources("", "hi\n"),
            HydrationError::UnexpectedSide(Side::Old),
        ),
        (
            &modified,
            FileSources {
                old: Some(TextStore::new("a\nb\n")),
                new: None,
            },
            HydrationError::MissingSide(Side::New),
        ),
        (&binary, sources("x", "y"), HydrationError::Binary),
        (
            &modified,
            sources("a\nb\n", "a\nd\n"),
            HydrationError::Mismatch {
                side: Side::New,
                line: 1,
            },
        ),
        (
            &modified,
            sources("a\nb", "a\nc"),
            HydrationError::Eof(Side::Old),
        ),
        (
            &patch_of("a\nb\n", "a\n", 0),
            sources("a\nb\n", "a"),
            HydrationError::Eof(Side::New),
        ),
    ];
    for (doc, sources, expected) in cases {
        assert_eq!(doc.hydrate_file(0, sources).err(), Some(expected));
    }
    let hydrated = added
        .hydrate_file(
            0,
            FileSources {
                old: None,
                new: Some(TextStore::new("hi\n")),
            },
        )
        .unwrap();
    assert_eq!(hydrated.coverage(0), SourceCoverage::Full);
}

#[test]
fn patch_only_gaps_know_their_length_above_hunks_but_not_after_the_last() {
    let old = numbered(1..31);
    let new = old.replace("l5\n", "x5\n").replace("l20\n", "x20\n");
    let patch = patch_of(&old, &new, 1);
    let gap = |hunk| GapId { file: 0, hunk };
    assert_eq!(patch.context_len(gap(Some(0))), ContextLen::Lines(3));
    assert_eq!(patch.context_len(gap(Some(1))), ContextLen::Lines(12));
    assert_eq!(patch.context_len(gap(None)), ContextLen::Unknown);
    let full = patch.hydrate_file(0, sources(&old, &new)).unwrap();
    assert_eq!(full.context_len(gap(None)), ContextLen::Lines(9));
}

#[test]
fn a_zero_min_hidden_policy_keeps_a_one_line_gap_collapsed() {
    let old = numbered(1..6);
    let new = old.replace("l1\n", "x1\n").replace("l5\n", "x5\n");
    let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1);
    let gaps = |policy| {
        let p = Projection::new(&doc, Mode::Unified, &Expansion::with_policy(&doc, policy));
        rows(&doc, &p)
            .lines()
            .filter(|l| l.starts_with("..."))
            .count()
    };
    assert_eq!(gaps(ContextPolicy::default()), 0);
    let keep = ContextPolicy {
        min_hidden: 0,
        ..ContextPolicy::default()
    };
    assert_eq!(gaps(keep), 1);
}

#[test]
fn collapsing_a_gap_hides_its_revealed_lines_again() {
    let old = numbered(1..31);
    let new = old.replace("l2\n", "x2\n").replace("l20\n", "x20\n");
    let doc = diff_texts(Some("f"), Some("f"), Some(&old), Some(&new), 1);
    let mut expansion = Expansion::new(&doc);
    let between = GapId {
        file: 0,
        hunk: Some(1),
    };
    let collapsed = Projection::new(&doc, Mode::Unified, &expansion).len();
    expansion.reveal(&doc, between, Reveal::Down, 5);
    assert!(expansion.collapse(between));
    assert_eq!(
        Projection::new(&doc, Mode::Unified, &expansion).len(),
        collapsed
    );
    assert_eq!(expansion.hidden(&doc, between), 15);
}

#[test]
fn file_facts_report_changes_that_have_no_lines() {
    let patch = "diff --git a/run.sh b/run.sh\n\
                 old mode 100644\n\
                 new mode 100755\n\
                 diff --git a/a.txt b/b.txt\n\
                 similarity index 100%\n\
                 rename from a.txt\n\
                 rename to b.txt\n\
                 diff --git a/c.txt b/d.txt\n\
                 similarity index 100%\n\
                 copy from c.txt\n\
                 copy to d.txt\n\
                 diff --git a/logo.png b/logo.png\n\
                 Binary files a/logo.png and b/logo.png differ\n\
                 diff --git a/e.txt b/e.txt\n\
                 --- a/e.txt\n\
                 +++ b/e.txt\n\
                 @@ -1 +1 @@\n\
                 -x\n\
                 +x\n\
                 \\ No newline at end of file\n";
    let doc = parse_unified(patch).unwrap();
    let facts: Vec<String> = (0..doc.file_count())
        .map(|f| {
            let facts = doc.facts(f);
            format!(
                "{} {}{}{}{}{}",
                doc.path(f),
                facts.status.name(),
                if facts.binary { " binary" } else { "" },
                facts
                    .mode_change
                    .map(|(o, n)| format!(" mode {o}->{n}"))
                    .unwrap_or_default(),
                if facts.has_hunks { " hunks" } else { "" },
                match (facts.old_missing_newline, facts.new_missing_newline) {
                    (false, true) => " newline removed",
                    (true, false) => " newline added",
                    _ => "",
                },
            )
        })
        .collect();
    assert_eq!(
        facts,
        [
            "run.sh modified mode 100644->100755",
            "b.txt renamed",
            "d.txt copied",
            "logo.png modified binary",
            "e.txt modified hunks newline removed",
        ]
    );
}

/// Time and peak memory of diffing the 64 MiB fixtures. Run in the test
/// profile (optimized) with `--ignored --nocapture`.
#[test]
#[ignore = "measurement, prints a report"]
fn report_large_inputs() {
    use crate::fixtures::{
        edit_line, every_nth, javascript, minified, one_block, peak_rss, repetitive,
        reset_peak_rss, scattered_edits,
    };
    const MIB: usize = 1 << 20;
    let size = std::env::var("DIFFY_DIFF_FIXTURE_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
        * MIB;
    // A little under the size, so edits keep the new side within it.
    let base = javascript(1, size - size / 64);
    let line = minified(2, size - size / 64);
    let repeated = repetitive(3, size - size / 64);
    type Make<'a> = Box<dyn Fn() -> (String, String) + 'a>;
    let cases: [(&str, Make); 4] = [
        (
            "1% scattered",
            Box::new(|| (base.clone(), scattered_edits(&base, 4, 10))),
        ),
        (
            "one huge block",
            Box::new(|| (base.clone(), one_block(&base, 5, 0.5))),
        ),
        (
            "minified line",
            Box::new(|| (line.clone(), edit_line(&line, 6, 5))),
        ),
        (
            "many small hunks",
            Box::new(|| (repeated.clone(), every_nth(&repeated, 7, 8))),
        ),
    ];
    let mib = |b: u64| b as f64 / MIB as f64;
    for (name, make) in cases {
        let (old, new) = make();
        eprintln!(
            "{name}: {:.1} MiB -> {:.1} MiB",
            mib(old.len() as u64),
            mib(new.len() as u64)
        );
        reset_peak_rss();
        let before = peak_rss().unwrap_or(0);
        let started = std::time::Instant::now();
        let doc = diff_texts(Some("f.js"), Some("f.js"), Some(&old), Some(&new), 3);
        let diffed = started.elapsed();
        let comparison = Comparison::new(&doc, ComparisonOptions::default());
        let compared = started.elapsed() - diffed;
        let peak = peak_rss().unwrap_or(0);
        let edits: u32 = doc.files().additions[0] + doc.files().deletions[0];
        if std::env::var_os("DIFFY_DIFF_COMPARE_PLAIN").is_some() {
            use crate::compute::line_changes_with;
            use crate::myers::Budget;
            let (a, b) = (TextStore::new(old.as_str()), TextStore::new(new.as_str()));
            let started = std::time::Instant::now();
            let plain = line_changes_with(&a, &b, Budget::UNLIMITED);
            let took = started.elapsed();
            let ours = line_changes_with(&a, &b, Budget::LINEAR);
            let count = |c: &[crate::myers::Change]| -> usize {
                c.iter().map(|c| c.old.len() + c.new.len()).sum()
            };
            eprintln!(
                "  plain search {took:.2?}: {} edits; budgeted: {} edits; identical: {}",
                count(&plain),
                count(&ours),
                plain == ours
            );
        }
        if !old.contains('\n') {
            reset_peak_rss();
            let started = std::time::Instant::now();
            let words = crate::paired_inline_diff(&old, &new, &crate::InlineOptions::default());
            eprintln!(
                "  inline diff of the line pair: {:.2?}, {:?}, {} + {} ranges, peak RSS {:.0} MiB",
                started.elapsed(),
                words.detail,
                words.old.len(),
                words.new.len(),
                mib(peak_rss().unwrap_or(0)),
            );
        }
        eprintln!(
            "{name}: diff {diffed:.2?}, comparison {compared:.2?}, {} hunks, {edits} lines \
             changed, {} pairings limited, peak RSS {:.0} MiB (+{:.0} over inputs)",
            doc.hunk_count(),
            comparison.limited_pairings(0),
            mib(peak),
            mib(peak.saturating_sub(before)),
        );
    }
}

fn reflow_doc(
    old_marks: &[crate::SemanticSpan],
    new_marks: &[(u32, crate::SemanticSpan)],
) -> DiffDocument {
    use crate::{AlignedRow, DocumentBuilder, FileMeta};
    let old = TextStore::new("a();\ncall(x, y);\nb();\n");
    let new = TextStore::new("a();\ncall(\n    x,\n    y,\n);\nb();\n");
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(FileMeta::default(), SourceCoverage::Full, old, new);
    file.structural(true);
    file.hunk(0, 0).context(1).change(1, 4).context(1);
    for (old, new) in [
        (Some(0), Some(0)),
        (Some(1), Some(1)),
        (None, Some(2)),
        (None, Some(3)),
        (None, Some(4)),
        (Some(2), Some(5)),
    ] {
        file.row(AlignedRow { old, new });
    }
    for (line, span) in new_marks {
        file.spans(Side::New, *line, &[*span]);
    }
    if !old_marks.is_empty() {
        file.spans(Side::Old, 1, old_marks);
    }
    file.finish().unwrap();
    builder.build()
}

fn novel(start: u32, end: u32) -> crate::SemanticSpan {
    crate::SemanticSpan {
        start,
        end,
        intensity: crate::ChangeIntensity::Novel,
    }
}

fn unified_kinds(doc: &DiffDocument) -> Vec<RowKind> {
    let comparison = Comparison::new(doc, ComparisonOptions::default());
    let p = Projection::with_comparison(doc, Mode::Unified, &Expansion::new(doc), &comparison);
    p.verify_integrity(doc).unwrap();
    (0..p.len())
        .map(|r| p.kind[r as usize])
        .filter(|k| k.is_line())
        .collect()
}

#[test]
fn a_reflowed_call_shows_only_its_new_lines() {
    use RowKind::{Context, Reflowed};
    let comma = [(3, novel(5, 6))];
    assert_eq!(
        unified_kinds(&reflow_doc(&[], &comma)),
        [Context, Reflowed, Context]
    );
}

#[test]
fn a_reflow_that_also_removes_a_token_keeps_the_old_line() {
    let kinds = unified_kinds(&reflow_doc(&[novel(8, 9)], &[(3, novel(5, 6))]));
    assert!(kinds.contains(&RowKind::Removed));
    assert!(!kinds.contains(&RowKind::Reflowed));
}

#[test]
fn new_code_inside_a_reflow_is_not_folded() {
    let marks = [(2, novel(4, 6)), (3, novel(4, 6))];
    assert!(!unified_kinds(&reflow_doc(&[], &marks)).contains(&RowKind::Reflowed));
}
