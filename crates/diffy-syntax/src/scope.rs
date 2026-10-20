use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use crate::{GrammarStore, LanguageId};

const CACHE_LIMIT: usize = 128;
const SOURCE_LIMIT: usize = 16 << 20;
#[cfg(feature = "engine")]
const LABEL_LIMIT: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSpan {
    pub lines: Range<u32>,
    pub label: Arc<str>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeTable {
    spans: Vec<ScopeSpan>,
}

impl ScopeTable {
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    pub fn spans(&self) -> &[ScopeSpan] {
        &self.spans
    }

    pub fn at(&self, line: u32) -> Vec<&str> {
        self.spans
            .iter()
            .filter(|span| span.lines.contains(&line))
            .map(|span| &*span.label)
            .collect()
    }

    #[cfg_attr(not(feature = "engine"), allow(dead_code))]
    fn from_spans(mut spans: Vec<ScopeSpan>) -> Self {
        spans.sort_by(|a, b| {
            a.lines
                .start
                .cmp(&b.lines.start)
                .then(b.lines.end.cmp(&a.lines.end))
        });
        spans.dedup();
        Self { spans }
    }
}

pub fn scope_table(store: &GrammarStore, language: &LanguageId, source: &str) -> ScopeTable {
    scope_table_pending(store, language, source).0
}

fn scope_table_pending(
    store: &GrammarStore,
    language: &LanguageId,
    source: &str,
) -> (ScopeTable, bool) {
    #[cfg(feature = "engine")]
    {
        if source.is_empty() || source.len() > SOURCE_LIMIT {
            return (ScopeTable::default(), false);
        }
        match store.grammar(language) {
            crate::store::Tag::Ready(grammar) => {
                let name = grammar
                    .name()
                    .unwrap_or_else(|| scope_language(language.as_str()));
                let table = grammar
                    .scope_query(name)
                    .and_then(|query| {
                        let tree = grammar.parse_document(source, &|| false)?;
                        Some(query.table(&tree, source))
                    })
                    .unwrap_or_default();
                (table, false)
            }
            crate::store::Tag::Pending => (ScopeTable::default(), true),
            crate::store::Tag::Unavailable => (ScopeTable::default(), false),
        }
    }
    #[cfg(not(feature = "engine"))]
    {
        let _ = (store, language, source, SOURCE_LIMIT);
        (ScopeTable::default(), false)
    }
}

#[cfg(feature = "engine")]
fn scope_language(tag: &str) -> &str {
    match tag {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "py" | "pyi" => "python",
        "h" => "c",
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "cpp",
        other => other,
    }
}

#[cfg(feature = "engine")]
struct Pattern {
    query: &'static str,
    prefix: &'static str,
    suffix: &'static str,
}

#[cfg(feature = "engine")]
const fn pattern(query: &'static str, prefix: &'static str, suffix: &'static str) -> Pattern {
    Pattern {
        query,
        prefix,
        suffix,
    }
}

#[cfg(feature = "engine")]
const RUST: &[Pattern] = &[
    pattern(
        "(mod_item name: (identifier) @name body: (_)) @scope",
        "mod ",
        "",
    ),
    pattern(
        "(impl_item trait: (_) @trait type: (_) @name) @scope",
        "impl ",
        "",
    ),
    pattern("(impl_item !trait type: (_) @name) @scope", "impl ", ""),
    pattern("(trait_item name: (_) @name) @scope", "trait ", ""),
    pattern("(struct_item name: (_) @name) @scope", "struct ", ""),
    pattern("(enum_item name: (_) @name) @scope", "enum ", ""),
    pattern("(function_item name: (_) @name) @scope", "fn ", ""),
];

#[cfg(feature = "engine")]
const ECMA: &[Pattern] = &[
    pattern("(class_declaration name: (_) @name) @scope", "class ", ""),
    pattern(
        "(abstract_class_declaration name: (_) @name) @scope",
        "class ",
        "",
    ),
    pattern(
        "(interface_declaration name: (_) @name) @scope",
        "interface ",
        "",
    ),
    pattern("(enum_declaration name: (_) @name) @scope", "enum ", ""),
    pattern("(internal_module name: (_) @name) @scope", "namespace ", ""),
    pattern(
        "(function_declaration name: (_) @name) @scope",
        "function ",
        "",
    ),
    pattern(
        "(generator_function_declaration name: (_) @name) @scope",
        "function ",
        "",
    ),
    pattern("(method_definition name: (_) @name) @scope", "", "()"),
    pattern(
        "(variable_declarator name: (identifier) @name value: [(arrow_function) (function_expression)]) @scope",
        "",
        "()",
    ),
];

#[cfg(feature = "engine")]
const GO: &[Pattern] = &[
    pattern("(type_spec name: (_) @name) @scope", "type ", ""),
    pattern(
        "(method_declaration receiver: (parameter_list (parameter_declaration type: (_) @receiver)) name: (_) @name) @scope",
        "func ",
        "",
    ),
    pattern("(function_declaration name: (_) @name) @scope", "func ", ""),
];

#[cfg(feature = "engine")]
const PYTHON: &[Pattern] = &[
    pattern("(class_definition name: (_) @name) @scope", "class ", ""),
    pattern("(function_definition name: (_) @name) @scope", "def ", ""),
];

#[cfg(feature = "engine")]
const C: &[Pattern] = &[
    pattern(
        "(namespace_definition name: (_) @name) @scope",
        "namespace ",
        "",
    ),
    pattern(
        "(class_specifier name: (_) @name body: (_)) @scope",
        "class ",
        "",
    ),
    pattern(
        "(struct_specifier name: (_) @name body: (_)) @scope",
        "struct ",
        "",
    ),
    pattern(
        "(enum_specifier name: (_) @name body: (_)) @scope",
        "enum ",
        "",
    ),
    pattern(
        "(function_definition declarator: (function_declarator declarator: (_) @name)) @scope",
        "",
        "()",
    ),
    pattern(
        "(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (_) @name))) @scope",
        "",
        "()",
    ),
    pattern(
        "(function_definition declarator: (reference_declarator (function_declarator declarator: (_) @name))) @scope",
        "",
        "()",
    ),
];

#[cfg(feature = "engine")]
const ZIG: &[Pattern] = &[
    pattern(
        "(variable_declaration (identifier) @name [(struct_declaration) (enum_declaration) (union_declaration)]) @scope",
        "const ",
        "",
    ),
    pattern("(function_declaration name: (_) @name) @scope", "fn ", ""),
    pattern("(test_declaration (_) @name) @scope", "test ", ""),
];

#[cfg(feature = "engine")]
const NIX: &[Pattern] = &[pattern(
    "(binding attrpath: (_) @name expression: [(function_expression) (attrset_expression) (rec_attrset_expression) (let_expression)]) @scope",
    "",
    "",
)];

#[cfg(feature = "engine")]
fn patterns(language: &str) -> &'static [Pattern] {
    match language {
        "rust" => RUST,
        "typescript" | "tsx" | "javascript" => ECMA,
        "go" => GO,
        "python" => PYTHON,
        "c" | "cpp" => C,
        "zig" => ZIG,
        "nix" => NIX,
        _ => &[],
    }
}

#[cfg(feature = "engine")]
pub(crate) struct ScopeQuery {
    query: tree_sitter::Query,
    labels: Vec<(&'static str, &'static str)>,
    scope: u32,
    name: u32,
    extra: Option<u32>,
    receiver: Option<u32>,
}

#[cfg(feature = "engine")]
impl ScopeQuery {
    pub(crate) fn compile(grammar: &tree_sitter::Language, language: &str) -> Option<Self> {
        let usable: Vec<&Pattern> = patterns(language)
            .iter()
            .filter(|pattern| tree_sitter::Query::new(grammar, pattern.query).is_ok())
            .collect();
        if usable.is_empty() {
            return None;
        }
        let text: Vec<&str> = usable.iter().map(|pattern| pattern.query).collect();
        let query = tree_sitter::Query::new(grammar, &text.join("\n")).ok()?;
        Some(Self {
            scope: query.capture_index_for_name("scope")?,
            name: query.capture_index_for_name("name")?,
            extra: query.capture_index_for_name("trait"),
            receiver: query.capture_index_for_name("receiver"),
            labels: usable
                .iter()
                .map(|pattern| (pattern.prefix, pattern.suffix))
                .collect(),
            query,
        })
    }

    fn table(&self, tree: &tree_sitter::Tree, source: &str) -> ScopeTable {
        use tree_sitter::StreamingIterator;

        let text = |node: tree_sitter::Node<'_>| clean(&source[node.byte_range()]);
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source.as_bytes());
        let mut spans = Vec::new();
        while let Some(found) = matches.next() {
            let node = |index: u32| {
                found
                    .captures
                    .iter()
                    .find(|capture| capture.index == index)
                    .map(|capture| capture.node)
            };
            let (Some(scope), Some(name)) = (node(self.scope), node(self.name)) else {
                continue;
            };
            let (prefix, suffix) = self.labels[found.pattern_index];
            let name = text(name);
            let label = match (
                self.extra.and_then(node).map(text),
                self.receiver.and_then(node).map(text),
            ) {
                (Some(trait_name), _) => format!("{prefix}{trait_name} for {name}{suffix}"),
                (_, Some(receiver)) => format!("{prefix}({receiver}) {name}{suffix}"),
                _ => format!("{prefix}{name}{suffix}"),
            };
            let start = scope.start_position().row as u32;
            let end = scope.end_position().row as u32;
            spans.push(ScopeSpan {
                lines: start..end.saturating_add(1),
                label: Arc::from(label),
            });
        }
        ScopeTable::from_spans(spans)
    }
}

#[cfg(feature = "engine")]
fn clean(text: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() > LABEL_LIMIT {
            let cut: String = out.chars().take(LABEL_LIMIT).collect();
            return format!("{cut}…");
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct ScopeResult {
    pub key: u64,
    pub generation: u64,
    pub table: Arc<ScopeTable>,
    pub pending: bool,
}

type Wake = Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>;

struct Job {
    client: u64,
    key: u64,
    generation: u64,
    language: LanguageId,
    source: Arc<str>,
}

enum Message {
    Register(u64, Sender<ScopeResult>, Wake),
    Job(Job),
    Retry,
    Forget(u64),
    Stop,
}

struct Shared {
    jobs: Sender<Message>,
    next_client: AtomicU64,
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.jobs.send(Message::Stop);
    }
}

pub struct ScopeWorker {
    shared: Arc<Shared>,
    client: u64,
    results: Receiver<ScopeResult>,
    wake: Wake,
}

impl ScopeWorker {
    pub fn new(store: GrammarStore) -> Self {
        let (jobs, messages) = channel();
        let retry = jobs.clone();
        store.subscribe(move || retry.send(Message::Retry).is_ok());
        let spawned = std::thread::Builder::new()
            .name("diffy-syntax-scope".to_owned())
            .spawn(move || run(&messages, &store));
        if let Err(error) = spawned {
            tracing_warn(&error);
        }
        Self::client_of(Arc::new(Shared {
            jobs,
            next_client: AtomicU64::new(0),
        }))
    }

    fn client_of(shared: Arc<Shared>) -> Self {
        let client = shared.next_client.fetch_add(1, Ordering::Relaxed);
        let (reply, results) = channel();
        let wake = Wake::default();
        let _ = shared
            .jobs
            .send(Message::Register(client, reply, wake.clone()));
        Self {
            shared,
            client,
            results,
            wake,
        }
    }

    pub fn share(&self) -> Self {
        Self::client_of(self.shared.clone())
    }

    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(wake));
    }

    pub fn request(&self, key: u64, generation: u64, language: LanguageId, source: Arc<str>) {
        let _ = self.shared.jobs.send(Message::Job(Job {
            client: self.client,
            key,
            generation,
            language,
            source,
        }));
    }

    pub fn try_recv(&self) -> Option<ScopeResult> {
        self.results.try_recv().ok()
    }
}

impl Drop for ScopeWorker {
    fn drop(&mut self) {
        let _ = self.shared.jobs.send(Message::Forget(self.client));
    }
}

fn tracing_warn(error: &std::io::Error) {
    #[cfg(feature = "engine")]
    tracing::warn!(%error, "scope worker unavailable");
    #[cfg(not(feature = "engine"))]
    let _ = error;
}

type CacheKey = (u64, LanguageId);

fn run(messages: &Receiver<Message>, store: &GrammarStore) {
    let mut clients: HashMap<u64, (Sender<ScopeResult>, Wake)> = HashMap::new();
    let mut latest: HashMap<(u64, u64), u64> = HashMap::new();
    let mut parked: HashMap<(u64, u64), Job> = HashMap::new();
    let mut cache: VecDeque<(CacheKey, Arc<ScopeTable>)> = VecDeque::new();
    while let Ok(first) = messages.recv() {
        let mut queued: HashMap<(u64, u64), Job> = HashMap::new();
        let mut order: Vec<(u64, u64)> = Vec::new();
        let mut retry = false;
        for message in std::iter::once(first).chain(messages.try_iter()) {
            match message {
                Message::Register(client, reply, wake) => {
                    clients.insert(client, (reply, wake));
                }
                Message::Job(job) => {
                    let slot = (job.client, job.key);
                    let newest = latest.entry(slot).or_insert(job.generation);
                    if job.generation < *newest {
                        continue;
                    }
                    *newest = job.generation;
                    parked.remove(&slot);
                    if !order.contains(&slot) {
                        order.push(slot);
                    }
                    queued.insert(slot, job);
                }
                Message::Retry => retry = true,
                Message::Stop => return,
                Message::Forget(client) => {
                    clients.remove(&client);
                    latest.retain(|slot, _| slot.0 != client);
                    parked.retain(|slot, _| slot.0 != client);
                    queued.retain(|slot, _| slot.0 != client);
                }
            }
        }
        if retry {
            for (slot, job) in parked.drain() {
                if let std::collections::hash_map::Entry::Vacant(entry) = queued.entry(slot) {
                    order.push(slot);
                    entry.insert(job);
                }
            }
        }
        for slot in order {
            let Some(job) = queued.remove(&slot) else {
                continue;
            };
            let cache_key = (job.key, job.language.clone());
            let cached = cache
                .iter()
                .find(|(key, _)| *key == cache_key)
                .map(|(_, table)| table.clone());
            let (table, pending) = match cached {
                Some(table) => (table, false),
                None => {
                    let (table, pending) = scope_table_pending(store, &job.language, &job.source);
                    let table = Arc::new(table);
                    if !pending {
                        if cache.len() == CACHE_LIMIT {
                            cache.pop_front();
                        }
                        cache.push_back((cache_key, table.clone()));
                    }
                    (table, pending)
                }
            };
            let result = ScopeResult {
                key: job.key,
                generation: job.generation,
                table,
                pending,
            };
            if pending {
                parked.insert(slot, job);
            }
            if let Some((reply, wake)) = clients.get(&slot.0) {
                if reply.send(result).is_err() {
                    continue;
                }
                let wake = wake.lock().unwrap_or_else(PoisonError::into_inner).clone();
                if let Some(wake) = wake {
                    wake();
                }
            }
        }
    }
}
