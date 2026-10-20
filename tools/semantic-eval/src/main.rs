mod audit;
mod capture;
mod check;
mod contract;
mod corpus;
mod engine;
mod explain;
mod fixtures;
mod gap;
mod gold;
mod grade;
mod moves;
mod mutate;
mod render;
mod report;
mod truth;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::corpus::Blobs;
use crate::engine::Engine;

#[derive(Parser)]
#[command(about = "Measures Diffy's semantic diff engines on real commits and fixtures.")]
struct Cli {
    #[arg(long, global = true, value_delimiter = ',')]
    weights: Vec<String>,
    #[arg(long, global = true)]
    legacy: bool,
    #[arg(long, global = true)]
    candidates: Option<usize>,
    #[arg(long, global = true)]
    no_floor: bool,
    #[arg(long, global = true)]
    no_credit: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(
        about = "Replays commits through each engine and summarizes time, fallbacks and invariants."
    )]
    Corpus(corpus::Args),
    #[command(about = "Checks each engine against the hand-checked fixtures.")]
    Fixtures(fixtures::Args),
    #[command(
        about = "Applies known edits to real files and checks the diff reports exactly them."
    )]
    Mutate(mutate::Args),
    #[command(about = "Grades the token links of generated edits whose true links are known.")]
    Truth(truth::Args),
    #[command(
        about = "Grades token links on real history against unique lines and same-named items."
    )]
    Audit(audit::Args),
    #[command(about = "Checks partial hand labels on real cases, with accepted alternatives.")]
    Gold(gold::Args),
    #[command(about = "Compares production search against exhaustive search on small files.")]
    Gap(gap::Args),
    #[command(about = "Scores how noisy move notes are on a list of commit and path pairs.")]
    Moves(moves::Args),
    #[command(about = "Renders one file's diff, from two files or from a commit.")]
    Show(ShowArgs),
    #[command(hide = true)]
    Tree { path: PathBuf },
    #[command(hide = true)]
    Bench {
        old: PathBuf,
        new: PathBuf,
        #[arg(long, default_value_t = 20)]
        times: u32,
    },
}

#[derive(clap::Args)]
struct ShowArgs {
    #[arg(long, default_value = "difftastic")]
    engine: Engine,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, conflicts_with_all = ["old", "new"])]
    commit: Option<String>,
    #[arg(long, requires = "new")]
    old: Option<PathBuf>,
    #[arg(long, requires = "old")]
    new: Option<PathBuf>,
    #[arg(long)]
    explain: bool,
    #[arg(long)]
    exhaustive: bool,
    path: Option<String>,
}

fn main() -> ExitCode {
    capture::install();
    let cli = Cli::parse();
    let defaults = diffy_semantic::eval::Options::default();
    let options = diffy_semantic::eval::Options {
        legacy: cli.legacy,
        candidates: cli.candidates.unwrap_or(defaults.candidates),
        floor: !cli.no_floor,
        credit: !cli.no_credit,
        ..defaults
    };
    let result = explain::weights(&cli.weights)
        .map(|weights| {
            diffy_semantic::eval::set(diffy_semantic::eval::Options { weights, ..options })
        })
        .and_then(|()| match cli.command {
            Command::Corpus(args) => corpus::run(args),
            Command::Fixtures(args) => fixtures::run(args),
            Command::Mutate(args) => mutate::run(args),
            Command::Moves(args) => moves::run(args),
            Command::Gap(args) => gap::run(args),
        Command::Gold(args) => gold::run(args),
            Command::Truth(args) => truth::run(args),
            Command::Audit(args) => audit::run(args),
            Command::Show(args) => show(args),
            Command::Tree { path } => tree(&path),
            Command::Bench { old, new, times } => bench(&old, &new, times),
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("semantic-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn show(args: ShowArgs) -> Result<(), String> {
    let read = |path: &PathBuf| std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()));
    let (path, old, new) = match (&args.commit, &args.old, &args.new) {
        (Some(commit), _, _) => {
            let path = args.path.clone().ok_or("--commit needs a path")?;
            let mut blobs = Blobs::spawn(&args.repo)?;
            let parent = format!("{commit}^");
            let old = blobs
                .read(&parent, &path)?
                .ok_or("path missing in parent")?;
            let new = blobs.read(commit, &path)?.ok_or("path missing in commit")?;
            (path, old, new)
        }
        (None, Some(old), Some(new)) => {
            let path = args
                .path
                .clone()
                .unwrap_or_else(|| new.to_string_lossy().into_owned());
            (path, read(old)?, read(new)?)
        }
        _ => return Err("pass --commit and a path, or --old and --new".to_owned()),
    };
    let run = engine::run(args.engine, &path, &old, &new)?;
    print!("{}", render::render(&run.doc, run.fallback.as_deref()));
    if let Some(semantics) = run.doc.semantics(0) {
        for item in semantics.outline() {
            println!(
                "outline {:?} {} {} {} {:?}",
                item.side,
                item.line + 1,
                item.kind,
                item.name,
                item.change
            );
        }
        for m in semantics.moves() {
            println!(
                "move old {}..{} -> new {}..{}",
                m.old_line + 1,
                m.old_line + m.old_lines,
                m.new_line + 1,
                m.new_line + m.lines
            );
        }
    }
    if args.explain || args.exhaustive {
        explain::show(&path, &old, &new, args.exhaustive)?;
    }
    let (_, mut violations) = check::check(&run.doc);
    if args.engine == Engine::Native {
        violations.extend(contract::check(&path, &old, &new));
    }
    println!("\n{:.2} ms", run.micros as f64 / 1000.0);
    for violation in &violations {
        println!("! {violation}");
    }
    if !violations.is_empty() {
        return Err(format!("{} invariant violations", violations.len()));
    }
    Ok(())
}

fn tree(path: &std::path::Path) -> Result<(), String> {
    let name = path.to_string_lossy();
    let language = diffy::core::compare::semantic::semantic_language(&name)
        .and_then(engine::language)
        .ok_or("no grammar")?;
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).map_err(|e| e.to_string())?;
    let tree = parser.parse(&text, None).ok_or("parse failed")?;
    let mut cursor = tree.walk();
    let mut depth = 0;
    loop {
        let node = cursor.node();
        if node.is_named() && depth <= 3 || node.is_error() {
            println!(
                "{}{} {}..{}",
                "  ".repeat(depth),
                node.kind(),
                node.start_position().row + 1,
                node.end_position().row + 1
            );
        }
        if cursor.goto_first_child() {
            depth += 1;
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Ok(());
            }
            depth -= 1;
        }
    }
}

fn bench(old: &std::path::Path, new: &std::path::Path, times: u32) -> Result<(), String> {
    let read = |p: &std::path::Path| std::fs::read_to_string(p).map_err(|e| e.to_string());
    let (a, b) = (read(old)?, read(new)?);
    let name = new.to_string_lossy();
    let language = diffy::core::compare::semantic::semantic_language(&name).ok_or("no grammar")?;
    let grammar = engine::language(language).ok_or("no grammar")?;
    let config = diffy_semantic::config(language).ok_or("no config")?;
    let limits = diffy_semantic::Limits::default();
    let mut times_ms = Vec::new();
    for _ in 0..times {
        let start = std::time::Instant::now();
        let out = diffy_semantic::diff(&grammar, config, &a, &b, &limits);
        times_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        if let Err(e) = out {
            return Err(e.to_string());
        }
    }
    times_ms.sort_by(f64::total_cmp);
    println!(
        "min {:.2} ms, median {:.2} ms",
        times_ms[0],
        times_ms[times_ms.len() / 2]
    );
    Ok(())
}
