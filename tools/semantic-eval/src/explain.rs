use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::Limits;
use diffy_semantic::eval::{self, Explained, Features, Options, TERMS, Weights};

use crate::engine;

pub fn explain(
    path: &str,
    old: &str,
    new: &str,
    limits: &Limits,
    options: &Options,
) -> Option<Result<Explained, diffy_semantic::Fallback>> {
    let name = semantic_language(path)?;
    let (language, config) = (engine::language(name)?, diffy_semantic::config(name)?);
    Some(eval::explain(&language, config, old, new, limits, options))
}

pub fn wide() -> Limits {
    Limits {
        max_work: u64::MAX / 4,
        max_time: std::time::Duration::from_secs(600),
        ..Limits::default()
    }
}

pub fn terms(f: &Features) -> String {
    TERMS
        .iter()
        .zip(f.0)
        .filter(|&(_, v)| v != 0)
        .map(|(t, v)| format!("{t}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn show(path: &str, old: &[u8], new: &[u8], exhaustive: bool) -> Result<(), String> {
    let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    let options = Options {
        margins: true,
        ..eval::get()
    };
    let weights = options.weights;
    let fast = explain(path, &old, &new, &Limits::default(), &options)
        .ok_or("no grammar")?
        .map_err(|e| e.to_string())?;
    let mut expected = fast.baseline;
    expected -= fast.predicted;
    println!(
        "\ncost {} of {} (baseline), predicted {}, repairs {}, work {}",
        fast.cost(&weights),
        weights.cost(&fast.baseline),
        weights.cost(&expected),
        fast.repairs,
        fast.work
    );
    println!("  actual    {}", terms(&fast.actual));
    println!(
        "  move credit in alignment {}, realized moves {}",
        fast.credited, fast.realized
    );
    if !fast.agrees() {
        println!("  predicted {}", terms(&expected));
        println!("! prediction disagrees with the evaluator");
    }
    for m in &fast.margins {
        println!(
            "margin {} old {} new {} best {} runner {}",
            m.kind,
            crate::grade::line_of(&old, m.old) + 1,
            crate::grade::line_of(&new, m.new) + 1,
            m.best,
            m.runner
        );
    }
    if exhaustive {
        let options = Options {
            exhaustive: true,
            ..eval::get()
        };
        let full = explain(path, &old, &new, &wide(), &options)
            .ok_or("no grammar")?
            .map_err(|e| e.to_string())?;
        println!(
            "exhaustive cost {} (gap {}), agrees {}, repairs {}",
            full.cost(&weights),
            fast.cost(&weights) - full.cost(&weights),
            full.agrees(),
            full.repairs
        );
        println!("  actual    {}", terms(&full.actual));
    }
    Ok(())
}

pub fn weights(pairs: &[String]) -> Result<Weights, String> {
    let mut w = Weights::DEFAULT;
    for pair in pairs {
        let (key, value) = pair.split_once('=').ok_or(format!("bad weight {pair}"))?;
        let value: i64 = value.parse().map_err(|_| format!("bad weight {pair}"))?;
        let slot = match key {
            "gamma" => &mut w.gamma,
            "rho" => &mut w.rho,
            "rho0" => &mut w.rho0,
            "iota" => &mut w.iota,
            "mu" => &mut w.mu,
            "follow" => &mut w.follow,
            "beta" => &mut w.beta,
            "phi" => &mut w.phi,
            _ => return Err(format!("unknown weight {key}")),
        };
        *slot = value;
    }
    Ok(w)
}

pub fn run(
    language: &diffy_semantic::Language,
    config: &diffy_semantic::Config,
    old: &str,
    new: &str,
    limits: &Limits,
) -> Result<Explained, diffy_semantic::Fallback> {
    eval::explain(language, config, old, new, limits, &eval::get())
}

#[derive(Default)]
pub struct Checks {
    runs: u32,
    disagree: u32,
    repaired: u32,
    repairs: u32,
    first: Option<String>,
}

impl Checks {
    pub fn note(&mut self, label: &str, e: &Explained) {
        if eval::get().legacy {
            return;
        }
        self.runs += 1;
        if !e.agrees() {
            self.disagree += 1;
            if self.first.is_none() {
                let mut expected = e.baseline;
                expected -= e.predicted;
                self.first = Some(format!(
                    "{label}: predicted {} actual {}",
                    terms(&expected),
                    terms(&e.actual)
                ));
            }
        }
        if e.repairs > 0 {
            self.repaired += 1;
            self.repairs += e.repairs;
            if self.first.is_none() {
                self.first = Some(format!("{label}: {} repairs", e.repairs));
            }
        }
    }

    pub fn report(&self) -> Result<(), String> {
        if eval::get().legacy {
            return Ok(());
        }
        println!(
            "cost model: {} runs, {} disagree with the evaluator, {} repaired ({} links)",
            self.runs, self.disagree, self.repaired, self.repairs
        );
        if let Some(first) = &self.first {
            println!("  first: {first}");
        }
        if self.disagree > 0 || self.repaired > 0 {
            return Err(format!(
                "{} runs disagree, {} repaired",
                self.disagree, self.repaired
            ));
        }
        Ok(())
    }
}
