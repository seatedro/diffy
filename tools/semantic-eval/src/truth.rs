use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::Limits;
use tree_sitter as ts;

use crate::corpus::Blobs;
use crate::engine;
use crate::explain::{self, Checks};
use crate::grade::{self, Grade, Partners, Span, Truth};
use crate::mutate::{self, LineIndex, Syntax};

const REGION_LINES: usize = 3;

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
    #[arg(long)]
    jsonl: Option<PathBuf>,
    #[arg(long)]
    compare: Option<PathBuf>,
    #[arg(long)]
    strict: bool,
    filter: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub fallback: bool,
    pub grade: Grade,
}

impl Record {
    pub fn misleads(&self) -> bool {
        !self.fallback && self.grade.misleads()
    }

    pub fn incomplete(&self) -> bool {
        !self.fallback && self.grade.region.incomplete()
    }
}

enum Piece {
    Copy(Range<usize>),
    Dup(Range<usize>),
    Fresh(String),
}

struct Case {
    kind: &'static str,
    back: &'static str,
    line: usize,
    new: String,
    origins: Vec<(Range<usize>, Option<usize>)>,
}

impl Case {
    fn build(
        kind: &'static str,
        back: &'static str,
        line: usize,
        old: &str,
        pieces: Vec<Piece>,
    ) -> Self {
        let mut new = String::with_capacity(old.len() + 256);
        let mut origins = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let at = new.len();
            match piece {
                Piece::Copy(range) => {
                    new.push_str(&old[range.clone()]);
                    origins.push((at..new.len(), Some(range.start)));
                }
                Piece::Dup(range) => {
                    new.push_str(&old[range]);
                    origins.push((at..new.len(), None));
                }
                Piece::Fresh(text) => {
                    new.push_str(&text);
                    origins.push((at..new.len(), None));
                }
            }
        }
        Self {
            kind,
            back,
            line,
            new,
            origins,
        }
    }

    fn region(&self) -> Vec<Span> {
        let mut points: Vec<(usize, usize)> = Vec::new();
        let mut previous: Option<usize> = None;
        for (range, from) in &self.origins {
            match from {
                Some(from) => {
                    if previous.is_some_and(|end| end != *from) {
                        points.push((range.start, range.start));
                    }
                    previous = Some(from + range.len());
                }
                None => {
                    points.push((range.start, range.end));
                    previous = None;
                }
            }
        }
        let lines = LineIndex::new(&self.new);
        let row = |byte: usize| {
            lines
                .starts
                .partition_point(|&s| s <= byte)
                .saturating_sub(1)
        };
        points
            .into_iter()
            .map(|(a, b)| {
                let first = row(a).saturating_sub(REGION_LINES);
                let last = (row(b) + REGION_LINES + 1).min(lines.starts.len());
                let end = lines.starts.get(last).copied().unwrap_or(self.new.len());
                lines.starts[first] as u32..end as u32
            })
            .collect()
    }

    fn unlinked(&self, old: &str, partners: &Partners<'_>) -> [Vec<Span>; 2] {
        let mut fresh: Vec<Span> = Vec::new();
        for token in partners.tokens(1) {
            let start = token.span.start as usize;
            let at = self
                .origins
                .partition_point(|(range, _)| range.end <= start);
            if let Some((range, None)) = self.origins.get(at)
                && token.span.end as usize <= range.end
            {
                fresh.push(token.span.clone());
            }
        }
        let mut kept: Vec<Range<usize>> = self
            .origins
            .iter()
            .filter_map(|(range, from)| from.map(|f| f..f + range.len()))
            .collect();
        kept.sort_by_key(|r| r.start);
        let mut gone: Vec<Span> = Vec::new();
        for token in partners.tokens(0) {
            let (start, end) = (token.span.start as usize, token.span.end as usize);
            let at = kept.partition_point(|r| r.start <= start);
            let inside = at > 0 && kept[at - 1].end >= end;
            if !inside && end <= old.len() {
                gone.push(token.span.clone());
            }
        }
        [gone, fresh]
    }

    fn truth(&self, partners: &Partners<'_>) -> Vec<(Span, Span)> {
        let mut out = Vec::new();
        for token in partners.tokens(1) {
            let (start, end) = (token.span.start as usize, token.span.end as usize);
            let at = self
                .origins
                .partition_point(|(range, _)| range.end <= start);
            let Some((range, Some(from))) = self.origins.get(at) else {
                continue;
            };
            if start < range.start || end > range.end {
                continue;
            }
            let old = (from + start - range.start) as u32..(from + end - range.start) as u32;
            if partners.token(0, &old).is_some() {
                out.push((old, token.span.clone()));
            }
        }
        out.sort_by_key(|(o, _)| o.start);
        out
    }
}

#[derive(Default)]
struct Tally {
    tried: u32,
    misled: u32,
    incomplete: u32,
    fallback: u32,
    invalid: u32,
    grade: Grade,
}

pub fn run(args: Args) -> Result<(), String> {
    let files = match &args.rev {
        Some(rev) => mutate::list_at(&args.repo, rev, &args.ext, args.filter.as_deref())?,
        None => mutate::list(&args.repo, &args.ext, args.filter.as_deref())?,
    };
    let mut blobs = match &args.rev {
        Some(_) => Some(Blobs::spawn(&args.repo)?),
        None => None,
    };
    let mut checks = Checks::default();
    let mut records: Vec<Record> = Vec::new();
    let files = match args.max_files {
        Some(max) => mutate::spread(files, max),
        None => files,
    };
    if let Some(out) = &args.out {
        std::fs::create_dir_all(out).map_err(|error| error.to_string())?;
    }
    let limits = Limits::default();
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
        let Some(name) = semantic_language(path) else {
            continue;
        };
        let (Some(syntax), Some(language), Some(config)) = (
            mutate::syntax(name),
            engine::language(name),
            diffy_semantic::config(name),
        ) else {
            continue;
        };
        let mut parser = ts::Parser::new();
        if parser.set_language(&language).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(&text, None) else {
            continue;
        };
        let errors = mutate::error_count(tree.root_node());
        let old_items = grade::items(tree.root_node(), &text);
        for case in cases(&tree, &text, &syntax, name, args.per_file) {
            let wanted = |kind: &str| args.kinds.is_empty() || args.kinds.iter().any(|k| k == kind);
            if !wanted(case.kind) && !wanted(case.back) {
                continue;
            }
            let Some(new_tree) = parser.parse(&case.new, None) else {
                continue;
            };
            if mutate::error_count(new_tree.root_node()) > errors {
                tallies.entry(case.kind).or_default().invalid += 1;
                continue;
            }
            let new_items = grade::items(new_tree.root_node(), &case.new);
            let id = |kind: &str| format!("{kind} {path}:{}", case.line + 1);
            let forward = explain::run(&language, config, &text, &case.new, &limits);
            let backward = explain::run(&language, config, &case.new, &text, &limits);
            let (Ok(forward), Ok(backward)) = (forward, backward) else {
                for kind in [case.kind, case.back] {
                    let tally = tallies.entry(kind).or_default();
                    tally.tried += 1;
                    tally.fallback += 1;
                    records.push(Record {
                        id: id(kind),
                        kind: kind.to_owned(),
                        fallback: true,
                        grade: Grade::default(),
                    });
                }
                continue;
            };
            checks.note(&id(case.kind), &forward);
            checks.note(&id(case.back), &backward);
            let partners = Partners::new(&forward.diff);
            let truth = case.truth(&partners);
            let [gone, fresh] = case.unlinked(&text, &partners);
            let region = case.region();
            let names = |items: &[grade::Item]| -> HashSet<u32> {
                items.iter().map(|item| item.name.start).collect()
            };
            let swapped: Vec<(Span, Span)> =
                truth.iter().map(|(o, n)| (n.clone(), o.clone())).collect();
            let (new_names, old_names) = (names(&new_items), names(&old_items));
            let runs = [
                (
                    case.kind,
                    &forward,
                    [text.as_str(), case.new.as_str()],
                    Truth {
                        links: &truth,
                        items: &new_names,
                        delimiters: &forward.delimiters,
                        unlinked: [&gone, &fresh],
                        region: Some((1, &region)),
                    },
                ),
                (
                    case.back,
                    &backward,
                    [case.new.as_str(), text.as_str()],
                    Truth {
                        links: &swapped,
                        items: &old_names,
                        delimiters: &backward.delimiters,
                        unlinked: [&fresh, &gone],
                        region: Some((0, &region)),
                    },
                ),
            ];
            for (kind, explained, texts, truth) in runs {
                if !wanted(kind) {
                    continue;
                }
                let partners = Partners::new(&explained.diff);
                let (grade, first) = grade::grade(texts, &partners, &truth);
                let record = Record {
                    id: id(kind),
                    kind: kind.to_owned(),
                    fallback: false,
                    grade,
                };
                let tally = tallies.entry(kind).or_default();
                tally.tried += 1;
                tally.grade.add(&grade);
                tally.incomplete += u32::from(record.incomplete());
                let misleads = record.misleads();
                records.push(record);
                if !misleads {
                    continue;
                }
                tally.misled += 1;
                if shown < args.show {
                    shown += 1;
                    println!(
                        "MISLEADS {kind} {path}:{} {}",
                        case.line + 1,
                        first.unwrap_or_default()
                    );
                }
                if let Some(out) = &args.out {
                    write_case(out, path, kind, case.line, texts)?;
                }
            }
        }
    }
    println!("{} files", files.len());
    println!(
        "{:14} {:>6} {:>6} {:>6} {:>5} {:>9} {:>8} {:>9} {:>8} {:>8} {:>6} {:>6}",
        "kind",
        "tried",
        "misled",
        "incomp",
        "fall",
        "precision",
        "recall",
        "edit prec",
        "edit rec",
        "spurious",
        "items✗",
        "lists✗"
    );
    let mut misled = 0;
    let mut incomplete = 0;
    for (kind, tally) in &tallies {
        let (a, r) = (&tally.grade.all, &tally.grade.region);
        println!(
            "{kind:14} {:6} {:6} {:6} {:5} {:9.4} {:8.4} {:9.4} {:8.4} {:8} {:6} {:6}",
            tally.tried,
            tally.misled,
            tally.incomplete,
            tally.fallback,
            ratio(a.right + a.equivalent, a.linked()),
            ratio(a.right + a.equivalent, a.truth()),
            ratio(r.right + r.equivalent, r.linked()),
            ratio(r.right + r.equivalent, r.truth()),
            a.spurious,
            a.items_wrong,
            a.lists_wrong
        );
        if tally.invalid > 0 {
            println!("{:14} {} invalid", "", tally.invalid);
        }
        misled += tally.misled;
        incomplete += tally.incomplete;
    }
    if let Some(path) = &args.jsonl {
        let mut out = String::new();
        for record in &records {
            out.push_str(&serde_json::to_string(record).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| e.to_string())?;
    }
    let regressed = match &args.compare {
        Some(path) => compare(path, &records, args.show)?,
        None => 0,
    };
    let checked = checks.report();
    if misled > 0 {
        return Err(format!("{misled} generated edits mislead"));
    }
    if args.strict && incomplete > 0 {
        return Err(format!("{incomplete} generated edits are incomplete"));
    }
    if regressed > 0 {
        return Err(format!("{regressed} cases regressed against the baseline"));
    }
    checked
}

pub fn compare(path: &Path, records: &[Record], show: usize) -> Result<u32, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut base: HashMap<String, Record> = HashMap::new();
    for line in text.lines() {
        let record: Record = serde_json::from_str(line).map_err(|e| e.to_string())?;
        base.insert(record.id.clone(), record);
    }
    let (mut same, mut missing, mut worse, mut better) = (0, 0, Vec::new(), Vec::new());
    for record in records {
        let Some(old) = base.get(&record.id) else {
            missing += 1;
            continue;
        };
        same += 1;
        let verdict = |r: &Record| (r.fallback, r.misleads(), r.incomplete());
        let (was, now) = (verdict(old), verdict(record));
        let rank = |v: (bool, bool, bool)| u8::from(v.0) * 4 + u8::from(v.1) * 2 + u8::from(v.2);
        if rank(now) > rank(was) {
            worse.push(&record.id);
        } else if rank(now) < rank(was) {
            better.push(&record.id);
        }
    }
    println!(
        "against {}: {same} shared cases, {missing} new, {} worse, {} better",
        path.display(),
        worse.len(),
        better.len()
    );
    for id in worse.iter().take(show) {
        println!("  worse {id}");
    }
    for id in better.iter().take(show) {
        println!("  better {id}");
    }
    Ok(worse.len() as u32)
}

fn ratio(part: u32, whole: u32) -> f64 {
    if whole == 0 {
        1.0
    } else {
        f64::from(part) / f64::from(whole)
    }
}

fn write_case(
    out: &Path,
    path: &str,
    kind: &str,
    line: usize,
    texts: [&str; 2],
) -> Result<(), String> {
    let file = Path::new(path);
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .replace('.', "-");
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("txt");
    let name = format!("{kind}-{stem}-{}.{ext}.txt", line + 1);
    let text = format!("<<<< old\n{}<<<< new\n{}<<<< expect\n", texts[0], texts[1]);
    std::fs::write(out.join(name), text).map_err(|error| error.to_string())
}

fn cases(
    tree: &ts::Tree,
    text: &str,
    syntax: &Syntax,
    language: &str,
    per_file: usize,
) -> Vec<Case> {
    let mut nodes = Vec::new();
    mutate::collect(tree.root_node(), &mut nodes);
    let lines = LineIndex::new(text);
    let functions: Vec<Function> = nodes
        .iter()
        .filter(|n| syntax.functions.contains(&n.kind()) && n.kind() != "decorated_definition")
        .filter_map(|&n| Function::new(n, text, &lines))
        .collect();
    let mut out = Vec::new();
    out.extend(mutate::spread(
        extracts(&functions, text, &lines, language, false),
        per_file,
    ));
    out.extend(mutate::spread(
        extracts(&functions, text, &lines, language, true),
        per_file,
    ));
    out.extend(mutate::spread(
        insert_statements(&functions, text, &lines),
        per_file,
    ));
    out.extend(mutate::spread(insert_items(&functions, text), per_file));
    out.extend(mutate::spread(clones(&functions, text, &lines), per_file));
    out
}

struct Function<'t> {
    node: ts::Node<'t>,
    name: Option<ts::Node<'t>>,
    statements: Vec<ts::Node<'t>>,
    lines: Range<usize>,
}

impl<'t> Function<'t> {
    fn new(node: ts::Node<'t>, text: &str, lines: &LineIndex) -> Option<Self> {
        if !lines.opens_line(text, &node) || !lines.closes_line(text, &node) {
            return None;
        }
        let body = node.child_by_field_name("body")?;
        let mut cursor = body.walk();
        let statements: Vec<ts::Node> = body
            .named_children(&mut cursor)
            .filter(|n| !n.kind().contains("comment"))
            .collect();
        let whole = statements
            .iter()
            .all(|s| lines.opens_line(text, s) && lines.closes_line(text, s))
            && statements
                .windows(2)
                .all(|w| w[0].end_position().row < w[1].start_position().row);
        if !whole {
            return None;
        }
        Some(Self {
            node,
            name: grade::name_node(node),
            statements,
            lines: whole_lines(lines, text, &node),
        })
    }
}

fn whole_lines(lines: &LineIndex, text: &str, node: &ts::Node) -> Range<usize> {
    let start = lines.starts[node.start_position().row];
    let end = (lines.line(node.end_position().row).end + 1).min(text.len());
    start..end
}

fn fresh_name(text: &str, base: &str) -> String {
    (1..)
        .map(|k| format!("{base}{k}"))
        .find(|name| !text.contains(name.as_str()))
        .unwrap_or_default()
}

fn extracts(
    functions: &[Function],
    text: &str,
    lines: &LineIndex,
    language: &str,
    most: bool,
) -> Vec<Case> {
    let mut out = Vec::new();
    for f in functions {
        let n = f.statements.len();
        let python = language == "python";
        let run = if most {
            let end = if python { n - 1 } else { n };
            if end < 3 {
                continue;
            }
            1..end
        } else {
            if n < 4 {
                continue;
            }
            n / 2 - 1..n / 2 + 1
        };
        let (first, last) = (&f.statements[run.start], &f.statements[run.end - 1]);
        let a = lines.starts[first.start_position().row];
        let b = whole_lines(lines, text, last).end;
        let fn_end = f.lines.end - 1;
        if b > fn_end || text.as_bytes().get(fn_end) != Some(&b'\n') {
            continue;
        }
        let indent = lines.indent(text, first.start_position().row);
        let outer = lines.indent(text, f.node.start_position().row);
        let base = if language == "go" {
            "helperX"
        } else {
            "helper_x"
        };
        let name = fresh_name(text, base);
        let method = f.node.kind() == "method_definition"
            || python
                && f.node
                    .parent()
                    .and_then(|p| p.parent())
                    .is_some_and(|p| p.kind() == "class_definition");
        let (head, call, tail) = template(language, &name, method);
        let pieces = vec![
            Piece::Copy(0..a),
            Piece::Fresh(format!("{indent}{call}\n")),
            Piece::Copy(b..fn_end),
            Piece::Fresh(format!("\n\n{outer}{head}\n")),
            Piece::Copy(a..b),
            Piece::Fresh(tail.map(|t| format!("{outer}{t}")).unwrap_or_default()),
            Piece::Copy(fn_end..text.len()),
        ];
        let (kind, back) = if most {
            ("extract-most", "inline-most")
        } else {
            ("extract", "inline")
        };
        out.push(Case::build(
            kind,
            back,
            first.start_position().row,
            text,
            pieces,
        ));
    }
    out
}

fn template(language: &str, name: &str, method: bool) -> (String, String, Option<&'static str>) {
    match language {
        "rust" => (format!("fn {name}() {{"), format!("{name}();"), Some("}")),
        "go" => (format!("func {name}() {{"), format!("{name}()"), Some("}")),
        "c" | "cpp" => (
            format!("static void {name}(void) {{"),
            format!("{name}();"),
            Some("}"),
        ),
        "python" if method => (format!("def {name}(self):"), format!("self.{name}()"), None),
        "python" => (format!("def {name}():"), format!("{name}()"), None),
        _ if method => (format!("{name}() {{"), format!("this.{name}();"), Some("}")),
        _ => (
            format!("function {name}() {{"),
            format!("{name}();"),
            Some("}"),
        ),
    }
}

fn freshen(text: &str, node: ts::Node, range: Range<usize>) -> String {
    let mut names = Vec::new();
    let mut nodes = Vec::new();
    mutate::collect(node, &mut nodes);
    for n in nodes {
        if n.child_count() == 0 && n.kind().contains("identifier") {
            names.push(n.byte_range());
        }
    }
    let mut out = String::new();
    let mut at = range.start;
    for name in names {
        out.push_str(&text[at..name.end]);
        out.push_str("_n1");
        at = name.end;
    }
    out.push_str(&text[at..range.end]);
    out
}

fn insert_statements(functions: &[Function], text: &str, lines: &LineIndex) -> Vec<Case> {
    let mut out = Vec::new();
    for f in functions {
        let Some(first) = f.statements.first() else {
            continue;
        };
        if f.statements.len() < 2 || first.end_position().row - first.start_position().row > 5 {
            continue;
        }
        let range = whole_lines(lines, text, first);
        let fresh = freshen(text, *first, range.clone());
        let pieces = vec![
            Piece::Copy(0..range.start),
            Piece::Fresh(fresh),
            Piece::Copy(range.start..text.len()),
        ];
        out.push(Case::build(
            "insert-stmt",
            "delete-stmt",
            first.start_position().row,
            text,
            pieces,
        ));
    }
    out
}

fn insert_items(functions: &[Function], text: &str) -> Vec<Case> {
    let mut out = Vec::new();
    for f in functions {
        let rows = f.node.end_position().row - f.node.start_position().row;
        if !(2..=40).contains(&rows) || f.name.is_none() {
            continue;
        }
        let fresh = freshen(text, f.node, f.lines.clone());
        let pieces = vec![
            Piece::Copy(0..f.lines.start),
            Piece::Fresh(fresh + "\n"),
            Piece::Copy(f.lines.start..text.len()),
        ];
        out.push(Case::build(
            "insert-item",
            "delete-item",
            f.node.start_position().row,
            text,
            pieces,
        ));
    }
    out
}

fn clones(functions: &[Function], text: &str, lines: &LineIndex) -> Vec<Case> {
    let mut out = Vec::new();
    for f in functions {
        let (Some(name), Some(last)) = (f.name, f.statements.last()) else {
            continue;
        };
        if f.statements.len() < 2 || name.child_count() > 0 {
            continue;
        }
        let cut = whole_lines(lines, text, last);
        let renamed = format!("{}_c", &text[name.byte_range()]);
        let pieces = vec![
            Piece::Copy(0..f.lines.start),
            Piece::Dup(f.lines.start..name.start_byte()),
            Piece::Fresh(fresh_name(text, &renamed)),
            Piece::Dup(name.end_byte()..f.lines.end),
            Piece::Fresh("\n".to_owned()),
            Piece::Copy(f.lines.start..cut.start),
            Piece::Copy(cut.end..text.len()),
        ];
        out.push(Case::build(
            "clone",
            "unclone",
            f.node.start_position().row,
            text,
            pieces,
        ));
    }
    out
}
