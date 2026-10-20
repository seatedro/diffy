use std::path::PathBuf;

use diffy::core::compare::semantic::semantic_language;
use diffy_semantic::{Limits, SemanticDiff, Token};

use crate::corpus::Blobs;
use crate::engine;
use crate::explain::{self, Checks};
use crate::grade::line_of;

#[derive(clap::Args)]
pub struct Args {
    file: PathBuf,
    #[arg(long)]
    verbose: bool,
}

struct Case {
    label: String,
    path: String,
    old: String,
    new: String,
    claims: Vec<(usize, String)>,
}

pub fn run(args: Args) -> Result<(), String> {
    let text = std::fs::read_to_string(&args.file).map_err(|e| e.to_string())?;
    let mut cases: Vec<Case> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["case", repo, rev, path] => {
                let mut blobs = Blobs::spawn(std::path::Path::new(repo))?;
                let read = |blobs: &mut Blobs, rev: &str| -> Result<String, String> {
                    let bytes = blobs.read(rev, path)?.ok_or(format!("{rev}:{path} missing"))?;
                    String::from_utf8(bytes).map_err(|e| e.to_string())
                };
                let old = read(&mut blobs, &format!("{rev}^"))?;
                let new = read(&mut blobs, rev)?;
                cases.push(Case {
                    label: format!("{rev} {path}"),
                    path: (*path).to_owned(),
                    old,
                    new,
                    claims: Vec::new(),
                });
            }
            ["files", old, new] => {
                let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
                cases.push(Case {
                    label: format!("{old} -> {new}"),
                    path: (*new).to_owned(),
                    old: read(old)?,
                    new: read(new)?,
                    claims: Vec::new(),
                });
            }
            _ => match cases.last_mut() {
                Some(case) => case.claims.push((n + 1, line.to_owned())),
                None => return Err(format!("line {}: claim before any case", n + 1)),
            },
        }
    }
    let limits = Limits::default();
    let mut checks = Checks::default();
    let (mut held, mut failed, mut cases_failed) = (0, 0, 0);
    for case in &cases {
        let name = semantic_language(&case.path).ok_or("no grammar")?;
        let (Some(language), Some(config)) =
            (engine::language(name), diffy_semantic::config(name))
        else {
            return Err(format!("{}: no grammar", case.path));
        };
        let explained = match explain::run(&language, config, &case.old, &case.new, &limits) {
            Ok(e) => e,
            Err(fallback) => {
                println!("FALLBACK {}: {fallback}", case.label);
                failed += case.claims.len();
                cases_failed += 1;
                continue;
            }
        };
        checks.note(&case.label, &explained);
        let view = View {
            diff: &explained.diff,
            texts: [&case.old, &case.new],
        };
        let mut bad = Vec::new();
        for (n, claim) in &case.claims {
            let holds = claim
                .split('|')
                .map(|alt| view.holds(alt.trim()))
                .collect::<Result<Vec<bool>, String>>()
                .map_err(|e| format!("{}:{n}: {e}", args.file.display()))?
                .into_iter()
                .any(|h| h);
            if holds {
                held += 1;
            } else {
                failed += 1;
                bad.push(claim);
            }
        }
        if !bad.is_empty() {
            cases_failed += 1;
            println!("FAIL {}", case.label);
            for claim in bad {
                println!("  {claim}");
            }
        } else if args.verbose {
            println!("ok   {}", case.label);
        }
    }
    println!(
        "{} cases, {cases_failed} failing; {held} claims hold, {failed} fail",
        cases.len()
    );
    checks.report()?;
    if failed > 0 {
        return Err(format!("{failed} gold claims fail"));
    }
    Ok(())
}

struct View<'a> {
    diff: &'a SemanticDiff,
    texts: [&'a str; 2],
}

impl View<'_> {
    fn side(word: &str) -> Result<usize, String> {
        match word {
            "old" => Ok(0),
            "new" => Ok(1),
            _ => Err(format!("expected old or new, got {word}")),
        }
    }

    fn lines(spec: &str) -> Result<(usize, usize), String> {
        let parse = |s: &str| s.parse::<usize>().map_err(|_| format!("bad line {s}"));
        match spec.split_once('-') {
            Some((a, b)) => Ok((parse(a)?, parse(b)?)),
            None => Ok((parse(spec)?, parse(spec)?)),
        }
    }

    fn line(&self, side: usize, token: &Token) -> usize {
        line_of(self.texts[side], token.span.start) + 1
    }

    fn text(&self, side: usize, token: &Token) -> &str {
        &self.texts[side][token.span.start as usize..token.span.end as usize]
    }

    fn worded(&self, side: usize, token: &Token) -> bool {
        self.text(side, token)
            .chars()
            .any(|c| c.is_alphanumeric() || c == '_')
    }

    fn find(&self, side: usize, spec: &str) -> Result<&Token, String> {
        let (line, text) = spec.split_once(':').ok_or(format!("expected LINE:TEXT, got {spec}"))?;
        let line: usize = line.parse().map_err(|_| format!("bad line {line}"))?;
        self.diff.tokens[side]
            .iter()
            .find(|t| self.line(side, t) == line && self.text(side, t).starts_with(text))
            .ok_or(format!("no token {text} on {} line {line}", ["old", "new"][side]))
    }

    fn range(&self, side: usize, (a, b): (usize, usize)) -> impl Iterator<Item = &Token> {
        self.diff.tokens[side].iter().filter(move |t| {
            let line = self.line(side, t);
            (a..=b).contains(&line) && self.worded(side, t)
        })
    }

    fn holds(&self, claim: &str) -> Result<bool, String> {
        let w: Vec<&str> = claim.split_whitespace().collect();
        let partner_line = |t: &Token| {
            t.partner
                .as_ref()
                .map(|p| line_of(self.texts[1], p.start) + 1)
        };
        Ok(match w.as_slice() {
            ["link", spec, line] => {
                let line: usize = line.parse().map_err(|_| format!("bad line {line}"))?;
                partner_line(self.find(0, spec)?) == Some(line)
            }
            ["notlink", spec, line] => {
                let line: usize = line.parse().map_err(|_| format!("bad line {line}"))?;
                partner_line(self.find(0, spec)?) != Some(line)
            }
            ["unlinked", side, spec] => self.find(Self::side(side)?, spec)?.partner.is_none(),
            ["kept", olds, news] | ["kept90", olds, news] => {
                let (olds, news) = (Self::lines(olds)?, Self::lines(news)?);
                let tokens: Vec<&Token> = self.range(0, olds).collect();
                let hits = tokens
                    .iter()
                    .filter(|t| partner_line(t).is_some_and(|l| (news.0..=news.1).contains(&l)))
                    .count();
                if w[0] == "kept" {
                    hits == tokens.len()
                } else {
                    hits * 10 >= tokens.len() * 9
                }
            }
            ["novel", side, lines] => {
                let side = Self::side(side)?;
                self.range(side, Self::lines(lines)?)
                    .all(|t| t.partner.is_none())
            }
            ["moved", side, lines] => {
                let side = Self::side(side)?;
                self.range(side, Self::lines(lines)?)
                    .all(|t| t.moved.is_some())
            }
            ["inplace", side, lines] => {
                let side = Self::side(side)?;
                self.range(side, Self::lines(lines)?)
                    .all(|t| t.moved.is_none())
            }
            ["nomoves"] => self.diff.moves.is_empty(),
            ["rename", p, q] => self
                .diff
                .renames
                .iter()
                .any(|r| &*r.old == *p && &*r.new == *q),
            ["norename", p, q] => !self
                .diff
                .renames
                .iter()
                .any(|r| &*r.old == *p && &*r.new == *q),
            _ => return Err(format!("unknown claim {claim}")),
        })
    }
}
