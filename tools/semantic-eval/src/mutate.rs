use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

use diffy::core::compare::semantic::semantic_language;
use diffy_diff::{
    ChangeIntensity, Comparison, ComparisonOptions, DiffDocument, Expansion, Mode, Projection,
    RowKind, Side,
};
use tree_sitter as ts;

use crate::check::{self, Step, change_blocks, tile};
use crate::engine::{self, Engine};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, value_delimiter = ',', default_value = "rs")]
    ext: Vec<String>,
    #[arg(long)]
    max_files: Option<usize>,
    #[arg(long, default_value_t = 2)]
    per_file: usize,
    #[arg(long, value_delimiter = ',')]
    kinds: Vec<String>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 20)]
    show: usize,
    #[arg(long)]
    rev: Option<String>,
    filter: Option<String>,
}

pub(crate) struct Syntax {
    pub(crate) functions: &'static [&'static str],
    pub(crate) block: &'static str,
    arguments: &'static str,
    wrap: (&'static str, Option<&'static str>),
}

pub(crate) fn syntax(language: &str) -> Option<Syntax> {
    Some(match language {
        "rust" => Syntax {
            functions: &["function_item"],
            block: "block",
            arguments: "arguments",
            wrap: ("if true {", Some("}")),
        },
        "go" => Syntax {
            functions: &["function_declaration", "method_declaration"],
            block: "block",
            arguments: "argument_list",
            wrap: ("if true {", Some("}")),
        },
        "c" | "cpp" => Syntax {
            functions: &["function_definition"],
            block: "compound_statement",
            arguments: "argument_list",
            wrap: ("if (1) {", Some("}")),
        },
        "javascript" | "typescript" | "tsx" => Syntax {
            functions: &["function_declaration", "method_definition"],
            block: "statement_block",
            arguments: "arguments",
            wrap: ("if (true) {", Some("}")),
        },
        "python" => Syntax {
            functions: &["function_definition", "decorated_definition"],
            block: "block",
            arguments: "argument_list",
            wrap: ("if True:", None),
        },
        _ => return None,
    })
}

#[derive(Clone)]
enum Expect {
    Rename {
        from: String,
        to: String,
    },
    Move,
    Reflow,
    Wrap {
        side: Side,
        outer: Vec<u32>,
        inner: Range<u32>,
        at: u32,
    },
}

impl Expect {
    fn reversed(&self) -> Self {
        match self {
            Self::Rename { from, to } => Self::Rename {
                from: to.clone(),
                to: from.clone(),
            },
            Self::Wrap {
                side,
                outer,
                inner,
                at,
            } => Self::Wrap {
                side: match side {
                    Side::Old => Side::New,
                    Side::New => Side::Old,
                },
                outer: outer.clone(),
                inner: inner.clone(),
                at: *at,
            },
            other => other.clone(),
        }
    }
}

struct Case {
    kind: &'static str,
    line: u32,
    old: String,
    new: String,
    expect: Expect,
}

#[derive(Default)]
struct Tally {
    tried: u32,
    passed: u32,
    invalid: u32,
    fallback: u32,
    reasons: BTreeMap<String, u32>,
}

pub fn run(args: Args) -> Result<(), String> {
    let files = match &args.rev {
        Some(rev) => list_at(&args.repo, rev, &args.ext, args.filter.as_deref())?,
        None => list(&args.repo, &args.ext, args.filter.as_deref())?,
    };
    let mut blobs = match &args.rev {
        Some(_) => Some(crate::corpus::Blobs::spawn(&args.repo)?),
        None => None,
    };
    let files = match args.max_files {
        Some(max) => spread(files, max),
        None => files,
    };
    if let Some(out) = &args.out {
        std::fs::create_dir_all(out).map_err(|error| error.to_string())?;
    }
    let mut tallies: BTreeMap<&'static str, Tally> = BTreeMap::new();
    let mut shown = 0;
    for path in &files {
        let text = match (&mut blobs, &args.rev) {
            (Some(blobs), Some(rev)) => blobs
                .read(rev, path)?
                .and_then(|bytes| String::from_utf8(bytes).ok()),
            _ => std::fs::read_to_string(args.repo.join(path)).ok(),
        };
        let Some(text) = text else {
            continue;
        };
        let Some(language) = semantic_language(path) else {
            continue;
        };
        let (Some(syntax), Some(grammar)) = (syntax(language), engine::language(language)) else {
            continue;
        };
        let mut parser = ts::Parser::new();
        if parser.set_language(&grammar).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(&text, None) else {
            continue;
        };
        let errors = error_count(tree.root_node());
        for case in cases(&tree, &text, &syntax, args.per_file) {
            if !args.kinds.is_empty() && !args.kinds.iter().any(|k| k == case.kind) {
                continue;
            }
            let tally = tallies.entry(case.kind).or_default();
            let valid = parser
                .parse(&case.new, None)
                .is_some_and(|tree| error_count(tree.root_node()) <= errors);
            if !valid {
                tally.invalid += 1;
                continue;
            }
            tally.tried += 1;
            let run = engine::run(
                Engine::Native,
                path,
                case.old.as_bytes(),
                case.new.as_bytes(),
            )?;
            if run.fallback.is_some() {
                tally.fallback += 1;
                continue;
            }
            let failures = judge(&run.doc, &case.expect);
            if failures.is_empty() {
                tally.passed += 1;
                continue;
            }
            for reason in &failures {
                *tally.reasons.entry(reason.clone()).or_default() += 1;
            }
            if shown < args.show {
                shown += 1;
                println!(
                    "FAIL {} {path}:{} {}",
                    case.kind,
                    case.line + 1,
                    failures.join(", ")
                );
            }
            if let Some(out) = &args.out {
                write_fixture(out, path, &case)?;
            }
        }
    }
    println!("{} files", files.len());
    for (kind, tally) in &tallies {
        println!(
            "{kind:8} {:5} tried {:5} passed {:4} failed {:4} fallback {:4} invalid",
            tally.tried,
            tally.passed,
            tally.tried - tally.passed - tally.fallback,
            tally.fallback,
            tally.invalid
        );
        for (reason, count) in &tally.reasons {
            println!("         {count:5} {reason}");
        }
    }
    let failed: u32 = tallies
        .values()
        .map(|t| t.tried - t.passed - t.fallback)
        .sum();
    if failed > 0 {
        return Err(format!("{failed} generated edits read wrong"));
    }
    Ok(())
}

pub(crate) fn list(
    repo: &Path,
    exts: &[String],
    filter: Option<&str>,
) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("ls-files")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| {
            Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| exts.iter().any(|e| e == ext))
        })
        .filter(|path| filter.is_none_or(|f| path.contains(f)))
        .map(str::to_owned)
        .collect())
}

pub(crate) fn list_at(
    repo: &Path,
    rev: &str,
    exts: &[String],
    filter: Option<&str>,
) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-tree", "-r", "--name-only", rev])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| {
            Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| exts.iter().any(|e| e == ext))
        })
        .filter(|path| filter.is_none_or(|f| path.contains(f)))
        .map(str::to_owned)
        .collect())
}

pub(crate) fn spread<T>(items: Vec<T>, max: usize) -> Vec<T> {
    let len = items.len();
    if len <= max {
        return items;
    }
    let picks: Vec<usize> = (0..max).map(|i| i * len / max).collect();
    items
        .into_iter()
        .enumerate()
        .filter(|(i, _)| picks.binary_search(i).is_ok())
        .map(|(_, item)| item)
        .collect()
}

fn write_fixture(out: &Path, path: &str, case: &Case) -> Result<(), String> {
    let file = Path::new(path);
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .replace('.', "-");
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("txt");
    let name = format!("{}-{stem}-{}.{ext}.txt", case.kind, case.line + 1);
    let text = format!("<<<< old\n{}<<<< new\n{}<<<< expect\n", case.old, case.new);
    std::fs::write(out.join(name), text).map_err(|error| error.to_string())
}

pub(crate) fn error_count(node: ts::Node) -> usize {
    if !node.has_error() {
        return 0;
    }
    let own = usize::from(node.is_error() || node.is_missing());
    let mut cursor = node.walk();
    let children: usize = node.children(&mut cursor).map(error_count).sum();
    own + children
}

fn cases(tree: &ts::Tree, text: &str, syntax: &Syntax, per_file: usize) -> Vec<Case> {
    let mut nodes = Vec::new();
    collect(tree.root_node(), &mut nodes);
    let lines = LineIndex::new(text);
    let mut out = Vec::new();
    for case in spread(renames(&nodes, text), per_file) {
        push_both(&mut out, case, "rename", "rename");
    }
    for case in spread(swaps(&nodes, text, &lines, syntax), per_file) {
        push_both(&mut out, case, "swap", "swap");
    }
    for case in spread(reflows(&nodes, text, &lines, syntax), per_file) {
        push_both(&mut out, case, "reflow", "join");
    }
    for case in spread(wraps(&nodes, text, &lines, syntax), per_file) {
        push_both(&mut out, case, "wrap", "unwrap");
    }
    out
}

fn push_both(out: &mut Vec<Case>, mut case: Case, kind: &'static str, back: &'static str) {
    case.kind = kind;
    out.push(Case {
        kind: back,
        line: case.line,
        old: case.new.clone(),
        new: case.old.clone(),
        expect: case.expect.reversed(),
    });
    out.push(case);
}

pub(crate) fn collect<'t>(node: ts::Node<'t>, out: &mut Vec<ts::Node<'t>>) {
    out.push(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, out);
    }
}

pub(crate) struct LineIndex {
    pub(crate) starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Self { starts }
    }

    pub(crate) fn line(&self, row: usize) -> Range<usize> {
        let start = self.starts[row];
        let end = self.starts.get(row + 1).map_or(usize::MAX, |&s| s - 1);
        start..end
    }

    pub(crate) fn opens_line(&self, text: &str, node: &ts::Node) -> bool {
        let start = self.starts[node.start_position().row];
        text[start..node.start_byte()].trim().is_empty()
    }

    pub(crate) fn closes_line(&self, text: &str, node: &ts::Node) -> bool {
        let end = self.line(node.end_position().row).end.min(text.len());
        text[node.end_byte()..end].trim().is_empty()
    }

    pub(crate) fn indent<'a>(&self, text: &'a str, row: usize) -> &'a str {
        let line = self.line(row);
        let line = &text[line.start..line.end.min(text.len())];
        &line[..line.len() - line.trim_start().len()]
    }
}

fn unit(indent: &str) -> &'static str {
    if indent.starts_with('\t') {
        "\t"
    } else {
        "    "
    }
}

fn renames(nodes: &[ts::Node], text: &str) -> Vec<Case> {
    let mut uses: HashMap<&str, Vec<&ts::Node>> = HashMap::new();
    for node in nodes {
        if node.kind() == "identifier" {
            uses.entry(&text[node.byte_range()]).or_default().push(node);
        }
    }
    let mut picks: Vec<(usize, &str)> = uses
        .iter()
        .filter(|(name, found)| {
            name.len() >= 3
                && found.len() >= 2
                && words(text, name).len() == found.len()
                && words(text, &renamed(name)).is_empty()
        })
        .map(|(name, found)| (found[0].start_byte(), *name))
        .collect();
    picks.sort();
    picks
        .into_iter()
        .map(|(first, name)| {
            let to = renamed(name);
            let mut new = String::with_capacity(text.len());
            let mut at = 0;
            for range in words(text, name) {
                new.push_str(&text[at..range.start]);
                new.push_str(&to);
                at = range.end;
            }
            new.push_str(&text[at..]);
            Case {
                kind: "",
                line: text[..first].matches('\n').count() as u32,
                old: text.to_owned(),
                new,
                expect: Expect::Rename {
                    from: name.to_owned(),
                    to,
                },
            }
        })
        .collect()
}

fn renamed(name: &str) -> String {
    format!("{name}_v2")
}

fn words(text: &str, word: &str) -> Vec<Range<usize>> {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word)
        .filter(|(i, _)| {
            let before = text[..*i].chars().next_back().is_none_or(|c| !ident(c));
            let after = text[i + word.len()..]
                .chars()
                .next()
                .is_none_or(|c| !ident(c));
            before && after
        })
        .map(|(i, _)| i..i + word.len())
        .collect()
}

fn swaps(nodes: &[ts::Node], text: &str, lines: &LineIndex, syntax: &Syntax) -> Vec<Case> {
    let mut out = Vec::new();
    for a in nodes {
        if !syntax.functions.contains(&a.kind()) {
            continue;
        }
        let Some(b) = a.next_named_sibling() else {
            continue;
        };
        if !syntax.functions.contains(&b.kind())
            || !text[a.end_byte()..b.start_byte()].trim().is_empty()
            || !lines.opens_line(text, a)
            || !lines.closes_line(text, &b)
            || a.prev_sibling()
                .is_some_and(|p| p.end_position().row + 1 >= a.start_position().row)
        {
            continue;
        }
        let (a, b) = (a.byte_range(), b.byte_range());
        let new = [
            &text[..a.start],
            &text[b.clone()],
            &text[a.end..b.start],
            &text[a.clone()],
            &text[b.end..],
        ]
        .concat();
        out.push(Case {
            kind: "",
            line: text[..a.start].matches('\n').count() as u32,
            old: text.to_owned(),
            new,
            expect: Expect::Move,
        });
    }
    out
}

fn reflows(nodes: &[ts::Node], text: &str, lines: &LineIndex, syntax: &Syntax) -> Vec<Case> {
    let mut out = Vec::new();
    for node in nodes {
        if node.kind() != syntax.arguments || node.start_position().row != node.end_position().row {
            continue;
        }
        let row = node.start_position().row;
        let line = lines.line(row);
        let line_text = &text[line.start..line.end.min(text.len())];
        if line_text.trim_start().starts_with('#') || line_text.trim_end().ends_with('\\') {
            continue;
        }
        let mut cursor = node.walk();
        let args: Vec<ts::Node> = node.named_children(&mut cursor).collect();
        if args.len() < 2 || args.iter().any(|a| a.kind().contains("comment")) {
            continue;
        }
        let open = node.start_byte();
        let gaps_ok = text[open + 1..args[0].start_byte()].trim().is_empty()
            && text[args[args.len() - 1].end_byte()..node.end_byte() - 1]
                .trim()
                .is_empty()
            && args
                .windows(2)
                .all(|w| text[w[0].end_byte()..w[1].start_byte()].trim() == ",");
        if !gaps_ok || &text[open..open + 1] != "(" {
            continue;
        }
        let indent = lines.indent(text, row);
        let inner = format!("{indent}{}", unit(indent));
        let body: Vec<&str> = args.iter().map(|a| &text[a.byte_range()]).collect();
        let new = format!(
            "{}(\n{inner}{})",
            &text[..open],
            body.join(&format!(",\n{inner}"))
        ) + &text[node.end_byte()..];
        out.push(Case {
            kind: "",
            line: row as u32,
            old: text.to_owned(),
            new,
            expect: Expect::Reflow,
        });
    }
    out
}

fn wraps(nodes: &[ts::Node], text: &str, lines: &LineIndex, syntax: &Syntax) -> Vec<Case> {
    let mut out = Vec::new();
    for node in nodes {
        if node.kind() != syntax.block {
            continue;
        }
        let mut cursor = node.walk();
        let mut statements: Vec<ts::Node> = node.named_children(&mut cursor).collect();
        if let [list] = statements.as_slice()
            && list.kind() == "statement_list"
        {
            let mut cursor = list.walk();
            statements = list.named_children(&mut cursor).collect();
        }
        for run in [3, 1] {
            let Some(window) = statements.windows(run).find(|w| wrappable(text, lines, w)) else {
                continue;
            };
            out.push(wrap(text, lines, syntax, window));
            break;
        }
    }
    out
}

fn wrappable(text: &str, lines: &LineIndex, run: &[ts::Node]) -> bool {
    let (first, last) = (run[0], run[run.len() - 1]);
    if !lines.opens_line(text, &first)
        || !lines.closes_line(text, &last)
        || run.iter().all(|node| node.kind().contains("comment"))
    {
        return false;
    }
    let rows = first.start_position().row..=last.end_position().row;
    let mut stack = run.to_vec();
    while let Some(node) = stack.pop() {
        if (node.kind().contains("string") || node.kind() == "jsx_text")
            && node.start_position().row != node.end_position().row
        {
            return false;
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    rows.into_iter().all(|row| {
        let line = lines.line(row);
        let line = &text[line.start..line.end.min(text.len())];
        !line.trim_start().starts_with('#') && !line.trim_end().ends_with('\\')
    })
}

fn wrap(text: &str, lines: &LineIndex, syntax: &Syntax, run: &[ts::Node]) -> Case {
    let (first, last) = (
        run[0].start_position().row,
        run[run.len() - 1].end_position().row,
    );
    let indent = lines.indent(text, first);
    let step = unit(indent);
    let start = lines.line(first).start;
    let end = lines.line(last).end.saturating_add(1).min(text.len());
    let mut new = String::with_capacity(text.len() + 64);
    new.push_str(&text[..start]);
    new.push_str(&format!("{indent}{}\n", syntax.wrap.0));
    for line in text[start..end].split_inclusive('\n') {
        if !line.trim().is_empty() {
            new.push_str(step);
        }
        new.push_str(line);
    }
    if !new.ends_with('\n') {
        new.push('\n');
    }
    let mut outer = vec![first as u32];
    let inner = first as u32 + 1..last as u32 + 2;
    if let Some(close) = syntax.wrap.1 {
        new.push_str(&format!("{indent}{close}\n"));
        outer.push(inner.end);
    }
    new.push_str(&text[end..]);
    Case {
        kind: "",
        line: first as u32,
        old: text.to_owned(),
        new,
        expect: Expect::Wrap {
            side: Side::New,
            outer,
            inner,
            at: first as u32,
        },
    }
}

fn judge(doc: &DiffDocument, expect: &Expect) -> Vec<String> {
    let mut out = Vec::new();
    let (_, violations) = check::check(doc);
    if !violations.is_empty() {
        out.push("invariant".to_owned());
    }
    let Some(semantics) = doc.semantics(0) else {
        out.push("no semantics".to_owned());
        return out;
    };
    let mut steps: Vec<Step> = Vec::new();
    for block in change_blocks(doc, 0) {
        match tile(doc.aligned_rows(block.hunk), block.old, block.new) {
            Some(tiled) => steps.extend(tiled),
            None => {
                out.push("untiled".to_owned());
                return out;
            }
        }
    }
    let blank = |side: Side, line: u32| {
        doc.text(0, side)
            .display_line(line)
            .is_none_or(|t| t.trim().is_empty())
    };
    let marks = |side: Side, line: u32, renamed: bool| {
        doc.semantic_spans(0, side, line)
            .iter()
            .any(|s| match s.intensity {
                ChangeIntensity::Novel | ChangeIntensity::NovelWord => true,
                ChangeIntensity::Renamed => renamed,
                ChangeIntensity::UnchangedContext => false,
            })
    };
    let marked = |step: &Step, renamed: bool| {
        step.0.is_some_and(|o| marks(Side::Old, o, renamed))
            || step.1.is_some_and(|n| marks(Side::New, n, renamed))
    };
    let blank_step = |step: &Step| {
        step.0.is_none_or(|o| blank(Side::Old, o)) && step.1.is_none_or(|n| blank(Side::New, n))
    };
    let mut fail = |reason: &str| {
        if !out.iter().any(|r| r == reason) {
            out.push(reason.to_owned());
        }
    };
    if !matches!(expect, Expect::Move) && !semantics.moves().is_empty() {
        fail("spurious move");
    }
    if !matches!(expect, Expect::Rename { .. }) && !semantics.renames().is_empty() {
        fail("spurious rename");
    }
    match expect {
        Expect::Rename { from, to } => {
            if !semantics
                .renames()
                .iter()
                .any(|r| &*r.old == from && &*r.new == to)
            {
                fail("rename missed");
            }
            for step in &steps {
                if (step.0.is_none() || step.1.is_none()) && !blank_step(step) {
                    fail("unpaired line");
                }
                if marked(step, false) {
                    fail("novel text");
                }
            }
        }
        Expect::Move => {
            let moves = semantics.moves();
            if moves.is_empty() {
                fail("move missed");
            }
            if moves.len() > 1 {
                fail("split move");
            }
            let inside = |side: Side, line: u32| {
                moves.iter().any(|m| match side {
                    Side::Old => (m.old_line..m.old_line + m.old_lines).contains(&line),
                    Side::New => (m.new_line..m.new_line + m.lines).contains(&line),
                })
            };
            for step in &steps {
                let stray = step
                    .0
                    .is_some_and(|o| marks(Side::Old, o, true) && !inside(Side::Old, o))
                    || step
                        .1
                        .is_some_and(|n| marks(Side::New, n, true) && !inside(Side::New, n));
                if stray {
                    fail("edit outside move");
                }
            }
        }
        Expect::Reflow => {
            if steps.iter().any(|step| marked(step, true)) {
                fail("marked text");
            }
            let comparison = Comparison::new(doc, ComparisonOptions::default());
            let p =
                Projection::with_comparison(doc, Mode::Unified, &Expansion::new(doc), &comparison);
            let open = p
                .kind
                .iter()
                .any(|kind| matches!(kind, RowKind::Added | RowKind::Removed | RowKind::Merged));
            if open {
                fail("not folded");
            }
        }
        Expect::Wrap {
            side,
            outer,
            inner,
            at,
        } => {
            for step in &steps {
                let (w, o) = match side {
                    Side::Old => (step.0, step.1),
                    Side::New => (step.1, step.0),
                };
                match w {
                    Some(w) if inner.contains(&w) => {
                        if o != Some(at + w - inner.start) {
                            fail("inner line unpaired");
                        }
                        if marked(step, true) {
                            fail("inner line marked");
                        }
                    }
                    Some(w) if outer.contains(&w) => {
                        if o.is_some() {
                            fail("wrapper paired");
                        }
                    }
                    _ if blank_step(step) => {}
                    _ => fail("stray change"),
                }
            }
        }
    }
    out
}
