use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;

use libloading::Library;
use tree_sitter as ts;
use tree_sitter::StreamingIterator;

use crate::error::{PhosphorError, Result};
use crate::{HighlightKind, HighlightSpan, LanguageId, LanguageMetadata};

#[derive(Debug)]
struct CompiledLanguage {
    language_id: LanguageId,
    language: ts::Language,
    query: ts::Query,
    capture_kinds: Vec<HighlightKind>,
    _pack_library: Option<Rc<Library>>,
}

thread_local! {
    static COMPILED_LANGUAGES: RefCell<HashMap<LanguageId, Rc<CompiledLanguage>>> =
        RefCell::new(HashMap::new());
    static PARSERS: RefCell<HashMap<LanguageId, ts::Parser>> = RefCell::new(HashMap::new());
}

pub(crate) const LANGUAGE_REGISTRY: &[LanguageMetadata] = &[
    LanguageMetadata {
        id: LanguageId::Bash,
        extensions: &["bash", "sh", "zsh"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::C,
        extensions: &["c", "h"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Cpp,
        extensions: &["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Go,
        extensions: &["go"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::JavaScript,
        extensions: &["js", "jsx", "mjs"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Json,
        extensions: &["json"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Nix,
        extensions: &["nix"],
        common: false,
    },
    LanguageMetadata {
        id: LanguageId::Python,
        extensions: &["py", "pyi"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Rust,
        extensions: &["rs"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Toml,
        extensions: &["toml"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::TypeScript,
        extensions: &["ts"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::TypeScriptTsx,
        extensions: &["tsx"],
        common: true,
    },
    LanguageMetadata {
        id: LanguageId::Zig,
        extensions: &["zig"],
        common: false,
    },
];

pub(crate) fn languages() -> &'static [LanguageMetadata] {
    LANGUAGE_REGISTRY
}

pub(crate) fn common_languages() -> impl Iterator<Item = LanguageId> + 'static {
    LANGUAGE_REGISTRY
        .iter()
        .filter(|language| language.common)
        .map(|language| language.id)
}

pub(crate) fn is_parser_available(language: LanguageId) -> bool {
    crate::pack::is_pack_installed(language)
}

pub(crate) fn guess_language(path: &Path) -> Option<LanguageId> {
    let extension = path.extension().and_then(OsStr::to_str)?;
    LANGUAGE_REGISTRY
        .iter()
        .find(|language| {
            language
                .extensions
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
        .map(|language| language.id)
}

pub(crate) fn highlight(language: LanguageId, source: &str) -> Result<Vec<HighlightSpan>> {
    if source.is_empty() {
        return Ok(Vec::new());
    }

    if !is_parser_available(language) {
        return Err(PhosphorError::MissingParser { language });
    }

    let compiled = compiled_language(language)?;
    let tree = parse_source(&compiled, source)?;
    let raw_spans = collect_spans(&compiled, &tree, source);
    compact_spans(language, raw_spans)
}

fn compiled_language(language: LanguageId) -> Result<Rc<CompiledLanguage>> {
    COMPILED_LANGUAGES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(compiled) = cache.get(&language) {
            return Ok(compiled.clone());
        }

        let compiled = Rc::new(compile_language(language)?);
        cache.insert(language, compiled.clone());
        Ok(compiled)
    })
}

fn compile_language(language: LanguageId) -> Result<CompiledLanguage> {
    let Some(pack) =
        crate::pack::load_pack(language).map_err(|error| PhosphorError::LoadParserPack {
            language,
            message: error.to_string(),
        })?
    else {
        return Err(PhosphorError::MissingParser { language });
    };
    let tree_sitter_language = pack.language;
    let query_source = pack.query_fragments.concat();
    let pack_library = Some(pack._library);

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

    Ok(CompiledLanguage {
        language_id: language,
        language: tree_sitter_language,
        query,
        capture_kinds,
        _pack_library: pack_library,
    })
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
