use std::path::Path;

use diffy_syntax::{GrammarStore, LanguageId, SourcePos, SymbolKind};

use super::index::SymbolIndex;
use crate::core::error::Result;
use crate::core::vcs::git::GitService;
use crate::core::vcs::model::RevisionId;

const MAX_TARGETS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePosition {
    pub path: String,
    pub revision: RevisionId,
    pub line: u32,
    pub byte: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefinitionTarget {
    pub path: String,
    pub revision: RevisionId,
    pub line: u32,
    pub column: u32,
    pub len: u32,
    pub kind: Option<SymbolKind>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolution {
    pub word: String,
    pub targets: Vec<DefinitionTarget>,
    pub at_definition: bool,
}

pub fn resolve(
    store: &GrammarStore,
    repo_path: &Path,
    position: &SourcePosition,
    head: &RevisionId,
    index: Option<&SymbolIndex>,
) -> Result<Resolution> {
    let git = GitService::new_with_open(repo_path)?;
    let bytes = git.read_file_bytes_at(&position.revision.id, &position.path)?;
    let source = String::from_utf8_lossy(&bytes);
    let lookup = LanguageId::from_path(&position.path).and_then(|language| {
        store.lookup_definition(&language, &source, position.line, position.byte)
    });
    let (word, at) = match &lookup {
        Some(lookup) => (lookup.word.to_string(), lookup.at),
        None => match word_at(&position.text, position.byte as usize) {
            Some((start, word)) => (
                word.to_owned(),
                SourcePos {
                    line: position.line,
                    column: start as u32,
                },
            ),
            None => return Ok(Resolution::default()),
        },
    };
    let len = word.len() as u32;
    let here = |path: &str, revision: &RevisionId, line: u32, column: u32| {
        path == position.path
            && *revision == position.revision
            && line == at.line
            && column == at.column
    };
    let same_file = |line: u32, column: u32, kind: Option<SymbolKind>| DefinitionTarget {
        path: position.path.clone(),
        revision: position.revision.clone(),
        line,
        column,
        len,
        kind,
    };
    let mut at_definition = false;
    if let Some(lookup) = &lookup {
        if let Some(local) = lookup.local {
            if local == at {
                at_definition = true;
            } else {
                return Ok(Resolution {
                    word,
                    targets: vec![same_file(local.line, local.column, None)],
                    at_definition,
                });
            }
        }
        let mut targets = Vec::new();
        for symbol in lookup.symbols.iter().filter(|symbol| *symbol.name == *word) {
            if here(
                &position.path,
                &position.revision,
                symbol.line,
                symbol.column,
            ) {
                at_definition = true;
            } else {
                targets.push(same_file(symbol.line, symbol.column, Some(symbol.kind)));
            }
        }
        if !targets.is_empty() {
            return Ok(Resolution {
                word,
                targets,
                at_definition,
            });
        }
    }
    let mut targets: Vec<(usize, DefinitionTarget)> = index
        .into_iter()
        .flat_map(|index| index.definitions(&word))
        .filter(|(file, symbol)| !here(&file.path, head, symbol.line, symbol.column))
        .map(|(file, symbol)| {
            (
                shared_dirs(&file.path, &position.path),
                DefinitionTarget {
                    path: file.path.to_string(),
                    revision: head.clone(),
                    line: symbol.line,
                    column: symbol.column,
                    len,
                    kind: Some(symbol.kind),
                },
            )
        })
        .collect();
    targets.sort_by(|(a_score, a), (b_score, b)| {
        b_score
            .cmp(a_score)
            .then_with(|| a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    targets.truncate(MAX_TARGETS);
    if targets.is_empty()
        && let Some(import) = lookup.as_ref().and_then(|lookup| lookup.import)
        && import != at
    {
        return Ok(Resolution {
            word,
            targets: vec![same_file(import.line, import.column, None)],
            at_definition,
        });
    }
    Ok(Resolution {
        word,
        targets: targets.into_iter().map(|(_, target)| target).collect(),
        at_definition,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    pub signature: Vec<String>,
    pub doc: Vec<String>,
    pub path: String,
    pub line: u32,
    pub more: usize,
}

pub fn describe(
    store: &GrammarStore,
    repo_path: &Path,
    position: &SourcePosition,
    head: &RevisionId,
    index: Option<&SymbolIndex>,
) -> Result<Option<Description>> {
    let resolution = resolve(store, repo_path, position, head, index)?;
    let (path, revision, line, more) = match resolution.targets.first() {
        _ if resolution.at_definition => (&position.path, &position.revision, position.line, 0),
        Some(target) => (
            &target.path,
            &target.revision,
            target.line,
            resolution.targets.len() - 1,
        ),
        None => return Ok(None),
    };
    let git = GitService::new_with_open(repo_path)?;
    let bytes = git.read_file_bytes_at(&revision.id, path)?;
    let source = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = source.lines().collect();
    let Some(signature) = signature(&lines, line as usize) else {
        return Ok(None);
    };
    let mut doc = doc_above(&lines, line as usize);
    if doc.is_empty() {
        doc = docstring_below(&lines, line as usize + signature.len());
    }
    Ok(Some(Description {
        signature,
        doc,
        path: path.clone(),
        line,
        more,
    }))
}

const SIGNATURE_LINES: usize = 6;
const DOC_LINES: usize = 8;
const LINE_CHARS: usize = 160;

fn clip(line: &str) -> String {
    match line.char_indices().nth(LINE_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

fn signature(lines: &[&str], start: usize) -> Option<Vec<String>> {
    let mut out: Vec<&str> = Vec::new();
    let mut ended = false;
    for line in lines.get(start..)?.iter().take(SIGNATURE_LINES) {
        if let Some(open) = line.find('{') {
            out.push(line[..open].trim_end());
            ended = true;
            break;
        }
        out.push(line.trim_end());
        let end = line.trim_end();
        if end.ends_with(';') || end.ends_with(':') || end.ends_with('=') || end.ends_with("=>") {
            ended = true;
            break;
        }
    }
    let cut = !ended && lines.len() > start + out.len();
    let indent = out
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()?;
    let mut signature: Vec<String> = out
        .iter()
        .map(|line| clip(line.get(indent..).unwrap_or(line.trim_start())))
        .collect();
    if cut {
        signature.push("…".to_owned());
    }
    Some(signature)
}

fn comment_text(line: &str) -> Option<&str> {
    let line = line.trim();
    for marker in ["///", "//!", "//", "/**", "/*", "*/", "*", "--", ";;", "#"] {
        if let Some(rest) = line.strip_prefix(marker) {
            if marker == "#" && (rest.starts_with('[') || rest.starts_with("![")) {
                return None;
            }
            let rest = rest.strip_suffix("*/").unwrap_or(rest);
            return Some(rest.strip_prefix(' ').unwrap_or(rest).trim_end());
        }
    }
    None
}

fn doc_above(lines: &[&str], line: usize) -> Vec<String> {
    let mut doc = Vec::new();
    for text in lines[..line.min(lines.len())].iter().rev() {
        let trimmed = text.trim();
        if trimmed.starts_with("#[") || trimmed.starts_with('@') {
            continue;
        }
        match comment_text(text) {
            Some(comment) => doc.push(comment),
            None => break,
        }
    }
    doc.reverse();
    tidy(doc)
}

fn docstring_below(lines: &[&str], from: usize) -> Vec<String> {
    let Some(first) = lines.get(from..).and_then(|rest| {
        rest.iter()
            .position(|line| !line.trim().is_empty())
            .map(|i| from + i)
    }) else {
        return Vec::new();
    };
    let opening = lines[first].trim();
    let Some(quote) = ["\"\"\"", "'''"]
        .into_iter()
        .find(|quote| opening.starts_with(quote))
    else {
        return Vec::new();
    };
    let mut doc = Vec::new();
    let mut text = &opening[quote.len()..];
    for next in lines[first + 1..].iter().take(DOC_LINES) {
        if let Some(end) = text.find(quote) {
            doc.push(&text[..end]);
            return tidy(doc);
        }
        doc.push(text);
        text = next.trim();
    }
    if let Some(end) = text.find(quote) {
        doc.push(&text[..end]);
    }
    tidy(doc)
}

fn tidy(doc: Vec<&str>) -> Vec<String> {
    let start = doc.iter().position(|line| !line.trim().is_empty());
    let end = doc.iter().rposition(|line| !line.trim().is_empty());
    match (start, end) {
        (Some(start), Some(end)) => doc[start..=end]
            .iter()
            .take(DOC_LINES)
            .map(|line| clip(line))
            .collect(),
        _ => Vec::new(),
    }
}

fn dirs(path: &str) -> Vec<&str> {
    match path.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').collect(),
        None => Vec::new(),
    }
}

fn shared_dirs(a: &str, b: &str) -> usize {
    dirs(a)
        .into_iter()
        .zip(dirs(b))
        .take_while(|(x, y)| x == y)
        .count()
}

fn word_at(text: &str, byte: usize) -> Option<(usize, &str)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let byte = byte.min(text.len());
    let start = text[..byte]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word(*c))
        .last()
        .map_or(byte, |(index, _)| index);
    let end = text[byte..]
        .char_indices()
        .find(|(_, c)| !is_word(*c))
        .map_or(text.len(), |(index, _)| byte + index);
    (start < end).then(|| (start, &text[start..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words_around_a_byte() {
        assert_eq!(word_at("let total = sum(a);", 6), Some((4, "total")));
        assert_eq!(word_at("let total = sum(a);", 9), Some((4, "total")));
        assert_eq!(word_at("let total = sum(a);", 10), None);
        assert_eq!(word_at("call(x)", 4), Some((0, "call")));
    }

    #[test]
    fn signatures_stop_at_the_body_and_keep_relative_indent() {
        let lines = [
            "    /// Opens the store.",
            "    #[inline]",
            "    pub fn open(",
            "        path: &Path,",
            "    ) -> Store {",
            "        todo!()",
        ];
        assert_eq!(
            signature(&lines, 2),
            Some(vec![
                "pub fn open(".to_owned(),
                "    path: &Path,".to_owned(),
                ") -> Store".to_owned(),
            ])
        );
        assert_eq!(doc_above(&lines, 2), vec!["Opens the store.".to_owned()]);
        assert_eq!(
            signature(&["const LIMIT: usize = 4;"], 0),
            Some(vec!["const LIMIT: usize = 4;".to_owned()])
        );
    }

    #[test]
    fn python_docstrings_follow_the_signature() {
        let lines = [
            "def load(path):",
            "    \"\"\"Read a file.",
            "",
            "    Returns bytes.\"\"\"",
            "    pass",
        ];
        assert_eq!(
            signature(&lines, 0),
            Some(vec!["def load(path):".to_owned()])
        );
        assert_eq!(
            docstring_below(&lines, 1),
            vec![
                "Read a file.".to_owned(),
                "".to_owned(),
                "Returns bytes.".to_owned()
            ]
        );
    }

    #[test]
    fn nearer_directories_rank_first() {
        assert_eq!(shared_dirs("src/app/a.rs", "src/app/b.rs"), 2);
        assert_eq!(shared_dirs("src/core/a.rs", "src/app/b.rs"), 1);
        assert_eq!(shared_dirs("lib.rs", "src/app/b.rs"), 0);
    }
}
