use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::{Limits, SemanticDiff};
use tree_sitter as ts;

use crate::corpus::{self, Blobs};
use crate::engine;
use crate::grade::{self, Partners, Score, Span};

const MIN_ANCHOR_WORD_CHARS: usize = 8;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, conflicts_with = "commits")]
    list: Option<PathBuf>,
    #[arg(long, default_value = "HEAD")]
    rev: String,
    #[arg(long, default_value_t = 200)]
    commits: usize,
    #[arg(long, value_delimiter = ',')]
    ext: Vec<String>,
    #[arg(long, default_value_t = 15)]
    worst: usize,
    #[arg(long)]
    jsonl: Option<PathBuf>,
}

#[derive(Default, serde::Serialize)]
struct Audit {
    file: String,
    anchors: Score,
    names: Score,
    asymmetric: u32,
    first: Option<String>,
}

impl Audit {
    fn wrong(&self) -> u32 {
        self.anchors.wrong + self.names.wrong
    }
}

pub fn run(args: Args) -> Result<(), String> {
    let mut blobs = Blobs::spawn(&args.repo)?;
    let mut pairs: Vec<(String, String, String)> = Vec::new();
    match &args.list {
        Some(list) => {
            let list = std::fs::read_to_string(list).map_err(|e| e.to_string())?;
            for line in list.lines() {
                let mut fields = line.split_whitespace();
                if let (Some(commit), Some(path)) = (fields.next(), fields.next()) {
                    pairs.push((format!("{commit}^"), commit.to_owned(), path.to_owned()));
                }
            }
        }
        None => {
            for (commit, parent) in corpus::commits(&args.repo, &args.rev, args.commits)? {
                for (old_path, new_path) in corpus::changes(&args.repo, &parent, &commit)? {
                    let ext = corpus::extension(&new_path);
                    if (args.ext.is_empty() || args.ext.contains(&ext)) && old_path == new_path {
                        pairs.push((parent.clone(), commit.clone(), new_path));
                    }
                }
            }
        }
    }
    let limits = Limits::default();
    let mut audits = Vec::new();
    for (parent, commit, path) in &pairs {
        let (Some(old), Some(new)) = (blobs.read(parent, path)?, blobs.read(commit, path)?) else {
            continue;
        };
        if old == new || corpus::binary(&old) || corpus::binary(&new) {
            continue;
        }
        let (Ok(old), Ok(new)) = (String::from_utf8(old), String::from_utf8(new)) else {
            continue;
        };
        let Some(name) = semantic_language(path) else {
            continue;
        };
        let (Some(language), Some(config)) = (engine::language(name), diffy_semantic::config(name))
        else {
            continue;
        };
        let (Ok(forward), Ok(backward)) = (
            diffy_semantic::diff(&language, config, &old, &new, &limits),
            diffy_semantic::diff(&language, config, &new, &old, &limits),
        ) else {
            continue;
        };
        let mut parser = ts::Parser::new();
        if parser.set_language(&language).is_err() {
            continue;
        }
        let (Some(old_tree), Some(new_tree)) = (parser.parse(&old, None), parser.parse(&new, None))
        else {
            continue;
        };
        let old_items = grade::items(old_tree.root_node(), &old);
        let new_items = grade::items(new_tree.root_node(), &new);
        let mut anchors = Score::default();
        let mut names = Score::default();
        let mut first = None;
        let directions = [
            (
                &forward,
                [old.as_str(), new.as_str()],
                [&old_items, &new_items],
            ),
            (
                &backward,
                [new.as_str(), old.as_str()],
                [&new_items, &old_items],
            ),
        ];
        for (diff, texts, [from, to]) in directions {
            let partners = Partners::new(diff);
            let item_names: HashSet<u32> = to.iter().map(|i| i.name.start).collect();
            let named = grade::same_named(from, to);
            let (n, named_first) = grade::grade(texts, &partners, &named, &item_names);
            let anchored = anchor_links(texts, &partners);
            let (a, anchor_first) = grade::grade(texts, &partners, &anchored, &item_names);
            names.add(&n);
            anchors.add(&a);
            first = first.or(named_first).or(anchor_first);
        }
        let audit = Audit {
            file: format!("{commit} {path}"),
            anchors,
            names,
            asymmetric: asymmetry(&forward, &backward),
            first,
        };
        audits.push(audit);
    }
    if let Some(path) = &args.jsonl {
        let mut out = String::new();
        for audit in &audits {
            out.push_str(&serde_json::to_string(audit).map_err(|e| e.to_string())?);
            out.push('\n');
        }
        std::fs::write(path, out).map_err(|e| e.to_string())?;
    }
    report(&audits, args.worst);
    Ok(())
}

fn report(audits: &[Audit], worst: usize) {
    let mut anchors = Score::default();
    let mut names = Score::default();
    for audit in audits {
        anchors.add(&audit.anchors);
        names.add(&audit.names);
    }
    let mut ranked: Vec<&Audit> = audits.iter().filter(|a| a.wrong() > 0).collect();
    ranked.sort_by_key(|a| {
        (
            std::cmp::Reverse(a.names.wrong),
            std::cmp::Reverse(a.wrong()),
        )
    });
    println!("names wrong  anchors wrong  missed  asym  file");
    for audit in ranked.iter().take(worst) {
        println!(
            "{:11} {:14} {:7} {:5}  {}\n      {}",
            audit.names.wrong,
            audit.anchors.wrong,
            audit.anchors.missed,
            audit.asymmetric,
            audit.file,
            audit.first.as_deref().unwrap_or("")
        );
    }
    let asym = audits.iter().filter(|a| a.asymmetric > 0).count();
    let per_k = |part: u32, whole: u32| f64::from(part) * 1000.0 / f64::from(whole.max(1));
    println!(
        "{} files: anchors {} links, {:.2} wrong and {:.2} missed per 1k; same-named items {}, {} wrong, {} missed; {} files wrong; {} asymmetric files",
        audits.len(),
        anchors.truth(),
        per_k(anchors.wrong, anchors.truth()),
        per_k(anchors.missed, anchors.truth()),
        names.truth(),
        names.wrong,
        names.missed,
        ranked.len(),
        asym
    );
}

fn anchor_links(texts: [&str; 2], partners: &Partners<'_>) -> Vec<(Span, Span)> {
    fn lines(text: &str) -> Vec<(usize, &str)> {
        let mut at = 0;
        text.split_inclusive('\n')
            .map(|line| {
                let start = at;
                at += line.len();
                let trimmed = line.trim_start();
                (start + line.len() - trimmed.len(), trimmed.trim_end())
            })
            .collect()
    }
    let sides = [lines(texts[0]), lines(texts[1])];
    let mut counts: HashMap<&str, [(u32, usize); 2]> = HashMap::new();
    for (side, lines) in sides.iter().enumerate() {
        for (index, (_, line)) in lines.iter().enumerate() {
            let words = line
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .count();
            if words >= MIN_ANCHOR_WORD_CHARS {
                let entry = counts.entry(line).or_default();
                entry[side].0 += 1;
                entry[side].1 = index;
            }
        }
    }
    let mut out = Vec::new();
    for (line, [(a, i), (b, j)]) in counts {
        if a != 1 || b != 1 {
            continue;
        }
        let (old_at, new_at) = (sides[0][i].0 as u32, sides[1][j].0 as u32);
        let len = line.len() as u32;
        let mut spans: Vec<Span> = partners
            .tokens(0)
            .filter(|t| t.span.start >= old_at && t.span.end <= old_at + len)
            .map(|t| t.span.clone())
            .collect();
        spans.sort_by_key(|s| s.start);
        let boundary = |s: &Span| {
            let text = &line[(s.start - old_at) as usize..(s.end - old_at) as usize];
            !text.chars().any(|c| c.is_alphanumeric() || c == '_')
                && (s.start == old_at || s.end == old_at + len)
        };
        for span in spans.into_iter().filter(|s| !boundary(s)) {
            let new = span.start - old_at + new_at..span.end - old_at + new_at;
            if partners.token(1, &new).is_some() {
                out.push((span, new));
            }
        }
    }
    out.sort_by_key(|(o, _)| o.start);
    out
}

fn asymmetry(forward: &SemanticDiff, backward: &SemanticDiff) -> u32 {
    let links = |tokens: &[diffy_semantic::Token]| -> HashSet<(u32, u32)> {
        tokens
            .iter()
            .filter_map(|t| t.partner.as_ref().map(|p| (t.span.start, p.start)))
            .collect()
    };
    let (a, b) = (links(&forward.tokens[0]), links(&backward.tokens[1]));
    a.symmetric_difference(&b).count() as u32
}
