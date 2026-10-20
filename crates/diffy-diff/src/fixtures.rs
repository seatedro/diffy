//! Deterministic large inputs for measurements: generated from a seed so
//! no giant file is checked in. Not part of the stable API.

/// A small xorshift generator: the same seed gives the same text on every
/// machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number below `n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

const WORDS: &[&str] = &[
    "value", "index", "count", "buffer", "result", "offset", "length", "parse", "render", "layout",
    "state", "frame", "cache", "query", "token", "scope", "label", "width", "height", "error",
];

/// About `bytes` bytes of plausible JavaScript, a statement per line.
pub fn javascript(seed: u64, bytes: usize) -> String {
    use std::fmt::Write;
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(bytes + 128);
    while out.len() < bytes {
        let n = rng.next_u64();
        let w = WORDS[(n % WORDS.len() as u64) as usize];
        let _ = match n % 5 {
            0 => writeln!(
                out,
                "const {w}{} = compute({}, \"label {}\");",
                n % 9973,
                n % 101,
                n % 7
            ),
            1 => writeln!(
                out,
                "function {w}{}(a, b) {{ return a + b * {}; }}",
                n % 99991,
                n % 13
            ),
            2 => writeln!(out, "// note {} about the {w}", n % 1_000_003),
            3 => writeln!(out, "if ({w}.count > {}) {{ {w}.count -= 1; }}", n % 5000),
            _ => writeln!(
                out,
                "items.push({{ id: {}, name: `{w} ${{{}}}` }});",
                n % 77777,
                n % 9
            ),
        };
    }
    out
}

/// `old` with about `per_mille`/1000 of its lines edited, inserted after,
/// or deleted, spread over the whole text.
pub fn scattered_edits(old: &str, seed: u64, per_mille: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(old.len() + old.len() / 50);
    for line in old.split_inclusive('\n') {
        if rng.below(1000) >= per_mille {
            out.push_str(line);
            continue;
        }
        match rng.below(3) {
            0 => {
                out.push_str(line.trim_end_matches('\n'));
                out.push_str(" // edited\n");
            }
            1 => {
                out.push_str(line);
                out.push_str(&javascript(rng.next_u64(), 1));
            }
            _ => {}
        }
    }
    out
}

/// `old` with the middle `share` of its bytes (on line boundaries)
/// replaced by unrelated JavaScript of about the same size: one change
/// block.
pub fn one_block(old: &str, seed: u64, share: f64) -> String {
    let len = old.len();
    let cut = |at: usize| old[..at].rfind('\n').map_or(0, |i| i + 1);
    let from = cut(((len as f64) * (0.5 - share / 2.0)) as usize);
    let to = cut(((len as f64) * (0.5 + share / 2.0)) as usize);
    let mut out = String::with_capacity(len + 128);
    out.push_str(&old[..from]);
    out.push_str(&javascript(seed, to - from));
    out.push_str(&old[to..]);
    out
}

/// About `bytes` bytes of minified JavaScript on one line, no newline.
pub fn minified(seed: u64, bytes: usize) -> String {
    use std::fmt::Write;
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(bytes + 128);
    while out.len() < bytes {
        let w = WORDS[rng.below(WORDS.len() as u64) as usize];
        let _ = match rng.below(4) {
            0 => write!(out, "var {w}{}={};", rng.below(100_000), rng.below(1 << 30)),
            1 => write!(
                out,
                "function {w}{}(a,b){{return a+b*{}}}",
                rng.below(1000),
                rng.below(97)
            ),
            2 => write!(out, "{w}.push(\"{}\");", rng.next_u64() % 1_000_000_007),
            _ => write!(out, "if({w}>{}){{{w}--}}", rng.below(5000)),
        };
    }
    out
}

/// `line` with `edits` short replacements at seeded positions, each on a
/// char boundary.
pub fn edit_line(line: &str, seed: u64, edits: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut at: Vec<usize> = (0..edits)
        .map(|_| rng.below(line.len().max(1) as u64) as usize)
        .collect();
    at.sort_unstable();
    let mut out = String::with_capacity(line.len() + edits * 16);
    let mut from = 0;
    for (i, &a) in at.iter().enumerate() {
        let a = line.floor_char_boundary(a).max(from);
        let skip = line.floor_char_boundary((a + 6).min(line.len())).max(a);
        out.push_str(&line[from..a]);
        out.push_str(&format!("EDIT{i}"));
        from = skip;
    }
    out.push_str(&line[from..]);
    out
}

/// Low-entropy lines (braces, blank lines, a few repeated statements):
/// the shape that defeats unique-line shortcuts.
pub fn repetitive(seed: u64, bytes: usize) -> String {
    const LINES: &[&str] = &[
        "}\n",
        "\n",
        "    return Ok(());\n",
        "{\n",
        "        break;\n",
        "    }\n",
        "    let x = 0;\n",
        "        x += 1;\n",
    ];
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(bytes + 32);
    while out.len() < bytes {
        out.push_str(LINES[rng.below(LINES.len() as u64) as usize]);
    }
    out
}

/// `old` with one line replaced every `every` lines: many small hunks.
pub fn every_nth(old: &str, seed: u64, every: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::with_capacity(old.len() + old.len() / every);
    for (i, line) in old.split_inclusive('\n').enumerate() {
        if i % every == every / 2 {
            out.push_str(&format!("changed {}\n", rng.below(8)));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Peak resident memory of this process since the last
/// [`reset_peak_rss`], in bytes, where the platform reports it (Linux).
pub fn peak_rss() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// Resets [`peak_rss`] to the current resident size (Linux).
pub fn reset_peak_rss() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}
