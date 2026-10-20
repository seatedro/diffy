use std::path::PathBuf;

use diffy_diff::{ChangeIntensity, DiffDocument, MOVE_GROUP_GAP, Side, move_leaders};

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
}

impl Noise {
    fn score(&self) -> u32 {
        self.novel + 10 * self.mostly_new + 2 * self.crowded + self.tiny + 2 * self.leftover
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
        let noise = measure(&run.doc);
        files += 1;
        total.add(&noise);
        scored.push((noise, format!("{commit} {path}")));
    }
    scored.sort_by_key(|(noise, _)| std::cmp::Reverse(noise.score()));
    println!("score moves notes novel mostly tiny crowded leftover  file");
    for (n, name) in scored.iter().take(args.worst) {
        println!(
            "{:5} {:5} {:5} {:5} {:6} {:4} {:7} {:8}  {name}",
            n.score(),
            n.moves,
            n.notes,
            n.novel,
            n.mostly_new,
            n.tiny,
            n.crowded,
            n.leftover
        );
    }
    println!(
        "{files} files: {} moves over {} lines, {} notes; {} novel lines inside moves, {} mostly new moves, {} one-line moves, {} crowded notes, {} leftover lines",
        total.moves,
        total.moved_lines,
        total.notes,
        total.novel,
        total.mostly_new,
        total.tiny,
        total.crowded,
        total.leftover
    );
    Ok(())
}

fn measure(doc: &DiffDocument) -> Noise {
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
        }
        if leaders[i] == i as u32 {
            notes.push(m.new_line);
        }
        let novel = (m.new_line..m.new_line + m.lines)
            .filter(|&line| mostly_novel(doc, Side::New, line))
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
            }
        }
    }
    noise.notes = notes.len() as u32;
    notes.sort_unstable();
    noise.crowded = notes
        .windows(2)
        .filter(|w| w[1] - w[0] <= MOVE_GROUP_GAP)
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
