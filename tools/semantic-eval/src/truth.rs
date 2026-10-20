use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::{Limits, SemanticDiff};
use tree_sitter as ts;

use crate::engine;
use crate::grade::{self, Partners, Score, Span};
use crate::mutate::{self, LineIndex, Syntax};

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
    filter: Option<String>,
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
    fallback: u32,
    invalid: u32,
    score: Score,
}

pub fn run(args: Args) -> Result<(), String> {
    let files = mutate::list(&args.repo, &args.ext, args.filter.as_deref())?;
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
        let Ok(text) = std::fs::read_to_string(args.repo.join(path)) else {
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
            let forward = diffy_semantic::diff(&language, config, &text, &case.new, &limits);
            let backward = diffy_semantic::diff(&language, config, &case.new, &text, &limits);
            let (Ok(forward), Ok(backward)) = (forward, backward) else {
                for kind in [case.kind, case.back] {
                    let tally = tallies.entry(kind).or_default();
                    tally.tried += 1;
                    tally.fallback += 1;
                }
                continue;
            };
            let partners = Partners::new(&forward);
            let truth = case.truth(&partners);
            let names = |items: &[grade::Item]| -> HashSet<u32> {
                items.iter().map(|item| item.name.start).collect()
            };
            let swapped: Vec<(Span, Span)> =
                truth.iter().map(|(o, n)| (n.clone(), o.clone())).collect();
            let runs: [(
                &'static str,
                &SemanticDiff,
                [&str; 2],
                &[(Span, Span)],
                HashSet<u32>,
            ); 2] = [
                (
                    case.kind,
                    &forward,
                    [&text, &case.new],
                    &truth,
                    names(&new_items),
                ),
                (
                    case.back,
                    &backward,
                    [&case.new, &text],
                    &swapped,
                    names(&old_items),
                ),
            ];
            for (kind, diff, texts, truth, items) in runs {
                if !wanted(kind) {
                    continue;
                }
                let partners = Partners::new(diff);
                let (score, first) = grade::grade(texts, &partners, truth, &items);
                let tally = tallies.entry(kind).or_default();
                tally.tried += 1;
                tally.score.add(&score);
                if !score.misleads() {
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
        "{:14} {:>6} {:>7} {:>6} {:>9} {:>8} {:>7} {:>11}",
        "kind", "tried", "misled", "fall", "precision", "recall", "items", "items wrong"
    );
    let mut misled = 0;
    for (kind, tally) in &tallies {
        let s = &tally.score;
        let linked = s.right + s.equivalent + s.wrong;
        println!(
            "{kind:14} {:6} {:7} {:6} {:9.4} {:8.4} {:7} {:11}",
            tally.tried,
            tally.misled,
            tally.fallback,
            ratio(s.right + s.equivalent, linked),
            ratio(s.right + s.equivalent, s.truth()),
            s.items,
            s.items_wrong
        );
        if tally.invalid > 0 {
            println!("{:14} {} invalid", "", tally.invalid);
        }
        misled += tally.misled;
    }
    if misled > 0 {
        return Err(format!("{misled} generated edits mislead"));
    }
    Ok(())
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
