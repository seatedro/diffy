use std::path::PathBuf;

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::Limits;
use diffy_semantic::eval::{self, Options};

use crate::corpus::{self, Blobs};
use crate::engine;
use crate::explain::{self, Checks};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, default_value = "HEAD")]
    rev: String,
    #[arg(long, default_value_t = 200)]
    commits: usize,
    #[arg(long, value_delimiter = ',')]
    ext: Vec<String>,
    #[arg(long, default_value_t = 6000)]
    max_bytes: usize,
    #[arg(long, default_value_t = 10)]
    worst: usize,
}

pub fn run(args: Args) -> Result<(), String> {
    let mut blobs = Blobs::spawn(&args.repo)?;
    let base = eval::get();
    let fast = Options {
        exhaustive: false,
        ..base
    };
    let full = Options {
        exhaustive: true,
        ..base
    };
    let weights = base.weights;
    let mut gaps: Vec<(i64, i64, String)> = Vec::new();
    let mut checks = Checks::default();
    let (mut fallbacks, mut micros) = (0, [0u128; 2]);
    for (commit, parent) in corpus::commits(&args.repo, &args.rev, args.commits)? {
        for (old_path, path) in corpus::changes(&args.repo, &parent, &commit)? {
            let ext = corpus::extension(&path);
            if old_path != path || !args.ext.is_empty() && !args.ext.contains(&ext) {
                continue;
            }
            let (Some(old), Some(new)) = (blobs.read(&parent, &path)?, blobs.read(&commit, &path)?)
            else {
                continue;
            };
            if old == new || old.len().max(new.len()) > args.max_bytes {
                continue;
            }
            let (Ok(old), Ok(new)) = (String::from_utf8(old), String::from_utf8(new)) else {
                continue;
            };
            let Some(name) = semantic_language(&path) else {
                continue;
            };
            let (Some(language), Some(config)) =
                (engine::language(name), diffy_semantic::config(name))
            else {
                continue;
            };
            let label = format!("{commit} {path}");
            let mut costs = [0i64; 2];
            let mut ok = true;
            for (k, (options, limits)) in [(fast, Limits::default()), (full, explain::wide())]
                .into_iter()
                .enumerate()
            {
                let start = std::time::Instant::now();
                match eval::explain(&language, config, &old, &new, &limits, &options) {
                    Ok(e) => {
                        checks.note(&label, &e, None);
                        costs[k] = e.cost(&weights);
                    }
                    Err(_) => ok = false,
                }
                micros[k] += start.elapsed().as_micros();
            }
            if !ok {
                fallbacks += 1;
                continue;
            }
            gaps.push((costs[0] - costs[1], costs[1], label));
        }
    }
    gaps.sort_by_key(|g| std::cmp::Reverse(g.0));
    let n = gaps.len().max(1);
    let worse = gaps.iter().filter(|g| g.0 > 0).count();
    let better = gaps.iter().filter(|g| g.0 < 0).count();
    let total: i64 = gaps.iter().map(|g| g.0).sum();
    let cost: i64 = gaps.iter().map(|g| g.1).sum();
    println!(
        "{} files, {fallbacks} fallbacks: production costs more on {worse}, less on {better}; total gap {total} over exhaustive cost {cost} ({:.3}%); time {:.1} ms vs {:.1} ms",
        gaps.len(),
        total as f64 * 100.0 / cost.max(1) as f64,
        micros[0] as f64 / 1000.0 / n as f64,
        micros[1] as f64 / 1000.0 / n as f64
    );
    for (gap, cost, label) in gaps.iter().take(args.worst).filter(|g| g.0 > 0) {
        println!("  +{gap} over {cost}  {label}");
    }
    for (gap, cost, label) in gaps.iter().rev().take(args.worst).filter(|g| g.0 < 0) {
        println!("  {gap} over {cost}  {label}");
    }
    checks.report()
}
