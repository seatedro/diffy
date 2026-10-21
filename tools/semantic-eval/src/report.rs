use std::collections::BTreeMap;

use crate::corpus::Record;
use crate::engine::Engine;

pub fn print(records: &[Record], slowest: usize) {
    if records.is_empty() {
        println!("no files measured");
        return;
    }
    println!(
        "{:<11} {:<5} {:>6} {:>6} {:>8} {:>8} {:>8} {:>9} {:>9} {:>8} {:>6} {:>6} {:>6} {:>5}",
        "engine",
        "ext",
        "files",
        "fall%",
        "p50ms",
        "p95ms",
        "p99ms",
        "maxms",
        "total_s",
        "coldms",
        "novel%",
        "frag",
        "unmark",
        "viol"
    );
    let mut groups: BTreeMap<(&str, &str), Vec<&Record>> = BTreeMap::new();
    for record in records {
        groups
            .entry((record.engine.name(), record.ext.as_str()))
            .or_default()
            .push(record);
        groups
            .entry((record.engine.name(), "all"))
            .or_default()
            .push(record);
    }
    for ((engine, ext), group) in &groups {
        row(engine, ext, group);
    }
    for (engine, group) in by_engine(records) {
        reasons(engine, &group);
        phases(engine, &group);
        violations(engine, &group);
        slowest_files(engine, &group, slowest);
    }
}

fn by_engine(records: &[Record]) -> BTreeMap<&'static str, Vec<&Record>> {
    let mut out: BTreeMap<&'static str, Vec<&Record>> = BTreeMap::new();
    for record in records {
        out.entry(record.engine.name()).or_default().push(record);
    }
    out
}

fn row(engine: &str, ext: &str, group: &[&Record]) {
    let mut warm: Vec<u64> = group.iter().filter(|r| !r.cold).map(|r| r.micros).collect();
    warm.sort_unstable();
    let cold = group
        .iter()
        .filter(|r| r.cold)
        .map(|r| r.micros)
        .max()
        .unwrap_or(0);
    let total: u64 = group.iter().map(|r| r.micros).sum();
    let fallback = group.iter().filter(|r| !r.structural).count();
    let structural: Vec<_> = group.iter().filter(|r| r.structural).collect();
    let sum = |f: fn(&Record) -> u64| structural.iter().map(|r| f(r)).sum::<u64>();
    let changed = sum(|r| r.metrics.changed_bytes);
    let novel = sum(|r| r.metrics.novel_bytes + r.metrics.word_bytes);
    let marked = sum(|r| u64::from(r.metrics.marked_lines));
    let fragments = sum(|r| u64::from(r.metrics.fragments));
    let unmarked = sum(|r| u64::from(r.metrics.unmarked_lines));
    let violations = group
        .iter()
        .filter(|r| !r.violations.is_empty() || r.error.is_some())
        .count();
    println!(
        "{:<11} {:<5} {:>6} {:>6} {:>8} {:>8} {:>8} {:>9} {:>9.2} {:>8} {:>6} {:>6} {:>6} {:>5}",
        engine,
        ext,
        group.len(),
        format!("{:.1}", percent(fallback as u64, group.len() as u64)),
        ms(percentile(&warm, 50)),
        ms(percentile(&warm, 95)),
        ms(percentile(&warm, 99)),
        ms(warm.last().copied().unwrap_or(0)),
        total as f64 / 1e6,
        ms(cold),
        ratio(structural.is_empty(), percent(novel, changed)),
        ratio(marked == 0, fragments as f64 / marked.max(1) as f64),
        unmarked,
        violations
    );
}

fn reasons(engine: &str, group: &[&Record]) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for record in group
        .iter()
        .filter(|r| !r.structural && r.engine != Engine::Line)
    {
        let reason = record
            .error
            .clone()
            .or_else(|| record.fallback.clone())
            .unwrap_or_else(|| "no reason given".to_owned());
        *counts.entry(bucket(&reason)).or_default() += 1;
    }
    if counts.is_empty() {
        return;
    }
    println!("\n{engine} line fallbacks:");
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (reason, count) in counts {
        println!("  {count:>6}  {reason}");
    }
}

fn bucket(reason: &str) -> String {
    reason
        .split(|c: char| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .join("#")
}

fn phases(engine: &str, group: &[&Record]) {
    let mut totals: BTreeMap<&str, f64> = BTreeMap::new();
    for record in group {
        for (phase, ms) in &record.phases {
            *totals.entry(phase.as_str()).or_default() += ms;
        }
    }
    if totals.is_empty() {
        return;
    }
    let all: f64 = totals.values().sum();
    let mut totals: Vec<_> = totals.into_iter().collect();
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!("\n{engine} phases (sum over files):");
    for (phase, total) in totals {
        println!(
            "  {phase:<18} {:>10.1} ms {:>6.1}%",
            total,
            100.0 * total / all
        );
    }
}

fn violations(engine: &str, group: &[&Record]) {
    let bad: Vec<_> = group
        .iter()
        .filter(|r| !r.violations.is_empty() || r.error.is_some())
        .collect();
    if bad.is_empty() {
        return;
    }
    println!("\n{engine} invariant failures:");
    for record in bad.iter().take(20) {
        let first = record
            .error
            .as_deref()
            .or(record.violations.first().map(String::as_str))
            .unwrap_or_default();
        println!("  {} {}  {first}", short(&record.commit), record.path);
    }
    if bad.len() > 20 {
        println!("  ... {} more", bad.len() - 20);
    }
}

fn slowest_files(engine: &str, group: &[&Record], count: usize) {
    if count == 0 {
        return;
    }
    let mut sorted: Vec<_> = group.to_vec();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.micros));
    println!("\n{engine} slowest files:");
    for record in sorted.into_iter().take(count) {
        let top = record
            .phases
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(phase, ms)| format!(" {phase}={ms:.0}ms"))
            .unwrap_or_default();
        let fallback = if record.structural { "" } else { " line" };
        println!(
            "  {:>9} ms {:>8} B {} {}{fallback}{top}",
            ms(record.micros),
            record.old_bytes.max(record.new_bytes),
            short(&record.commit),
            record.path,
        );
    }
}

fn percentile(sorted: &[u64], p: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

fn ratio(empty: bool, value: f64) -> String {
    if empty {
        "-".to_owned()
    } else {
        format!("{value:.2}")
    }
}

fn ms(micros: u64) -> String {
    format!("{:.2}", micros as f64 / 1000.0)
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}
