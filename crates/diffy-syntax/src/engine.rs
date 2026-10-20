//! Loading grammars from packs, parsing, and query evaluation, including
//! languages embedded in others (tree-sitter injections).
//!
//! A [`Grammar`] is shared by every thread; parsers are not `Sync`, so each
//! thread keeps its own per grammar.
//!
//! [`highlight`] parses the host layer, runs its grammar's injection query
//! to find embedded regions, and parses each region as a layer of its own
//! over the original source (tree-sitter included ranges), so every node
//! keeps its byte and row coordinates. Layers are processed breadth first,
//! one parse at a time, so one parser per grammar serves every layer.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tree_sitter as ts;
use tree_sitter::StreamingIterator;

use crate::pack::{PackError, PackManifest, supported_abi, verify_files};
use crate::store::Tag;
use crate::{HighlightKind, HighlightSpan, LanguageId};

/// A loaded language: its tree-sitter grammar, compiled highlight query,
/// and compiled injection query when its pack has one.
pub(crate) struct Grammar {
    id: u64,
    name: Option<Arc<str>>,
    language: ts::Language,
    highlights: Arc<str>,
    query: ts::Query,
    capture_kinds: Vec<HighlightKind>,
    /// `@none`, which clears the colors of the captures around it (Markdown
    /// uses it so a fenced block's content is not all `@text.literal`).
    none: Option<u32>,
    injections: Option<Injections>,
    scopes: std::sync::OnceLock<Option<Arc<crate::scope::ScopeQuery>>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static PARSERS: RefCell<HashMap<u64, ts::Parser>> = RefCell::new(HashMap::new());
}

impl Grammar {
    /// Verifies the pack's files against `manifest` (already validated),
    /// loads its library, and compiles its highlight and injection queries.
    ///
    /// The library stays loaded for the rest of the process: tree-sitter
    /// languages, parsers, and trees on other threads point into it, and
    /// nothing tracks when the last of them is gone.
    pub(crate) fn load(dir: &Path, manifest: &PackManifest) -> Result<Self, PackError> {
        verify_files(dir, manifest)?;
        let highlights = std::fs::read_to_string(dir.join(&manifest.highlights.path))?;
        let injections = match &manifest.injections {
            Some(file) => Some(std::fs::read_to_string(dir.join(&file.path))?),
            None => None,
        };
        let grammar = Self::compile(
            &dir.join(&manifest.library.path),
            &manifest.symbol,
            Some(manifest.abi),
            &highlights,
            injections.as_deref(),
        )?;
        Ok(Self {
            name: Some(Arc::from(manifest.language.as_str())),
            ..grammar
        })
    }

    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub(crate) fn language(&self) -> &ts::Language {
        &self.language
    }

    pub(crate) fn highlights(&self) -> &Arc<str> {
        &self.highlights
    }

    pub(crate) fn compile(
        library_path: &Path,
        symbol: &str,
        expected_abi: Option<u32>,
        highlights: &str,
        injections: Option<&str>,
    ) -> Result<Self, PackError> {
        // SAFETY: loading a library runs its initializers, and calling the
        // symbol runs its code. The pack passed its SHA-256 check against a
        // manifest that is either signed (downloaded packs) or in a
        // directory the app opted into (local packs); see the threat model
        // in the crate docs.
        let language = unsafe {
            let library = libloading::Library::new(library_path)
                .map_err(|e| PackError::Library(e.to_string()))?;
            let library: &'static libloading::Library = Box::leak(Box::new(library));
            let mut name = symbol.as_bytes().to_vec();
            name.push(0);
            let symbol = library
                .get::<unsafe extern "C" fn() -> *const ()>(&name)
                .map_err(|e| PackError::Library(e.to_string()))?;
            ts::Language::new(tree_sitter_language::LanguageFn::from_raw(*symbol))
        };
        // The manifest's ABI was checked; this checks the library agrees.
        let abi = language.abi_version() as u32;
        if expected_abi.is_some_and(|expected| expected != abi) || !supported_abi().contains(&abi) {
            let range = supported_abi();
            return Err(PackError::Abi {
                abi,
                min: *range.start(),
                max: *range.end(),
            });
        }
        let query =
            lenient_query(&language, highlights).map_err(|e| PackError::Query(e.to_string()))?;
        let capture_kinds = query
            .capture_names()
            .iter()
            .map(|name| capture_name_to_highlight_kind(name))
            .collect();
        let none = query.capture_index_for_name("none");
        let injections = injections
            .map(|text| Injections::compile(&language, text))
            .transpose()?;
        Ok(Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: None,
            language,
            highlights: Arc::from(highlights),
            query,
            none,
            capture_kinds,
            injections,
            scopes: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn scope_query(&self, language: &str) -> Option<Arc<crate::scope::ScopeQuery>> {
        self.scopes
            .get_or_init(|| {
                crate::scope::ScopeQuery::compile(&self.language, language).map(Arc::new)
            })
            .clone()
    }

    pub(crate) fn parse_document(
        &self,
        source: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<ts::Tree> {
        self.parse(source, &[], cancelled)
    }

    /// Parses `source`, reading only `ranges` when there are any; `None`
    /// when the parse failed or `cancelled` stopped it.
    fn parse(
        &self,
        source: &str,
        ranges: &[ts::Range],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<ts::Tree> {
        PARSERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let parser = match cache.entry(self.id) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let mut parser = ts::Parser::new();
                    parser.set_language(&self.language).ok()?;
                    entry.insert(parser)
                }
            };
            // An empty list resets the parser to the whole document.
            parser.set_included_ranges(ranges).ok()?;
            let bytes = source.as_bytes();
            let mut progress = |_: &ts::ParseState| stop_if(cancelled());
            let options = ts::ParseOptions::new().progress_callback(&mut progress);
            let tree = parser.parse_with_options(
                &mut |i, _| bytes.get(i..).unwrap_or_default(),
                None,
                Some(options),
            );
            if tree.is_none() {
                // Unless reset, a stopped parse would resume where it left
                // off on the next call, which parses other text.
                parser.reset();
            }
            tree
        })
    }

    /// `(start, end, kind, pattern)` for every capture with a highlight
    /// kind, and the ranges of `@none` captures.
    /// `cancelled` stops the query early, leaving the lists partial.
    fn collect_spans(
        &self,
        tree: &ts::Tree,
        source: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> (Vec<Captured>, Vec<(usize, usize)>) {
        let mut cursor = ts::QueryCursor::new();
        let mut progress = |_: &ts::QueryCursorState| stop_if(cancelled());
        let options = ts::QueryCursorOptions::new().progress_callback(&mut progress);
        let mut captures =
            cursor.captures_with_options(&self.query, tree.root_node(), source.as_bytes(), options);
        let (mut raw, mut clears) = (Vec::new(), Vec::new());
        while let Some((query_match, capture_index)) = captures.next() {
            let capture = query_match.captures[*capture_index];
            if Some(capture.index) == self.none {
                clears.push((capture.node.start_byte(), capture.node.end_byte()));
                continue;
            }
            let kind = self
                .capture_kinds
                .get(capture.index as usize)
                .copied()
                .unwrap_or_default();
            let (start, end) = (capture.node.start_byte(), capture.node.end_byte());
            if kind != HighlightKind::Normal && end > start {
                raw.push((start, end, kind, query_match.pattern_index));
            }
        }
        clears.sort_unstable();
        (raw, clears)
    }
}

/// A grammar's compiled injection query and what each pattern sets, per
/// <https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html#language-injection>.
struct Injections {
    query: ts::Query,
    /// `@injection.content`: the nodes whose text is the embedded document.
    content: Option<u32>,
    /// `@injection.language`: a node whose text names the language.
    language: Option<u32>,
    patterns: Vec<InjectionPattern>,
}

#[derive(Default)]
struct InjectionPattern {
    /// `#set! injection.language "name"`.
    language: Option<LanguageId>,
    /// `injection.combined`: every match of the pattern (per language) is
    /// one document, as for template fragments split by interpolations.
    combined: bool,
    /// `injection.include-children`: the content nodes' children are part
    /// of the document; by default their named children's text is cut out
    /// of it.
    include_children: bool,
    /// `injection.self`: the language of the layer the match is in.
    itself: bool,
    /// `injection.parent`: the language of the layer that embedded that one.
    parent: bool,
}

impl Injections {
    fn compile(language: &ts::Language, text: &str) -> Result<Self, PackError> {
        let query =
            lenient_query(language, text).map_err(|e| PackError::InjectionQuery(e.to_string()))?;
        let patterns = (0..query.pattern_count())
            .map(|index| {
                let mut pattern = InjectionPattern::default();
                for property in query.property_settings(index) {
                    match &*property.key {
                        "injection.language" => {
                            pattern.language =
                                property.value.as_deref().and_then(LanguageId::from_fence);
                        }
                        "injection.combined" => pattern.combined = true,
                        "injection.include-children" => pattern.include_children = true,
                        "injection.self" => pattern.itself = true,
                        "injection.parent" => pattern.parent = true,
                        _ => {}
                    }
                }
                pattern
            })
            .collect();
        Ok(Self {
            content: query.capture_index_for_name("injection.content"),
            language: query.capture_index_for_name("injection.language"),
            patterns,
            query,
        })
    }

    /// Calls `visit` with each embedded document in `tree`: its language
    /// and its ranges, clipped to `parent` (the ranges of the layer `tree`
    /// parsed). Documents arrive as they are found, in match order, then
    /// the combined ones in order of first match; `visit` breaks to end the
    /// search once no later document could become a layer.
    ///
    /// Each match, and each content node's child walked to cut it out,
    /// spends one unit of `budget`: `false` when the budget ran out first.
    /// `cancelled` ends the search early.
    fn find(
        &self,
        tree: &ts::Tree,
        source: &str,
        parent: &[ts::Range],
        budget: &mut usize,
        cancelled: &dyn Fn() -> bool,
        visit: &mut dyn FnMut(Target, Vec<ts::Range>) -> ControlFlow<()>,
    ) -> bool {
        let Some(content) = self.content else {
            return true;
        };
        // One document per pattern and language, in order of first match.
        let mut combined: Vec<(usize, Target, Vec<ts::Node<'_>>)> = Vec::new();
        let mut cursor = ts::QueryCursor::new();
        let mut progress = |_: &ts::QueryCursorState| stop_if(cancelled());
        let options = ts::QueryCursorOptions::new().progress_callback(&mut progress);
        let mut matches =
            cursor.matches_with_options(&self.query, tree.root_node(), source.as_bytes(), options);
        while let Some(found_match) = matches.next() {
            let Some(left) = budget.checked_sub(1) else {
                return false;
            };
            *budget = left;
            let pattern = &self.patterns[found_match.pattern_index];
            let mut nodes = Vec::new();
            let mut named = None;
            for capture in found_match.captures {
                if capture.index == content {
                    nodes.push(capture.node);
                } else if Some(capture.index) == self.language {
                    named = capture
                        .node
                        .utf8_text(source.as_bytes())
                        .ok()
                        .and_then(|text| LanguageId::from_fence(text.trim()));
                }
            }
            // A captured name wins over the pattern's properties, as in
            // tree-sitter-highlight.
            let target = match named.or_else(|| pattern.language.clone()) {
                Some(language) => Target::Named(language),
                None if pattern.itself => Target::Itself,
                None if pattern.parent => Target::Parent,
                None => continue,
            };
            if nodes.is_empty() {
                continue;
            }
            if pattern.combined {
                let index = found_match.pattern_index;
                match combined
                    .iter_mut()
                    .find(|(i, t, _)| *i == index && *t == target)
                {
                    Some((_, _, all)) => all.extend(nodes),
                    None => combined.push((index, target, nodes)),
                }
                continue;
            }
            let Some(ranges) = content_ranges(&nodes, pattern.include_children, parent, budget)
            else {
                return false;
            };
            if !ranges.is_empty() && visit(target, ranges).is_break() {
                return true;
            }
        }
        for (index, target, nodes) in combined {
            let include_children = self.patterns[index].include_children;
            let Some(ranges) = content_ranges(&nodes, include_children, parent, budget) else {
                return false;
            };
            if !ranges.is_empty() && visit(target, ranges).is_break() {
                return true;
            }
        }
        true
    }
}

/// The language an injection asks for.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Named(LanguageId),
    Itself,
    Parent,
}

/// The ranges of `nodes` (without their children's unless
/// `include_children`), sorted, merged, and clipped to `parent`. Each
/// child walked spends one unit of `budget`; `None` when it runs out.
const MAX_DROPPED_PATTERNS: usize = 16;

fn lenient_query(language: &ts::Language, text: &str) -> Result<ts::Query, ts::QueryError> {
    let mut text = std::borrow::Cow::Borrowed(text);
    let mut dropped = 0;
    loop {
        let error = match ts::Query::new(language, &text) {
            Ok(query) => return Ok(query),
            Err(error) => error,
        };
        let Some(cut) = (dropped < MAX_DROPPED_PATTERNS)
            .then(|| pattern_at(&text, error.offset))
            .flatten()
        else {
            return Err(error);
        };
        tracing::warn!(%error, "dropping a query pattern the grammar rejects");
        text.to_mut().replace_range(cut, "");
        dropped += 1;
    }
}

fn pattern_at(text: &str, offset: usize) -> Option<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut starts = Vec::new();
    let (mut depth, mut at) = (0usize, 0);
    while at < bytes.len() {
        match bytes[at] {
            b';' => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'"' => {
                if depth == 0 {
                    starts.push(at);
                }
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' => {
                if depth == 0 {
                    starts.push(at);
                }
                depth += 1;
            }
            b')' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        at += 1;
    }
    let index = starts
        .partition_point(|&start| start <= offset)
        .checked_sub(1)?;
    let end = starts.get(index + 1).copied().unwrap_or(bytes.len());
    Some(starts[index]..end)
}

fn content_ranges(
    nodes: &[ts::Node<'_>],
    include_children: bool,
    parent: &[ts::Range],
    budget: &mut usize,
) -> Option<Vec<ts::Range>> {
    let mut own = Vec::new();
    for node in nodes {
        if include_children {
            own.push(node.range());
            continue;
        }
        let mut at = (node.start_byte(), node.start_position());
        // Named children only, as Neovim does: grammars like Markdown's
        // block grammar give content nodes anonymous punctuation children
        // that are part of the embedded text, while named ones (a block
        // quote's `> ` continuation) are not.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            *budget = budget.checked_sub(1)?;
            if child.start_byte() > at.0 {
                own.push(range(at, (child.start_byte(), child.start_position())));
            }
            if child.end_byte() > at.0 {
                at = (child.end_byte(), child.end_position());
            }
        }
        if node.end_byte() > at.0 {
            own.push(range(at, (node.end_byte(), node.end_position())));
        }
    }
    own.sort_by_key(|r| r.start_byte);
    // Tree-sitter wants disjoint ranges; nested content nodes overlap.
    let mut merged: Vec<ts::Range> = Vec::with_capacity(own.len());
    for next in own {
        match merged.last_mut() {
            Some(last) if next.start_byte <= last.end_byte => {
                if next.end_byte > last.end_byte {
                    last.end_byte = next.end_byte;
                    last.end_point = next.end_point;
                }
            }
            _ => merged.push(next),
        }
    }
    Some(intersect(&merged, parent))
}

fn range(start: (usize, ts::Point), end: (usize, ts::Point)) -> ts::Range {
    ts::Range {
        start_byte: start.0,
        end_byte: end.0,
        start_point: start.1,
        end_point: end.1,
    }
}

/// The overlap of two sorted, disjoint range lists.
fn intersect(a: &[ts::Range], b: &[ts::Range]) -> Vec<ts::Range> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        let start = if x.start_byte >= y.start_byte { x } else { y };
        let end = if x.end_byte <= y.end_byte { x } else { y };
        if start.start_byte < end.end_byte {
            out.push(range(
                (start.start_byte, start.start_point),
                (end.end_byte, end.end_point),
            ));
        }
        if x.end_byte <= y.end_byte {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Bounds on the embedded layers of one highlight, so a deeply nested or
/// adversarial source costs a bounded amount of parsing and searching.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Nesting below the host layer: 1 allows embedded languages but not
    /// languages embedded in those.
    pub(crate) depth: u32,
    /// Embedded layers in total.
    pub(crate) layers: usize,
    /// Bytes parsed across embedded layers.
    pub(crate) bytes: usize,
    /// Injection query matches, plus content children walked, across all
    /// layers. The other limits count only regions that become layers;
    /// this one also bounds regions that never do (languages without a
    /// grammar, repeats).
    pub(crate) discovery: usize,
}

impl Limits {
    pub(crate) fn for_source(len: usize) -> Self {
        let bytes = len.saturating_mul(4).saturating_add(64 << 10);
        Self {
            depth: 8,
            // One per paragraph of a long Markdown document.
            layers: 4096,
            bytes,
            // A match or child per node, and nodes are at most about one
            // per byte parsed.
            discovery: bytes,
        }
    }
}

/// The result of [`highlight`].
#[derive(Debug, Default)]
pub(crate) struct Highlights {
    pub(crate) spans: Vec<HighlightSpan>,
    /// Embedded languages whose grammars are still arriving; their regions
    /// keep the host's colors until they do.
    pub(crate) unresolved: Vec<LanguageId>,
    /// A limit left embedded regions unparsed.
    pub(crate) truncated: bool,
    pub(crate) work: Work,
}

/// What a highlight spent against its [`Limits`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Work {
    /// Embedded layers queued for parsing.
    pub(crate) layers: usize,
    /// Units of [`Limits::discovery`] spent.
    pub(crate) discovered: usize,
}

/// One parse: a grammar over some ranges of the source.
struct Layer {
    grammar: Arc<Grammar>,
    /// The grammar of the layer that embedded this one.
    parent: Option<Arc<Grammar>>,
    ranges: Vec<ts::Range>,
    depth: u32,
}

/// `(start, end, kind)`: a span before conversion to [`HighlightSpan`].
type Raw = (usize, usize, HighlightKind);

/// `(start, end, kind, pattern)`: a capture before compaction.
type Captured = (usize, usize, HighlightKind, usize);

/// Highlights `source` with `root` and every language embedded in it,
/// looking embedded languages up with `resolve` (which returns shared
/// handles, so no store lock is held while parsing). Deeper layers take
/// precedence over the host spans they overlap.
///
/// `cancelled` is polled while parsing and searching; once it returns
/// true the highlight stops and returns nothing.
pub(crate) fn highlight(
    root: &Arc<Grammar>,
    source: &str,
    limits: Limits,
    resolve: &mut dyn FnMut(&LanguageId) -> Tag,
    cancelled: &dyn Fn() -> bool,
) -> Highlights {
    highlight_parsed(root, None, source, limits, resolve, cancelled)
}

/// [`highlight`] with the host layer already parsed as `host`, when it is.
fn highlight_parsed(
    root: &Arc<Grammar>,
    mut host: Option<ts::Tree>,
    source: &str,
    limits: Limits,
    resolve: &mut dyn FnMut(&LanguageId) -> Tag,
    cancelled: &dyn Fn() -> bool,
) -> Highlights {
    let mut out = Highlights::default();
    if source.is_empty() {
        return out;
    }
    let whole = range((0, ts::Point::default()), (source.len(), end_point(source)));
    // A grammar over ranges it already parsed would embed the same layers
    // again, forever (a self-injection of a whole node, or two languages
    // embedding each other over the same text).
    let mut seen: HashSet<(u64, Vec<(usize, usize)>)> = HashSet::new();
    seen.insert((root.id, vec![(0, source.len())]));
    let mut queue = VecDeque::from([Layer {
        grammar: root.clone(),
        parent: None,
        ranges: vec![whole],
        depth: 0,
    }]);
    let mut bytes = 0usize;
    let mut budget = limits.discovery;
    let mut merged: Vec<Raw> = Vec::new();
    while let Some(layer) = queue.pop_front() {
        if cancelled() {
            return Highlights::default();
        }
        let included = if layer.depth == 0 {
            &[][..]
        } else {
            &layer.ranges
        };
        let parsed = match host.take().filter(|_| layer.depth == 0) {
            Some(tree) => Some(tree),
            None => layer.grammar.parse(source, included, cancelled),
        };
        let Some(tree) = parsed else {
            continue;
        };
        // A node spanning several ranges (an embedded document's root) also
        // covers the host text between them, which stays the host's.
        let (captured, clears) = layer.grammar.collect_spans(&tree, source, cancelled);
        let spans = clip(subtract(compact_spans(captured), &clears), &layer.ranges);
        // Breadth first, so layers arrive shallowest first and each deeper
        // one overrides what it covers.
        merged = overlay(merged, &spans);
        let Some(injections) = &layer.grammar.injections else {
            continue;
        };
        // Past these limits no region of this layer can become a layer, so
        // the search only has to find out whether there is one to report,
        // and once that is reported there is nothing left to look for.
        let full = layer.depth >= limits.depth || out.work.layers >= limits.layers || budget == 0;
        if full && out.truncated {
            continue;
        }
        let complete = injections.find(
            &tree,
            source,
            &layer.ranges,
            &mut budget,
            cancelled,
            &mut |target, ranges| {
                if layer.depth >= limits.depth {
                    out.truncated = true;
                    return ControlFlow::Break(());
                }
                // Budgets come before resolving, which can load a pack or
                // start a download. A region past them counts as truncated
                // even when its grammar would turn out to be missing.
                let len: usize = ranges.iter().map(|r| r.end_byte - r.start_byte).sum();
                if out.work.layers >= limits.layers {
                    out.truncated = true;
                    return ControlFlow::Break(());
                }
                if bytes.saturating_add(len) > limits.bytes {
                    out.truncated = true;
                    // A later, smaller region may still fit.
                    return ControlFlow::Continue(());
                }
                let grammar = match target {
                    Target::Itself => layer.grammar.clone(),
                    Target::Parent => match &layer.parent {
                        Some(parent) => parent.clone(),
                        None => return ControlFlow::Continue(()),
                    },
                    Target::Named(language) => match resolve(&language) {
                        Tag::Ready(grammar) => grammar,
                        Tag::Pending => {
                            if !out.unresolved.contains(&language) {
                                out.unresolved.push(language);
                            }
                            return ControlFlow::Continue(());
                        }
                        Tag::Unavailable => return ControlFlow::Continue(()),
                    },
                };
                let key = (
                    grammar.id,
                    ranges.iter().map(|r| (r.start_byte, r.end_byte)).collect(),
                );
                if !seen.insert(key) {
                    return ControlFlow::Continue(());
                }
                out.work.layers += 1;
                bytes += len;
                queue.push_back(Layer {
                    grammar,
                    parent: Some(layer.grammar.clone()),
                    ranges,
                    depth: layer.depth + 1,
                });
                ControlFlow::Continue(())
            },
        );
        if !complete {
            out.truncated = true;
        }
    }
    out.work.discovered = limits.discovery - budget;
    out.spans = finish(merged, source);
    out
}

/// Window sizes of [`highlight_windows`], in bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Windowing {
    /// Sources longer than this are highlighted window by window; shorter
    /// ones in one go. Also the size a window starts at.
    pub(crate) window: usize,
    /// The largest window tried while looking for a place to cut: a parse
    /// tree takes tens of bytes per source byte, so this bounds a
    /// highlight's memory.
    pub(crate) max_window: usize,
    /// A window is cut only before this many bytes from its end, so the
    /// parse up to the cut never depends on where the window was
    /// truncated.
    pub(crate) margin: usize,
    /// Bytes highlighted before and after the focus ahead of the exact
    /// pass.
    pub(crate) focus_before: usize,
    pub(crate) focus_after: usize,
}

impl Windowing {
    pub(crate) const DEFAULT: Self = Self {
        window: 2 << 20,
        max_window: 8 << 20,
        margin: 64 << 10,
        focus_before: 64 << 10,
        focus_after: 192 << 10,
    };
}

/// One window's highlight from [`highlight_windows`]: spans in whole-source
/// coordinates, all inside `range`.
pub(crate) struct Window {
    pub(crate) range: std::ops::Range<usize>,
    /// Exact windows follow one another from the start and color their
    /// bytes as a whole-file parse would. An inexact one is the focus
    /// highlighted on its own, ahead of the exact pass; its colors hold
    /// until an exact window covers them.
    pub(crate) exact: bool,
    pub(crate) highlights: Highlights,
}

/// Highlights a long `source` window by window, so memory stays bounded by
/// the largest window's parse tree, and colors near the focus arrive
/// before the rest.
///
/// Exact windows run from the start. Each parses a window of
/// [`Windowing::window`] bytes as a document of its own, cuts it where its
/// last top-level node before the window's final [`Windowing::margin`]
/// starts, and keeps the spans before the cut; the next window starts
/// there. A top-level node starts the same way whatever came before, so
/// those spans match a whole-file parse. A window without such a node, or
/// with an error early on, doubles up to [`Windowing::max_window`], then
/// cuts anyway (at a line end when it has no node to cut at); only then
/// can the next window start inside a construct and color differently.
///
/// Before each exact window, when `focus` (polled each time) lies past the
/// exact windows so far and outside the last inexact window, the lines
/// around the focus are highlighted on their own and emitted first: a
/// window of a few hundred kilobytes colors in a fraction of the time an
/// exact one takes, even at the very start.
///
/// The exact pass starts at `from`: 0, or where an earlier call yielded,
/// with the inexact window that call colored last.
/// After each exact window it hands off, `yield_after` is asked with the
/// number handed off in this call, and the call stops when it says so (so a
/// worker can take turns between long sources).
#[allow(clippy::too_many_arguments)]
pub(crate) fn highlight_windows(
    root: &Arc<Grammar>,
    source: &str,
    sizes: Windowing,
    (from, mut inexact, yield_after): (
        usize,
        Option<std::ops::Range<usize>>,
        &dyn Fn(usize) -> bool,
    ),
    resolve: &(dyn Fn(&LanguageId) -> Tag + Sync),
    cancelled: &(dyn Fn() -> bool + Sync),
    focus: &dyn Fn() -> Option<usize>,
    emit: &mut dyn FnMut(Window),
) -> WindowsEnd {
    // This thread parses windows and finds their cuts; a second one runs
    // the queries of each parsed window meanwhile, in order, so exact
    // windows still arrive in order. A parsed window is handed over only
    // once the query thread is free, which bounds the trees alive to two.
    std::thread::scope(|scope| {
        let (to_query, parsed) =
            std::sync::mpsc::sync_channel::<(usize, usize, usize, ts::Tree)>(0);
        let (done_tx, done) = std::sync::mpsc::channel::<Window>();
        let query = move |(at, cut, end, tree): (usize, usize, usize, ts::Tree)| {
            let text = &source[at..end];
            let found = highlight_parsed(
                root,
                Some(tree),
                text,
                Limits::for_source(text.len()),
                &mut |language| resolve(language),
                cancelled,
            );
            Window {
                range: at..cut,
                exact: true,
                highlights: shifted(found, at, cut),
            }
        };
        let querying = std::thread::Builder::new()
            .name("diffy-syntax-query".to_owned())
            .spawn_scoped(scope, move || {
                for window in parsed {
                    if cancelled() || done_tx.send(query(window)).is_err() {
                        return;
                    }
                }
            })
            .ok();
        // Without the second thread, windows are queried here in turn.
        let mut inline = querying.is_none().then_some(query);
        let len = source.len();
        let mut at = from.min(len);
        let mut handed = 0;
        while at < len {
            for window in done.try_iter() {
                emit(window);
            }
            if cancelled() {
                return WindowsEnd::Cancelled;
            }
            if let Some(f) = focus().map(|f| f.min(len))
                && f >= at
                && inexact.as_ref().is_none_or(|r| !r.contains(&f))
            {
                let start = line_start(source, f.saturating_sub(sizes.focus_before)).max(at);
                let end = line_end(source, (f + sizes.focus_after).min(len));
                let text = &source[start..end];
                let found = highlight(
                    root,
                    text,
                    Limits::for_source(text.len()),
                    &mut |language| resolve(language),
                    cancelled,
                );
                if cancelled() {
                    return WindowsEnd::Cancelled;
                }
                emit(Window {
                    range: start..end,
                    exact: false,
                    highlights: shifted(found, start, end),
                });
                inexact = Some(start..end);
            }
            let mut size = sizes.window;
            let (end, cut, tree) = loop {
                let end = if len - at <= size {
                    len
                } else {
                    line_end(source, at + size)
                };
                let Some(tree) = root.parse(&source[at..end], &[], cancelled) else {
                    return WindowsEnd::Cancelled;
                };
                if end == len {
                    break (end, len, tree);
                }
                let before = (end - at).saturating_sub(sizes.margin);
                match top_level_cut(&tree, before) {
                    Cut::At(cut) => break (end, at + cut, tree),
                    Cut::EarlyError(cut) if size >= sizes.max_window => {
                        break (end, at + cut, tree);
                    }
                    _ => {}
                }
                if size >= sizes.max_window {
                    let cut = line_start(source, at + before).max(at + 1);
                    break (end, cut, tree);
                }
                size *= 2;
            };
            match &mut inline {
                Some(query) => emit(query((at, cut, end, tree))),
                None => {
                    if to_query.send((at, cut, end, tree)).is_err() {
                        return WindowsEnd::Cancelled;
                    }
                }
            }
            at = cut;
            handed += 1;
            if at < len && yield_after(handed) {
                break;
            }
        }
        drop(to_query);
        for window in done {
            emit(window);
        }
        if cancelled() {
            WindowsEnd::Cancelled
        } else if at < len {
            WindowsEnd::Yielded(at, inexact)
        } else {
            WindowsEnd::Done
        }
    })
}

/// How [`highlight_windows`] stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowsEnd {
    Done,
    /// It yielded; the exact pass goes on from this byte, and this is the
    /// inexact window it last colored.
    Yielded(usize, Option<std::ops::Range<usize>>),
    Cancelled,
}

/// Where [`top_level_cut`] would cut a window.
enum Cut {
    At(usize),
    /// The window's first error comes early (see [`top_level_cut`]).
    EarlyError(usize),
    None,
}

/// Where to cut a window parsed as `tree`: the start of its last
/// top-level node that starts after the beginning and at or before byte
/// `before`, but no later than the first top-level node with an error.
///
/// Truncating the window can leave a construct open (a template string or
/// block comment running past its end), and the parser then reads what
/// follows the construct's start as code: nodes that look fine but are not
/// what a whole-file parse finds. The node holding the open construct has
/// an error, so cutting at or before the first error keeps those out. A
/// first error in the window's first quarter would leave little to keep;
/// it is reported with the plain cut, so the caller can try a larger
/// window (the construct may close in it) before taking the error as the
/// source's own.
fn top_level_cut(tree: &ts::Tree, before: usize) -> Cut {
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut cut = None;
    let mut early = false;
    for child in root.children(&mut cursor) {
        let start = child.start_byte();
        if start > before {
            break;
        }
        if start > 0 {
            if child.has_error() && !early {
                if start >= before / 4 {
                    return Cut::At(cut.unwrap_or(start));
                }
                early = true;
            }
            cut = Some(start);
        }
    }
    match cut {
        Some(cut) if early => Cut::EarlyError(cut),
        Some(cut) => Cut::At(cut),
        None => Cut::None,
    }
}

/// `found`, whose spans are offsets into a text starting at `start`, as
/// spans of the whole source, cut off at `end`.
fn shifted(mut found: Highlights, start: usize, end: usize) -> Highlights {
    let base = u32::try_from(start).unwrap_or(u32::MAX);
    let limit = end.saturating_sub(start);
    found.spans.retain_mut(|span| {
        let r = span.range();
        if r.start >= limit {
            return false;
        }
        span.length = (r.end.min(limit) - r.start) as u32;
        span.offset = span.offset.saturating_add(base);
        true
    });
    found
}

/// How far [`line_start`] and [`line_end`] look for a line break before
/// settling for a character boundary, so a minified source of one huge
/// line still splits into bounded windows.
const LINE_SEARCH: usize = 64 << 10;

/// The start of the line holding byte `at`, or `at` itself (on a character
/// boundary) when that line starts more than [`LINE_SEARCH`] earlier.
fn line_start(source: &str, at: usize) -> usize {
    let at = source.floor_char_boundary(at);
    let from = source.floor_char_boundary(at.saturating_sub(LINE_SEARCH));
    source[from..at]
        .rfind('\n')
        .map_or(if from == 0 { 0 } else { at }, |i| from + i + 1)
}

/// The end of the line holding byte `at`, past its newline, or `at` itself
/// (on a character boundary) when that line ends more than
/// [`LINE_SEARCH`] later.
fn line_end(source: &str, at: usize) -> usize {
    let at = source.floor_char_boundary(at);
    let to = source.floor_char_boundary((at + LINE_SEARCH).min(source.len()));
    source[at..to]
        .find('\n')
        .map_or(if to == source.len() { to } else { at }, |i| at + i + 1)
}

/// Stops a tree-sitter parse or query when `cancelled`.
fn stop_if(cancelled: bool) -> ControlFlow<()> {
    if cancelled {
        ControlFlow::Break(())
    } else {
        ControlFlow::Continue(())
    }
}

/// The row and byte column just past the end of `source`.
fn end_point(source: &str) -> ts::Point {
    let row = source.bytes().filter(|&b| b == b'\n').count();
    let column = source.len() - source.rfind('\n').map_or(0, |i| i + 1);
    ts::Point { row, column }
}

/// Resolves overlapping captures into sorted, disjoint spans, nesting
/// them as tree-sitter's highlighter does: a capture inside another wins
/// over it for its own bytes (a code span's delimiters inside the code
/// span), and of captures over the same bytes the later query pattern wins
/// (generic `(identifier) @variable` rules come first in the queries).
fn compact_spans(mut raw: Vec<Captured>) -> Vec<Raw> {
    raw.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.3.cmp(&b.3))
    });
    let mut spans = Vec::with_capacity(raw.len());
    // Open captures, innermost last, with their (clamped) ends; `at` is
    // where the spans emitted so far end.
    let mut open: Vec<(usize, HighlightKind)> = Vec::new();
    let mut at = 0usize;
    fn emit(spans: &mut Vec<Raw>, at: &mut usize, end: usize, kind: HighlightKind) {
        if *at < end {
            spans.push((*at, end, kind));
            *at = end;
        }
    }
    for (start, end, kind, _) in raw {
        while let Some(&(open_end, open_kind)) = open.last() {
            if open_end > start {
                break;
            }
            emit(&mut spans, &mut at, open_end, open_kind);
            open.pop();
        }
        if let Some(&(_, open_kind)) = open.last() {
            emit(&mut spans, &mut at, start, open_kind);
        }
        at = at.max(start);
        // A capture crossing the end of the one around it stops there.
        let end = open.last().map_or(end, |&(open_end, _)| end.min(open_end));
        if end > start {
            open.push((end, kind));
        }
    }
    while let Some((open_end, open_kind)) = open.pop() {
        emit(&mut spans, &mut at, open_end, open_kind);
    }
    spans
}

/// The parts of sorted, disjoint `spans` inside sorted, disjoint `ranges`.
fn clip(spans: Vec<Raw>, ranges: &[ts::Range]) -> Vec<Raw> {
    let mut out = Vec::with_capacity(spans.len());
    let mut first = 0;
    for (start, end, kind) in spans {
        while ranges.get(first).is_some_and(|r| r.end_byte <= start) {
            first += 1;
        }
        for r in ranges[first..].iter().take_while(|r| r.start_byte < end) {
            let (start, end) = (start.max(r.start_byte), end.min(r.end_byte));
            if start < end {
                out.push((start, end, kind));
            }
        }
    }
    out
}

/// `top` laid over `base` (both sorted and disjoint): base spans are cut
/// where top spans overlap them, and the result is sorted and disjoint.
fn overlay(base: Vec<Raw>, top: &[Raw]) -> Vec<Raw> {
    if top.is_empty() {
        return base;
    }
    let holes: Vec<(usize, usize)> = top.iter().map(|t| (t.0, t.1)).collect();
    let mut out = subtract(base, &holes);
    out.extend_from_slice(top);
    out.sort_unstable_by_key(|span| span.0);
    out
}

/// Sorted, disjoint `spans` without the bytes of `holes` (sorted by start,
/// possibly overlapping).
fn subtract(spans: Vec<Raw>, holes: &[(usize, usize)]) -> Vec<Raw> {
    if holes.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + holes.len());
    let mut first = 0;
    for (start, end, kind) in spans {
        while holes.get(first).is_some_and(|h| h.1 <= start) {
            first += 1;
        }
        let mut at = start;
        for hole in holes[first..].iter().take_while(|h| h.0 < end) {
            if hole.0 > at {
                out.push((at, hole.0, kind));
            }
            at = at.max(hole.1);
        }
        if at < end {
            out.push((at, end, kind));
        }
    }
    out
}

/// Spans as the API returns them: on char boundaries (embedded ranges
/// come from nodes, so this only guards against a misbehaving grammar),
/// with offsets saturating for sources past 4 GiB.
fn finish(spans: Vec<Raw>, source: &str) -> Vec<HighlightSpan> {
    spans
        .into_iter()
        .filter_map(|(start, end, kind)| {
            let start = source.ceil_char_boundary(start);
            let end = source.floor_char_boundary(end);
            (start < end).then(|| HighlightSpan {
                offset: u32::try_from(start).unwrap_or(u32::MAX),
                length: u32::try_from(end - start).unwrap_or(u32::MAX),
                kind,
            })
        })
        .collect()
}

/// Maps tree-sitter capture names (nvim and grammar-repository
/// conventions) to highlight kinds; unknown names are `Normal`.
fn capture_name_to_highlight_kind(name: &str) -> HighlightKind {
    if name.starts_with("variable.builtin") || name.starts_with("function.builtin") {
        HighlightKind::Builtin
    } else if name.starts_with("variable.member") {
        HighlightKind::Property
    } else if name.starts_with("function") {
        HighlightKind::Function
    } else if name.starts_with("module.builtin") {
        HighlightKind::Builtin
    } else if name.starts_with("module") {
        HighlightKind::Namespace
    } else if name.starts_with("keyword") {
        HighlightKind::Keyword
    } else if name.starts_with("string")
        || name.starts_with("escape")
        || name.starts_with("character")
    {
        HighlightKind::String
    } else if name.starts_with("comment") {
        HighlightKind::Comment
    } else if name.starts_with("number") {
        HighlightKind::Number
    } else if name.starts_with("type") || name.starts_with("constructor") {
        HighlightKind::Type
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
    } else if let Some(markup) = name
        .strip_prefix("text.")
        .or_else(|| name.strip_prefix("markup."))
    {
        // Prose markup (Markdown's queries use nvim's older `text.*` names)
        // borrows the nearest code kinds, so themes need no new tones.
        markup_kind(markup)
    } else {
        HighlightKind::Normal
    }
}

/// Kinds for `text.*` and `markup.*` captures, named without the prefix.
fn markup_kind(name: &str) -> HighlightKind {
    if name.starts_with("title") || name.starts_with("heading") {
        HighlightKind::Keyword
    } else if name.starts_with("strong") {
        HighlightKind::Type
    } else if name.starts_with("emphasis") || name.starts_with("italic") {
        HighlightKind::Attribute
    } else if name.starts_with("literal") || name.starts_with("raw") {
        HighlightKind::String
    } else if name.starts_with("uri") || name.starts_with("reference") || name.starts_with("link") {
        HighlightKind::Label
    } else if name.starts_with("quote") {
        HighlightKind::Comment
    } else {
        HighlightKind::Normal
    }
}
