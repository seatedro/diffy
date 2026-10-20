mod align;
mod config;
mod cost;
#[cfg(feature = "eval")]
mod legacy;
mod matcher;
mod syntax;

use std::fmt;
use std::ops::Range;
use std::time::{Duration, Instant};

use diffy_diff::SemanticSpan;
use tree_sitter as ts;

pub use config::{Config, config};
pub use diffy_diff::{ItemChange, Move, OutlineItem, Rename};
pub use tree_sitter::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub spans: [Vec<SemanticSpan>; 2],
}

#[derive(Debug, Clone, Default)]
pub struct SemanticDiff {
    pub chunks: Vec<Vec<Row>>,
    pub outline: Vec<OutlineItem>,
    pub renames: Vec<Rename>,
    pub moves: Vec<Move>,
    /// Lines joined by corresponding tokens across more than one line on
    /// either side: a reflow, possibly with edits.
    pub groups: Vec<Group>,
    /// Every token of each side, list delimiters included, in source order.
    pub tokens: [Vec<Token>; 2],
}

/// Old and new lines that hold corresponding tokens, as half-open ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub old: Range<u32>,
    pub new: Range<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// Corresponds to an equal token on the other side.
    Kept,
    /// Corresponds to a token renamed consistently throughout.
    Renamed,
    /// A string or comment corresponding to one with changed words.
    Edited,
    /// No counterpart.
    Novel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub span: Range<u32>,
    pub state: TokenState,
    /// The corresponding token's span on the other side.
    pub partner: Option<Range<u32>>,
    /// The move holding this token, by index into the moves.
    pub moved: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_errors: usize,
    pub max_work: u64,
    pub max_time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1_000_000,
            max_errors: 100,
            max_work: 20_000_000,
            max_time: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    TooLarge(usize),
    ParseErrors(usize),
    ErrorCoverage(usize),
    Unparsed,
    Budget,
    Timeout,
}

impl fmt::Display for Fallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(bytes) => write!(f, "{bytes} bytes exceeds the semantic byte limit"),
            Self::ParseErrors(count) => write!(f, "{count} parse errors"),
            Self::ErrorCoverage(percent) => {
                write!(f, "parse errors cover {percent}% of the file")
            }
            Self::Unparsed => f.write_str("the parser gave up"),
            Self::Budget => f.write_str("exceeded the semantic work budget"),
            Self::Timeout => f.write_str("exceeded the semantic time budget"),
        }
    }
}

impl std::error::Error for Fallback {}

pub fn diff(
    language: &ts::Language,
    config: &Config,
    old: &str,
    new: &str,
    limits: &Limits,
) -> Result<SemanticDiff, Fallback> {
    #[cfg(feature = "eval")]
    let options = eval::current();
    #[cfg(not(feature = "eval"))]
    let options = matcher::Options::default();
    run(
        language,
        config,
        old,
        new,
        limits,
        &options,
        |_, _, _, diff| diff,
    )
}

fn run<T>(
    language: &ts::Language,
    config: &Config,
    old: &str,
    new: &str,
    limits: &Limits,
    options: &matcher::Options,
    finish: impl FnOnce(&syntax::Tree<'_>, &syntax::Tree<'_>, matcher::Matching, SemanticDiff) -> T,
) -> Result<T, Fallback> {
    let deadline = Instant::now() + limits.max_time;
    let bytes = old.len().max(new.len());
    if bytes > limits.max_bytes {
        return Err(Fallback::TooLarge(bytes));
    }
    let mut parser = ts::Parser::new();
    parser
        .set_language(language)
        .map_err(|_| Fallback::Unparsed)?;
    let old_tree = parser.parse(old, None).ok_or(Fallback::Unparsed)?;
    let new_tree = parser.parse(new, None).ok_or(Fallback::Unparsed)?;
    for (tree, text) in [(&old_tree, old), (&new_tree, new)] {
        let covered = error_bytes(tree.root_node());
        if covered * 100 > text.len() * MAX_ERROR_PERCENT {
            return Err(Fallback::ErrorCoverage(covered * 100 / text.len().max(1)));
        }
    }
    let old = syntax::Tree::lower(&old_tree, old, config);
    let new = syntax::Tree::lower(&new_tree, new, config);
    let errors = old.errors + new.errors;
    if errors > limits.max_errors {
        return Err(Fallback::ParseErrors(errors));
    }
    #[cfg(feature = "eval")]
    let matching = if options.legacy {
        legacy::run(&old, &new, limits.max_work, deadline)?
    } else {
        matcher::run(&old, &new, limits.max_work, deadline, options)?
    };
    #[cfg(not(feature = "eval"))]
    let matching = matcher::run(&old, &new, limits.max_work, deadline, options)?;
    let out = align::rows(&old, &new, &matching);
    let diff = SemanticDiff {
        chunks: out.chunks,
        outline: out.outline,
        renames: out.renames,
        moves: out.moves,
        groups: out.groups,
        tokens: out.tokens,
    };
    Ok(finish(&old, &new, matching, diff))
}

#[cfg(feature = "eval")]
pub mod eval {
    pub use crate::cost::{Features, TERMS, Weights};
    use crate::{Config, Fallback, Limits, SemanticDiff, cost, matcher, ts};

    #[derive(Debug, Clone, Copy)]
    pub struct Options {
        pub weights: Weights,
        pub candidates: usize,
        pub floor: bool,
        pub exhaustive: bool,
        pub margins: bool,
        pub legacy: bool,
    }

    const DEFAULT: Options = Options {
        weights: Weights::DEFAULT,
        candidates: 4,
        floor: true,
        exhaustive: false,
        margins: false,
        legacy: false,
    };

    impl Default for Options {
        fn default() -> Self {
            DEFAULT
        }
    }

    static CURRENT: std::sync::Mutex<Options> = std::sync::Mutex::new(DEFAULT);

    pub fn set(options: Options) {
        if let Ok(mut current) = CURRENT.lock() {
            *current = options;
        }
    }

    pub fn get() -> Options {
        CURRENT.lock().map(|o| *o).unwrap_or_default()
    }

    pub(crate) fn current() -> matcher::Options {
        inner(&get())
    }

    fn inner(options: &Options) -> matcher::Options {
        matcher::Options {
            weights: options.weights,
            candidates: options.candidates,
            floor: options.floor,
            exhaustive: options.exhaustive,
            margins: options.margins,
            legacy: options.legacy,
        }
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Margin {
        pub kind: &'static str,
        pub old: u32,
        pub new: u32,
        pub best: i64,
        pub runner: i64,
    }

    pub struct Explained {
        pub diff: SemanticDiff,
        pub delimiters: [std::collections::HashSet<u32>; 2],
        pub baseline: Features,
        pub predicted: Features,
        pub actual: Features,
        pub repairs: u32,
        pub margins: Vec<Margin>,
        pub work: u64,
    }

    impl Explained {
        pub fn cost(&self, weights: &Weights) -> i64 {
            weights.cost(&self.actual)
        }

        pub fn agrees(&self) -> bool {
            let mut expected = self.baseline;
            expected -= self.predicted;
            expected == self.actual
        }
    }

    pub fn explain(
        language: &ts::Language,
        config: &Config,
        old: &str,
        new: &str,
        limits: &Limits,
        options: &Options,
    ) -> Result<Explained, Fallback> {
        crate::run(
            language,
            config,
            old,
            new,
            limits,
            &inner(options),
            |old, new, m, diff| {
                let mut baseline = Features::default();
                for tree in [old, new] {
                    baseline[cost::UNMATCHED] += tree
                        .nodes
                        .iter()
                        .map(|n| cost::weight(tree, n))
                        .sum::<i64>();
                }
                let actual = cost::evaluate(
                    [old, new],
                    &m.partner,
                    m.moves.len(),
                    &diff.tokens,
                    &diff.renames,
                );
                let delimiters = [old, new].map(|tree| {
                    tree.nodes
                        .iter()
                        .flat_map(|n| [n.open.clone(), n.close.clone()])
                        .filter(|r| !r.is_empty())
                        .map(|r| r.start)
                        .collect()
                });
                Explained {
                    diff,
                    delimiters,
                    baseline,
                    predicted: m.predicted,
                    actual,
                    repairs: m.repairs,
                    margins: m
                        .margins
                        .iter()
                        .map(|g| Margin {
                            kind: g.kind,
                            old: g.old,
                            new: g.new,
                            best: g.best,
                            runner: g.runner,
                        })
                        .collect(),
                    work: m.work,
                }
            },
        )
    }
}

const MAX_ERROR_PERCENT: usize = 20;

fn error_bytes(node: ts::Node<'_>) -> usize {
    if node.is_error() {
        return node.end_byte() - node.start_byte();
    }
    if !node.has_error() {
        return 0;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).map(error_bytes).sum()
}
