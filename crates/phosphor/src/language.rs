use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;

use tree_sitter as ts;
use tree_sitter::StreamingIterator;
use tree_sitter_language::LanguageFn;

use crate::error::{PhosphorError, Result};
use crate::{HighlightKind, HighlightSpan, LanguageId};

#[derive(Clone, Copy)]
struct LanguageSpec {
    id: LanguageId,
    extensions: &'static [&'static str],
    language_fn: LanguageFn,
    query_fragments: &'static [&'static str],
}

#[derive(Debug)]
struct CompiledLanguage {
    language_id: LanguageId,
    language: ts::Language,
    query: ts::Query,
    capture_kinds: Vec<HighlightKind>,
}

thread_local! {
    static COMPILED_LANGUAGES: RefCell<HashMap<LanguageId, Rc<CompiledLanguage>>> =
        RefCell::new(HashMap::new());
    static PARSERS: RefCell<HashMap<LanguageId, ts::Parser>> = RefCell::new(HashMap::new());
}

const LANGUAGE_SPECS: &[LanguageSpec] = &[
    LanguageSpec {
        id: LanguageId::Bash,
        extensions: &["bash", "sh", "zsh"],
        language_fn: tree_sitter_bash::LANGUAGE,
        query_fragments: &[tree_sitter_bash::HIGHLIGHT_QUERY],
    },
    LanguageSpec {
        id: LanguageId::C,
        extensions: &["c", "h"],
        language_fn: tree_sitter_c::LANGUAGE,
        query_fragments: &[tree_sitter_c::HIGHLIGHT_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Cpp,
        extensions: &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        language_fn: tree_sitter_cpp::LANGUAGE,
        query_fragments: &[tree_sitter_cpp::HIGHLIGHT_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Go,
        extensions: &["go"],
        language_fn: tree_sitter_go::LANGUAGE,
        query_fragments: &[tree_sitter_go::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::JavaScript,
        extensions: &["js", "jsx", "mjs"],
        language_fn: tree_sitter_javascript::LANGUAGE,
        query_fragments: &[tree_sitter_javascript::HIGHLIGHT_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Json,
        extensions: &["json"],
        language_fn: tree_sitter_json::LANGUAGE,
        query_fragments: &[tree_sitter_json::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Nix,
        extensions: &["nix"],
        language_fn: tree_sitter_nix::LANGUAGE,
        query_fragments: &[tree_sitter_nix::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Python,
        extensions: &["py", "pyi"],
        language_fn: tree_sitter_python::LANGUAGE,
        query_fragments: &[tree_sitter_python::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Rust,
        extensions: &["rs"],
        language_fn: tree_sitter_rust_orchard::LANGUAGE,
        query_fragments: &[tree_sitter_rust_orchard::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::Toml,
        extensions: &["toml"],
        language_fn: tree_sitter_toml_ng::LANGUAGE,
        query_fragments: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
    },
    LanguageSpec {
        id: LanguageId::TypeScript,
        extensions: &["ts"],
        language_fn: tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        query_fragments: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
    },
    LanguageSpec {
        id: LanguageId::TypeScriptTsx,
        extensions: &["tsx"],
        language_fn: tree_sitter_typescript::LANGUAGE_TSX,
        query_fragments: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
    },
    LanguageSpec {
        id: LanguageId::Zig,
        extensions: &["zig"],
        language_fn: tree_sitter_zig::LANGUAGE,
        query_fragments: &[tree_sitter_zig::HIGHLIGHTS_QUERY],
    },
];

pub(crate) fn guess_language(path: &Path) -> Option<LanguageId> {
    let extension = path.extension().and_then(OsStr::to_str)?;
    LANGUAGE_SPECS
        .iter()
        .find(|spec| {
            spec.extensions
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
        .map(|spec| spec.id)
}

pub(crate) fn highlight(language: LanguageId, source: &str) -> Result<Vec<HighlightSpan>> {
    if source.is_empty() {
        return Ok(Vec::new());
    }

    let compiled = compiled_language(language);
    let tree = parse_source(&compiled, source)?;
    let raw_spans = collect_spans(&compiled, &tree, source);
    compact_spans(language, raw_spans)
}

fn compiled_language(language: LanguageId) -> Rc<CompiledLanguage> {
    COMPILED_LANGUAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache
            .entry(language)
            .or_insert_with(|| Rc::new(compile_language(language)))
            .clone()
    })
}

fn compile_language(language: LanguageId) -> CompiledLanguage {
    let spec = language_spec(language);
    let tree_sitter_language = ts::Language::new(spec.language_fn);
    let query_source = spec.query_fragments.concat();
    let query = ts::Query::new(&tree_sitter_language, &query_source).unwrap_or_else(|error| {
        panic!(
            "invalid phosphor highlight query for {}: {error}",
            language.name()
        )
    });
    let capture_kinds = query
        .capture_names()
        .iter()
        .map(|name| capture_name_to_highlight_kind(name))
        .collect();

    CompiledLanguage {
        language_id: language,
        language: tree_sitter_language,
        query,
        capture_kinds,
    }
}

fn parse_source(compiled: &CompiledLanguage, source: &str) -> Result<ts::Tree> {
    PARSERS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let parser = match cache.entry(compiled.language_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut parser = ts::Parser::new();
                parser.set_language(&compiled.language).map_err(|error| {
                    PhosphorError::InitParser {
                        language: compiled.language_id,
                        message: error.to_string(),
                    }
                })?;
                entry.insert(parser)
            }
        };

        let tree = parser
            .parse(source, None)
            .ok_or(PhosphorError::ParseFailed {
                language: compiled.language_id,
            })?;
        Ok(tree)
    })
}

fn collect_spans(
    compiled: &CompiledLanguage,
    tree: &ts::Tree,
    source: &str,
) -> Vec<(usize, usize, HighlightKind, usize)> {
    let mut cursor = ts::QueryCursor::new();
    let mut captures = cursor.captures(&compiled.query, tree.root_node(), source.as_bytes());
    let mut raw_spans = Vec::new();

    while let Some((query_match, capture_index)) = captures.next() {
        let capture = query_match.captures[*capture_index];
        let kind = compiled
            .capture_kinds
            .get(capture.index as usize)
            .copied()
            .unwrap_or(HighlightKind::Normal);
        if kind == HighlightKind::Normal {
            continue;
        }

        let node = capture.node;
        let start = node.start_byte();
        let end = node.end_byte();
        if end > start {
            raw_spans.push((start, end, kind, query_match.pattern_index));
        }
    }

    raw_spans
}

fn compact_spans(
    language: LanguageId,
    mut raw_spans: Vec<(usize, usize, HighlightKind, usize)>,
) -> Result<Vec<HighlightSpan>> {
    let _ = language;
    raw_spans.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| right.3.cmp(&left.3)));

    let raw_span_count = raw_spans.len();
    let mut covered = 0usize;
    let mut spans = Vec::with_capacity(raw_span_count);
    for (start, end, kind, _) in raw_spans {
        if start < covered {
            continue;
        }
        spans.push(HighlightSpan {
            offset: start as u32,
            length: (end - start) as u32,
            kind,
        });
        covered = end;
    }

    Ok(spans)
}

fn language_spec(language: LanguageId) -> &'static LanguageSpec {
    LANGUAGE_SPECS
        .iter()
        .find(|spec| spec.id == language)
        .unwrap_or_else(|| panic!("missing phosphor language spec for {}", language.name()))
}

fn capture_name_to_highlight_kind(name: &str) -> HighlightKind {
    if name.starts_with("keyword") {
        HighlightKind::Keyword
    } else if name.starts_with("string") || name.starts_with("escape") {
        HighlightKind::String
    } else if name.starts_with("comment") {
        HighlightKind::Comment
    } else if name.starts_with("number") {
        HighlightKind::Number
    } else if name.starts_with("type") || name.starts_with("constructor") {
        HighlightKind::Type
    } else if name.starts_with("function") {
        HighlightKind::Function
    } else if name.starts_with("operator") {
        HighlightKind::Operator
    } else if name.starts_with("punctuation") {
        HighlightKind::Punctuation
    } else if name.starts_with("variable") || name.starts_with("parameter") {
        HighlightKind::Variable
    } else if name.starts_with("constant") || name.starts_with("boolean") {
        HighlightKind::Constant
    } else if name.starts_with("builtin") {
        HighlightKind::Builtin
    } else if name.starts_with("attribute") {
        HighlightKind::Attribute
    } else if name.starts_with("tag") {
        HighlightKind::Tag
    } else if name.starts_with("property") {
        HighlightKind::Property
    } else if name.starts_with("namespace") {
        HighlightKind::Namespace
    } else if name.starts_with("label") {
        HighlightKind::Label
    } else if name.starts_with("preproc") {
        HighlightKind::Preprocessor
    } else {
        HighlightKind::Normal
    }
}
