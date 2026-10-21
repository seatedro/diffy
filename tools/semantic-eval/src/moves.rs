use std::path::PathBuf;

use diffy_diff::{
    ChangeIntensity, Comparison, ComparisonOptions, DiffDocument, Expansion, MOVE_GROUP_GAP, Mode,
    Projection, RowKind, Side, move_leaders,
};

use crate::corpus::Blobs;
use crate::engine::{self, Engine};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    list: PathBuf,
    #[arg(long, default_value_t = 15)]
    worst: usize,
    #[arg(long)]
    dump: bool,
}

#[derive(Default, Clone, Copy)]
struct Noise {
    moves: u32,
    moved_lines: u32,
    notes: u32,
    novel: u32,
    mostly_new: u32,
    tiny: u32,
    crowded: u32,
    leftover: u32,
    silent: u32,
}

impl Noise {
    fn score(&self) -> u32 {
        self.novel
            + 10 * self.mostly_new
            + 2 * self.crowded
            + self.tiny
            + 2 * self.leftover
            + 3 * self.silent
    }

    fn add(&mut self, other: &Noise) {
        self.moves += other.moves;
        self.moved_lines += other.moved_lines;
        self.notes += other.notes;
        self.novel += other.novel;
        self.mostly_new += other.mostly_new;
        self.tiny += other.tiny;
        self.crowded += other.crowded;
        self.leftover += other.leftover;
        self.silent += other.silent;
    }
}

pub fn run(args: Args) -> Result<(), String> {
    let list = std::fs::read_to_string(&args.list).map_err(|e| e.to_string())?;
    let mut blobs = Blobs::spawn(&args.repo)?;
    let mut total = Noise::default();
    let mut files = 0;
    let mut scored = Vec::new();
    for line in list.lines() {
        let mut fields = line.split_whitespace();
        let (Some(commit), Some(path)) = (fields.next(), fields.next()) else {
            continue;
        };
        let parent = format!("{commit}^");
        let (Some(old), Some(new)) = (blobs.read(&parent, path)?, blobs.read(commit, path)?) else {
            continue;
        };
        let run = engine::run(Engine::Native, path, &old, &new)?;
        if run.fallback.is_some() {
            continue;
        }
        let noise = measure(
            &run.doc,
            args.dump.then_some(format!("{commit} {path}")).as_deref(),
        );
        files += 1;
        total.add(&noise);
        scored.push((noise, format!("{commit} {path}")));
    }
    scored.sort_by_key(|(noise, _)| std::cmp::Reverse(noise.score()));
    println!("score moves notes novel mostly tiny crowded leftover silent  file");
    for (n, name) in scored.iter().take(args.worst) {
        println!(
            "{:5} {:5} {:5} {:5} {:6} {:4} {:7} {:8} {:6}  {name}",
            n.score(),
            n.moves,
            n.notes,
            n.novel,
            n.mostly_new,
            n.tiny,
            n.crowded,
            n.leftover,
            n.silent
        );
    }
    println!(
        "{files} files: {} moves over {} lines, {} notes; {} novel lines inside moves, {} mostly new moves, {} one-line moves, {} crowded notes, {} leftover lines, {} silent lines",
        total.moves,
        total.moved_lines,
        total.notes,
        total.novel,
        total.mostly_new,
        total.tiny,
        total.crowded,
        total.leftover,
        total.silent
    );
    Ok(())
}

fn measure(doc: &DiffDocument, dump: Option<&str>) -> Noise {
    let text = |side: Side, line: u32| {
        doc.text(0, side)
            .display_line(line)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let report = |class: &str, side: Side, line: u32| {
        if let Some(name) = dump {
            println!("{class}\t{name}:{}\t{}", line + 1, text(side, line));
        }
    };
    let mut noise = Noise::default();
    let Some(semantics) = doc.semantics(0) else {
        return noise;
    };
    let moves = semantics.moves();
    let leaders = move_leaders(moves);
    noise.moves = moves.len() as u32;
    let mut notes: Vec<u32> = Vec::new();
    for (i, m) in moves.iter().enumerate() {
        noise.moved_lines += m.lines;
        if m.lines == 1 && m.old_lines == 1 {
            noise.tiny += 1;
            report("tiny", Side::New, m.new_line);
        }
        if leaders[i] == i as u32 {
            notes.push(m.new_line);
        }
        let novel = (m.new_line..m.new_line + m.lines)
            .filter(|&line| mostly_novel(doc, Side::New, line))
            .inspect(|&line| report("novel", Side::New, line))
            .count() as u32;
        noise.novel += novel;
        if novel * 3 > m.lines {
            noise.mostly_new += 1;
        }
        for line in [m.old_line.checked_sub(1), Some(m.old_line + m.old_lines)]
            .into_iter()
            .flatten()
        {
            if leftover(doc, line) {
                noise.leftover += 1;
                report("leftover", Side::Old, line);
            }
        }
    }
    let comparison = Comparison::new(doc, ComparisonOptions::default());
    let p = Projection::with_comparison(doc, Mode::Unified, &Expansion::new(doc), &comparison);
    for row in 0..p.len() {
        if let (Some(count), Some(old), Some(name)) = (p.reflow(row), p.fold_old(row), dump) {
            let new = p.line(row, Side::New).unwrap_or(0);
            println!(
                "fold\t{name}:{}\t{} old -> {count} new\t{}",
                new + 1,
                old.len(),
                text(Side::New, new)
            );
        }
        let side = match p.kind[row as usize] {
            RowKind::Added => Side::New,
            RowKind::Removed => Side::Old,
            _ => continue,
        };
        let Some(line) = p.line(row, side) else {
            continue;
        };
        let blank = text(side, line).is_empty();
        let worded = text(side, line)
            .chars()
            .any(|c| c.is_alphanumeric() || c == '_');
        if !blank && worded && doc.semantic_spans(0, side, line).is_empty() {
            noise.silent += 1;
            report(
                match side {
                    Side::Old => "silent-old",
                    Side::New => "silent-new",
                },
                side,
                line,
            );
        }
    }
    noise.notes = notes.len() as u32;
    notes.sort_unstable();
    noise.crowded = notes
        .windows(2)
        .filter(|w| w[1] - w[0] <= MOVE_GROUP_GAP)
        .inspect(|w| report("crowded", Side::New, w[1]))
        .count() as u32;
    noise
}

fn mostly_novel(doc: &DiffDocument, side: Side, line: u32) -> bool {
    let Some(text) = doc.text(0, side).display_line(line) else {
        return false;
    };
    let solid = |s: &str| s.bytes().filter(|b| !b.is_ascii_whitespace()).count();
    let all = solid(text);
    let novel: usize = doc
        .semantic_spans(0, side, line)
        .iter()
        .filter(|s| s.intensity == ChangeIntensity::Novel)
        .map(|s| solid(&text[s.start as usize..s.end as usize]))
        .sum();
    all > 0 && novel * 2 > all
}

fn leftover(doc: &DiffDocument, line: u32) -> bool {
    let Some(text) = doc.text(0, Side::Old).display_line(line) else {
        return false;
    };
    let bare = text
        .split(|c: char| c.is_whitespace() || "{}();".contains(c))
        .all(|word| word.is_empty() || word == "else");
    bare && !text.trim().is_empty() && mostly_novel(doc, Side::Old, line)
}
