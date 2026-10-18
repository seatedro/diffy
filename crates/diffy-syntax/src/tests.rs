use super::*;

/// `kind:text` for every span whose kind is in `kinds`, in source order.
fn dump(spans: &[HighlightSpan], source: &str, kinds: &[HighlightKind]) -> String {
    spans
        .iter()
        .filter(|span| kinds.contains(&span.kind))
        .map(|span| format!("{}:{}", span.kind.name(), &source[span.range()]))
        .collect::<Vec<_>>()
        .join(" ")
}

fn language(tag: &str) -> LanguageId {
    LanguageId::from_fence(tag).unwrap()
}

const RUST: &str = "fn main() {\n    let greeting = \"hi\";\n    if true { return; }\n}\n";
const RUST_DUMP: &str = "keyword:fn keyword:let string:\"hi\" keyword:if keyword:return";

// Catches a pack the tool builds failing to load, its query failing to
// compile against its grammar, or an alias not reaching it.
#[test]
fn rust_pack_built_by_the_tool_highlights_keywords_and_strings() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };

    let dumped = dump(
        &highlight(&store, &language("rs"), RUST),
        RUST,
        &[HighlightKind::Keyword, HighlightKind::String],
    );

    assert_eq!(dumped, RUST_DUMP);
}

// Catches excerpts sharing lexical state, or their spans landing at the
// excerpt's own offsets: a block comment opened in the first fragment and
// closed only after the second must not color the second, whose spans sit
// at its place in the source.
#[test]
fn fragments_parse_independently_in_source_coordinates() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let source = "/* open\nlet a = 1;\n*/\n";

    let spans = highlight_fragments(&store, &language("rs"), source, &[0..8, 8..19]);

    assert_eq!(
        dump(
            &spans,
            source,
            &[HighlightKind::Comment, HighlightKind::Keyword]
        ),
        "keyword:let"
    );
}

/// Packs named `name` that reuse `base`'s grammar and highlight query
/// with `injections` as their injection query, in a temporary root.
fn fixture_packs(fixtures: &[(&str, &str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (name, base, injections) in fixtures {
        let from = testing::pack_root().join(pack::TARGET).join(base);
        let to = root.path().join(pack::TARGET).join(name);
        std::fs::create_dir_all(&to).unwrap();
        let mut manifest: pack::PackManifest =
            serde_json::from_slice(&std::fs::read(from.join(pack::MANIFEST)).unwrap()).unwrap();
        for file in [&manifest.library, &manifest.highlights] {
            std::fs::copy(from.join(&file.path), to.join(&file.path)).unwrap();
        }
        std::fs::write(to.join("injections.scm"), injections).unwrap();
        manifest.language = (*name).to_owned();
        manifest.aliases.clear();
        manifest.extensions.clear();
        manifest.injections = Some(pack::PackFile {
            path: "injections.scm".to_owned(),
            sha256: pack::sha256_file(&to.join("injections.scm")).unwrap(),
            size: injections.len() as u64,
            url: None,
        });
        std::fs::write(
            to.join(pack::MANIFEST),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
    root
}

/// A store over `fixtures` (see [`fixture_packs`]) and then the test
/// packs, or `None` (the test skips) without the `packs` it needs.
fn fixture_store(
    packs: &[&str],
    fixtures: &[(&str, &str, &str)],
) -> Option<(tempfile::TempDir, GrammarStore)> {
    testing::store_with_all(packs)?;
    let root = fixture_packs(fixtures);
    let config = StoreConfig::new()
        .local_packs(root.path())
        .local_packs(testing::pack_root());
    Some((root, GrammarStore::new(config)))
}

// Catches embedded regions staying plain: a script and a style element
// are parsed as JavaScript and CSS inside the HTML host.
#[test]
fn html_script_and_style_take_javascript_and_css_colors() {
    let Some(store) = testing::store_with_all(&["html", "javascript", "css"]) else {
        return;
    };
    let source = "<script>let x = f(1);</script>\n<style>p { color: red; }</style>\n";

    let dumped = dump(
        &highlight(&store, &language("html"), source),
        source,
        &[
            HighlightKind::Keyword,
            HighlightKind::Function,
            HighlightKind::Property,
        ],
    );

    assert_eq!(dumped, "keyword:let function:f property:color");
}

// Catches a fence's language name skipping the store's aliases, so a block
// tagged `rs` would stay plain inside a Markdown source.
#[test]
fn markdown_fence_tag_resolves_through_aliases() {
    let Some(store) = testing::store_with_all(&["markdown", "markdown_inline", "rust"]) else {
        return;
    };
    let source = "# Notes\n\n```rs\nfn main() { run(); }\n```\n";

    let dumped = dump(
        &highlight(&store, &language("md"), source),
        source,
        &[HighlightKind::Keyword, HighlightKind::Function],
    );

    assert_eq!(
        dumped,
        "keyword:Notes keyword:fn function:main function:run"
    );
}

// Catches the text of a content node's children leaking into the embedded
// document: a fence inside a block quote must parse without its `> `
// markers, so the string spanning two lines stays one string literal.
#[test]
fn quoted_fence_parses_without_its_continuation_markers() {
    let Some(store) = testing::store_with_all(&["markdown", "markdown_inline", "rust"]) else {
        return;
    };
    let source = "> ```rust\n> let s = \"a\n> b\";\n> ```\n";

    let dumped = dump(
        &highlight(&store, &language("md"), source),
        source,
        &[HighlightKind::Keyword, HighlightKind::String],
    );

    // The info string and the fence's line end are the block's literal.
    assert_eq!(
        dumped,
        "string:rust\n keyword:let string:\"a\n string:b\" string:\n"
    );
}

// Catches injections not recursing: a macro's token tree is Rust again,
// and so is the token tree of a macro inside it.
#[test]
fn macro_inside_a_macro_is_parsed_at_each_depth() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let source = "fn main() { vec![foo(bar!(baz(1)))]; }\n";

    let dumped = dump(
        &highlight(&store, &language("rust"), source),
        source,
        &[HighlightKind::Function],
    );

    assert_eq!(
        dumped,
        "function:main function:vec function:! function:foo function:bar function:! function:baz"
    );
}

// Catches byte offsets drifting around multibyte text: embedded spans
// must land on the original source's bytes.
#[test]
fn embedded_spans_keep_byte_offsets_after_multibyte_text() {
    let Some(store) = testing::store_with_all(&["html", "javascript"]) else {
        return;
    };
    let source = "<p>\u{65e5}\u{672c}\u{8a9e}</p>\n<script>let \u{e9} = \"\u{43a}\u{43b}\u{44e}\u{447}\"; f(1);</script>\n";

    let dumped = dump(
        &highlight(&store, &language("html"), source),
        source,
        &[HighlightKind::String, HighlightKind::Function],
    );

    assert_eq!(dumped, "string:\"\u{43a}\u{43b}\u{44e}\u{447}\" function:f");
}

// Catches Markdown prose staying plain: its `text.*` captures map to
// kinds, and `@none` keeps a fenced block's code from all taking the
// block's literal color.
#[test]
fn markdown_markup_takes_colors() {
    let Some(store) = testing::store_with_all(&["markdown", "markdown_inline"]) else {
        return;
    };
    let source = "# Title\n\nSome **bold**, *em*, `code`, <https://x.io>.\n\n```text\nplain\n```\n";

    let spans = highlight(&store, &language("md"), source);
    let dumped = dump(
        &spans,
        source,
        &[
            HighlightKind::Keyword,
            HighlightKind::Type,
            HighlightKind::Attribute,
            HighlightKind::String,
            HighlightKind::Label,
        ],
    );

    // `plain` is the fenced block's content, which `@none` clears.
    assert_eq!(
        dumped,
        "keyword:Title type:bold attribute:em string:code label:<https://x.io> string:text\n string:\n"
    );
}

// Catches combined injections parsed piece by piece: consecutive doc
// comments are one embedded document, so a string can span two of them.
#[test]
fn combined_injection_parses_every_match_as_one_document() {
    const DOCS: &str = "((doc_comment) @injection.content
 (#set! injection.language \"rust\")
 (#set! injection.combined))";
    let Some((_root, store)) = fixture_store(&["rust"], &[("rustdoc", "rust", DOCS)]) else {
        return;
    };
    let source = "/// let s = \"a\n/// b\";\nfn f() {}\n";

    let dumped = dump(
        &highlight(&store, &language("rustdoc"), source),
        source,
        &[HighlightKind::Keyword, HighlightKind::String],
    );

    assert_eq!(dumped, "keyword:let string:\"a\n string: b\" keyword:fn");
}

// Catches `injection.self` and `injection.parent` resolving to the wrong
// grammar: a macro's token tree parses as the macro's own language, or as
// the language that embedded the Rust (JavaScript, where `function` is a
// keyword).
#[test]
fn self_and_parent_injections_table() {
    const SELF: &str = "((macro_invocation (token_tree) @injection.content)
 (#set! injection.self)
 (#set! injection.include-children))";
    const PARENT: &str = "((macro_invocation (token_tree) @injection.content)
 (#set! injection.parent)
 (#set! injection.include-children))";
    let Some((_root, store)) = fixture_store(
        &["rust", "javascript"],
        &[("rust_self", "rust", SELF), ("rust_parent", "rust", PARENT)],
    ) else {
        return;
    };
    let kinds = [HighlightKind::Keyword, HighlightKind::Function];
    let cases = [
        ("self", "rust_self", "m!(f(1));"),
        ("parent", "javascript", "rust_parent`m!(function f() {})`;"),
    ];
    let mut dumps = Vec::new();
    for (name, tag, source) in cases {
        let dumped = dump(&highlight(&store, &language(tag), source), source, &kinds);
        dumps.push(format!("{name}: {dumped}"));
    }

    assert_eq!(
        dumps,
        [
            "self: function:m function:! function:f",
            "parent: function:rust_parent function:m function:! keyword:function function:f",
        ]
    );
}

// Catches a self-injection over the text it came from looping until a
// limit cuts it off: the repeat is recognized and nothing is truncated.
#[test]
fn self_injection_of_the_same_text_stops_without_truncating() {
    const LOOP: &str = "((source_file) @injection.content
 (#set! injection.self)
 (#set! injection.include-children))";
    let Some((_root, store)) = fixture_store(&["rust"], &[("rust-loop", "rust", LOOP)]) else {
        return;
    };
    let source = "fn f() {}\n";

    let outcome = store.highlight(&language("rust-loop"), source);

    assert_eq!(
        (
            dump(&outcome.spans, source, &[HighlightKind::Keyword]),
            outcome.truncated
        ),
        ("keyword:fn".to_owned(), false)
    );
}

// Catches unbounded embedding: past each limit the innermost macro stays
// unparsed and the outcome says it was truncated.
#[test]
fn injection_limits_table() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let source = "fn main() { a!(b!(c(1))); }\n";
    let rust = language("rust");
    let defaults = engine::Limits::for_source(source.len());
    let cases = [
        ("defaults", defaults),
        (
            "depth",
            engine::Limits {
                depth: 1,
                ..defaults
            },
        ),
        (
            "layers",
            engine::Limits {
                layers: 1,
                ..defaults
            },
        ),
        (
            "bytes",
            engine::Limits {
                bytes: 8,
                ..defaults
            },
        ),
    ];
    let mut results = Vec::new();
    for (name, limits) in cases {
        let outcome = store.highlight_within(&rust, source, limits, &|| false);
        let colored = outcome
            .spans
            .iter()
            .any(|span| span.kind == HighlightKind::Function && &source[span.range()] == "c");
        results.push((name, colored, outcome.truncated));
    }

    assert_eq!(
        results,
        [
            ("defaults", true, false),
            ("depth", false, true),
            ("layers", false, true),
            ("bytes", false, true),
        ]
    );
}

// Catches injection discovery doing work no limit pays for: past the
// layer and depth limits the search stops at the first region it cannot
// use, and regions that never become layers (no grammar) stop at the
// discovery limit instead of every match being visited.
#[test]
fn injection_discovery_stops_at_its_limits_table() {
    const NOLANG: &str = "((macro_invocation (token_tree) @injection.content)
 (#set! injection.language \"nolang\"))";
    let Some((_root, store)) = fixture_store(&["rust"], &[("rust-nolang", "rust", NOLANG)]) else {
        return;
    };
    let siblings = "m!(x);\n".repeat(1000);
    let nested = format!("{}x{};\n", "m!(".repeat(200), ")".repeat(200));
    let defaults = engine::Limits::for_source(siblings.len());
    let cases = [
        (
            "layers",
            "rust",
            &siblings,
            engine::Limits {
                layers: 2,
                ..defaults
            },
        ),
        (
            "depth",
            "rust",
            &nested,
            engine::Limits {
                depth: 2,
                ..defaults
            },
        ),
        (
            "discovery",
            "rust-nolang",
            &siblings,
            engine::Limits {
                discovery: 50,
                ..defaults
            },
        ),
    ];
    let mut results = Vec::new();
    for (name, tag, source, limits) in cases {
        let outcome = store.highlight_within(&language(tag), source, limits, &|| false);
        results.push((
            name,
            outcome.truncated,
            outcome.work.layers,
            outcome.work.discovered,
        ));
    }

    assert_eq!(
        results,
        [
            ("layers", true, 2, 3),
            ("depth", true, 2, 3),
            ("discovery", true, 0, 50),
        ]
    );
}

// Catches a cancelled highlight parsing on: once cancellation is asked
// for during the host parse, the parse stops there, nothing is
// highlighted, and nothing polls again.
#[test]
fn highlight_cancelled_mid_parse_stops_at_that_checkpoint() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let source = "fn f() { m!(x); }\n".repeat(2000);
    // Poll 1 is before the host parse; poll 2 is inside it.
    let polls = std::cell::Cell::new(0);
    let cancelled = || {
        polls.set(polls.get() + 1);
        polls.get() >= 2
    };

    let outcome = store.highlight_until(&language("rust"), &source, &cancelled);

    assert_eq!((outcome.spans.len(), polls.get()), (0, 2));
}

#[cfg(feature = "download")]
mod download {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, channel};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use serde_json::{Value, json};

    use super::*;
    use crate::pack::{self, PackError, PackManifest, verify_index};

    const SEED: [u8; 32] = [7; 32];
    const OTHER_SEED: [u8; 32] = [9; 32];

    fn key(seed: &[u8; 32]) -> PublicKey {
        quark_update::manifest::public_key(seed).unwrap()
    }

    fn key_bytes(seed: &[u8; 32]) -> [u8; 32] {
        pack::unhex(&key(seed).to_hex())
            .unwrap()
            .try_into()
            .unwrap()
    }

    fn sign(payload: &Value) -> String {
        quark_update::manifest::sign(&SEED, payload).unwrap()
    }

    fn sha(bytes: &[u8]) -> String {
        pack::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
    }

    fn file(path: &str, bytes: &[u8]) -> Value {
        json!({ "path": path, "sha256": sha(bytes), "size": bytes.len() })
    }

    /// A manifest for `language` whose files hold the given bytes.
    fn manifest(language: &str, library: &[u8], highlights: &[u8]) -> Value {
        json!({
            "schema": 1,
            "language": language,
            "aliases": ["rs"],
            "extensions": ["rs"],
            "version": "1.0.0",
            "target": pack::TARGET,
            "abi": 14,
            "symbol": "tree_sitter_rust",
            "library": file(&format!("libtree-sitter-{language}{}", std::env::consts::DLL_SUFFIX), library),
            "highlights": file("highlights.scm", highlights),
        })
    }

    fn index(packs: Vec<Value>) -> Value {
        json!({ "schema": 1, "target": pack::TARGET, "packs": packs })
    }

    /// The verified languages, or the error, as one line.
    fn outcome(document: &str, seed: &[u8; 32]) -> String {
        match verify_index(document.as_bytes(), &[key_bytes(seed)], pack::TARGET) {
            Ok(index) => {
                let mut parts: Vec<String> =
                    index.packs.iter().map(|p| p.language.clone()).collect();
                parts.extend(index.rejected.iter().map(|(language, error)| {
                    let kind = match error {
                        PackError::Abi { abi, .. } => format!("abi {abi}"),
                        PackError::Unsafe { field, .. } => format!("unsafe {field}"),
                        other => other.to_string(),
                    };
                    format!("rejected {language}: {kind}")
                }));
                parts.join(", ")
            }
            Err(PackError::BadSignature) => "bad signature".to_owned(),
            Err(PackError::WrongTarget { found, .. }) => format!("wrong target {found}"),
            Err(error) => error.to_string(),
        }
    }

    // Catches an index accepted without a valid signature from a trusted
    // key, for another platform, or with entries the loader must not act
    // on.
    #[test]
    fn index_verification_table() {
        let good = || manifest("rust", b"lib", b"(identifier) @variable");
        let with = |field: &str, value: Value| {
            let mut pack = good();
            pack[field] = value;
            sign(&index(vec![pack]))
        };
        let tampered = {
            let mut document: Value = serde_json::from_str(&sign(&index(vec![good()]))).unwrap();
            document["payload"]["packs"][0]["symbol"] = json!("tree_sitter_evil");
            document.to_string()
        };
        let mut other_target = index(vec![good()]);
        other_target["target"] = json!("riscv64gc-unknown-none-elf");

        let cases = [
            ("valid", sign(&index(vec![good()])), SEED, "rust"),
            ("tampered", tampered, SEED, "bad signature"),
            (
                "wrong key",
                sign(&index(vec![good()])),
                OTHER_SEED,
                "bad signature",
            ),
            (
                "wrong platform",
                sign(&other_target),
                SEED,
                "wrong target riscv64gc-unknown-none-elf",
            ),
            (
                "abi mismatch",
                with("abi", json!(99)),
                SEED,
                "rejected rust: abi 99",
            ),
            (
                "path traversal",
                with("library", file("../libevil.so", b"lib")),
                SEED,
                "rejected rust: unsafe library",
            ),
            (
                "symbol injection",
                with("symbol", json!("tree_sitter_rust; rm")),
                SEED,
                "rejected rust: unsafe symbol",
            ),
        ];
        for (name, document, seed, expected) in cases {
            assert_eq!(outcome(&document, &seed), expected, "{name}");
        }
    }

    /// Serves `files` by path over HTTP/1.1 on localhost, one request per
    /// connection, and logs each request's path. Responses wait while a
    /// test holds `hold`, or the [`Server::gate`] of their path. Range
    /// requests get the rest of the file from their offset, and a path in
    /// `cut` sends only that many bytes before closing the connection.
    struct Server {
        url: String,
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        cut: Arc<Mutex<HashMap<String, usize>>>,
        hold: Arc<Mutex<()>>,
        gates: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
        log: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Server {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let files: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let hold: Arc<Mutex<()>> = Arc::default();
            let gates: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>> = Arc::default();
            let stop = Arc::new(AtomicBool::new(false));
            let (thread_files, thread_log, thread_hold, thread_stop) =
                (files.clone(), log.clone(), hold.clone(), stop.clone());
            let thread_gates = gates.clone();
            let cut: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
            let thread_cut = cut.clone();
            let thread = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if thread_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" || line.is_empty() {
                            break;
                        }
                        let lower = line.to_ascii_lowercase();
                        if let Some(offset) = lower
                            .strip_prefix("range: bytes=")
                            .and_then(|rest| rest.trim().strip_suffix('-'))
                        {
                            range = offset.parse::<usize>().ok();
                        }
                    }
                    let path = request.split(' ').nth(1).unwrap_or("").to_owned();
                    thread_log.lock().unwrap().push(path.clone());
                    drop(thread_hold.lock().unwrap());
                    let gate = thread_gates.lock().unwrap().get(&path).cloned();
                    if let Some(gate) = gate {
                        drop(gate.lock().unwrap());
                    }
                    let body = thread_files.lock().unwrap().get(&path).cloned();
                    let (status, body, extra) = match (body, range) {
                        (Some(body), Some(offset)) if offset < body.len() => (
                            "206 Partial Content",
                            body[offset..].to_vec(),
                            format!(
                                "Content-Range: bytes {offset}-{}/{}\r\n",
                                body.len() - 1,
                                body.len()
                            ),
                        ),
                        (Some(body), _) => ("200 OK", body, String::new()),
                        (None, _) => ("404 Not Found", Vec::new(), String::new()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let sent = thread_cut
                        .lock()
                        .unwrap()
                        .get(&path)
                        .map_or(body.len(), |&cut| cut.min(body.len()));
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body[..sent]);
                }
            });
            Self {
                url,
                files,
                cut,
                hold,
                gates,
                log,
                stop,
                thread: Some(thread),
            }
        }

        fn serve(&self, path: &str, bytes: Vec<u8>) {
            self.files.lock().unwrap().insert(path.to_owned(), bytes);
        }

        /// A lock that holds responses for `path` while a test holds it.
        fn gate(&self, path: &str) -> Arc<Mutex<()>> {
            self.gates
                .lock()
                .unwrap()
                .entry(path.to_owned())
                .or_default()
                .clone()
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// A store downloading from `server`'s `/packs/{target}/index.json`
    /// into `cache`, and a channel that hears each resolution.
    fn downloading_store(server: &Server, cache: &Path) -> (GrammarStore, Receiver<()>) {
        downloading_store_over(server, cache, StoreConfig::new())
    }

    /// [`downloading_store`] after the local packs in `config`.
    fn downloading_store_over(
        server: &Server,
        cache: &Path,
        config: StoreConfig,
    ) -> (GrammarStore, Receiver<()>) {
        let downloads = Downloads::new(
            "dev.quark.test",
            format!("{}/packs/{{target}}/index.json", server.url),
            &[key(&SEED)],
        )
        .cache_dir(cache);
        let store = GrammarStore::new(config.downloads(downloads));
        let (sender, receiver) = channel();
        store.subscribe(move || sender.send(()).is_ok());
        (store, receiver)
    }

    fn index_path() -> String {
        format!("/packs/{}/index.json", pack::TARGET)
    }

    fn wait(resolved: &Receiver<()>) {
        resolved
            .recv_timeout(Duration::from_secs(60))
            .expect("the fetch thread resolves the language");
    }

    // Catches an unknown language costing a network round trip (or a new
    // index fetch) on every lookup instead of being remembered.
    #[test]
    fn unknown_languages_resolve_plain_with_one_index_fetch() {
        let server = Server::start();
        server.serve(&index_path(), sign(&index(vec![])).into_bytes());
        let cache = tempfile::tempdir().unwrap();
        let (store, resolved) = downloading_store(&server, cache.path());

        let first = store.status(&language("klingon"));
        wait(&resolved);
        let statuses = [
            first,
            store.status(&language("klingon")),
            store.status(&language("vulcan")),
        ];

        assert_eq!(
            (statuses, server.log()),
            (
                [
                    LanguageStatus::Pending,
                    LanguageStatus::Unavailable,
                    LanguageStatus::Unavailable
                ],
                vec![index_path()]
            )
        );
    }

    // Catches downloaded bytes reaching the cache (and dlopen) when they
    // do not match the signed index.
    #[test]
    fn pack_with_a_sha_mismatch_is_not_installed() {
        let server = Server::start();
        let pack = manifest("rust", b"the real library", b"(identifier) @variable");
        server.serve(&index_path(), sign(&index(vec![pack.clone()])).into_bytes());
        let library = pack["library"]["path"].as_str().unwrap().to_owned();
        let base = format!("/packs/{}/rust", pack::TARGET);
        server.serve(&format!("{base}/{library}"), b"a swapped library".to_vec());
        server.serve(
            &format!("{base}/highlights.scm"),
            b"(identifier) @variable".to_vec(),
        );
        let cache = tempfile::tempdir().unwrap();
        let (store, resolved) = downloading_store(&server, cache.path());

        store.status(&language("rust"));
        wait(&resolved);
        // In whichever version directory the pack would have gone.
        let installed = std::fs::read_dir(cache.path().join(pack::TARGET).join("rust"))
            .into_iter()
            .flatten()
            .any(|dir| dir.unwrap().path().join(&library).exists());

        assert_eq!(
            (store.status(&language("rust")), installed),
            (LanguageStatus::Unavailable, false)
        );
    }

    // Catches a language looked up while the index is downloading being
    // remembered as unavailable for the session because the (still empty)
    // cache lacks it.
    #[test]
    fn language_looked_up_during_the_index_fetch_stays_pending() {
        let server = Server::start();
        let pack = manifest("go", b"a library", b"(identifier) @variable");
        server.serve(&index_path(), sign(&index(vec![pack])).into_bytes());
        let cache = tempfile::tempdir().unwrap();
        let (store, _) = downloading_store(&server, cache.path());

        let held = server.hold.lock().unwrap();
        store.status(&language("rust"));
        while server.log().is_empty() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let during = store.status(&language("go"));
        drop(held);

        assert_eq!(during, LanguageStatus::Pending);
    }

    /// The tool-built packs for `languages` in the test root, served
    /// under `/packs`.
    fn serve_packs(server: &Server, root: &Path, languages: &[&str]) {
        let mut manifests = Vec::new();
        for language in languages {
            let dir = root.join(pack::TARGET).join(language);
            let manifest: PackManifest =
                serde_json::from_slice(&std::fs::read(dir.join(pack::MANIFEST)).unwrap()).unwrap();
            for file in manifest.files() {
                server.serve(
                    &format!("/packs/{}/{language}/{}", pack::TARGET, file.path),
                    std::fs::read(dir.join(&file.path)).unwrap(),
                );
            }
            manifests.push(serde_json::to_value(&manifest).unwrap());
        }
        server.serve(&index_path(), sign(&index(manifests)).into_bytes());
    }

    // Catches a downloaded grammar never reaching blocks that asked for it
    // while it was on its way: the worker must answer plain at once, then
    // send the colored result for the same request.
    #[test]
    fn downloaded_pack_recolors_a_waiting_request() {
        if testing::store_with("rust").is_none() {
            return;
        }
        let server = Server::start();
        serve_packs(&server, &testing::pack_root(), &["rust"]);
        let cache = tempfile::tempdir().unwrap();
        let (store, _) = downloading_store(&server, cache.path());
        let worker = HighlightWorker::new(store);

        worker.request(1, 1, language("rust"), Arc::from(RUST));
        let first = worker.recv().unwrap();
        let second = worker.recv().unwrap();

        let kinds = [HighlightKind::Keyword, HighlightKind::String];
        assert_eq!(
            [
                (
                    first.generation,
                    first.pending,
                    dump(&first.spans, RUST, &kinds)
                ),
                (
                    second.generation,
                    second.pending,
                    dump(&second.spans, RUST, &kinds)
                ),
            ],
            [(1, true, String::new()), (1, false, RUST_DUMP.to_owned())]
        );
    }

    // Catches embedded grammars that arrive after the first highlight never
    // reaching it: a request is answered with the host's colors at once,
    // then again as each embedded pack lands, while a request whose
    // grammars did not change hears nothing new.
    #[test]
    fn embedded_grammars_recolor_a_request_as_they_arrive() {
        if testing::store_with_all(&["html", "javascript", "css"]).is_none() {
            return;
        }
        let server = Server::start();
        let root = testing::pack_root();
        serve_packs(&server, &root, &["javascript", "css"]);
        let css: PackManifest = serde_json::from_slice(
            &std::fs::read(root.join(pack::TARGET).join("css").join(pack::MANIFEST)).unwrap(),
        )
        .unwrap();
        let css_gate = server.gate(&format!("/packs/{}/css/{}", pack::TARGET, css.library.path));
        let held = css_gate.lock().unwrap();
        // HTML is local; JavaScript and CSS download.
        let local = tempfile::tempdir().unwrap();
        let html = local.path().join(pack::TARGET).join("html");
        std::fs::create_dir_all(&html).unwrap();
        for entry in std::fs::read_dir(root.join(pack::TARGET).join("html")).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), html.join(entry.file_name())).unwrap();
        }
        let cache = tempfile::tempdir().unwrap();
        let config = StoreConfig::new().local_packs(local.path());
        let (store, _) = downloading_store_over(&server, cache.path(), config);
        let worker = HighlightWorker::new(store);
        const BOTH: &str = "<script>f(1)</script>\n<style>p { color: red; }</style>\n";
        const STYLE: &str = "<style>p { color: red; }</style>\n";

        worker.request(1, 1, language("html"), Arc::from(BOTH));
        worker.request(2, 1, language("html"), Arc::from(STYLE));
        let mut results: HashMap<u64, Vec<String>> = HashMap::new();
        let mut take = |result: Highlighted| {
            let source = if result.slot == 1 { BOTH } else { STYLE };
            let kinds = [HighlightKind::Function, HighlightKind::Property];
            let unresolved: Vec<&str> = result.unresolved.iter().map(LanguageId::as_str).collect();
            results.entry(result.slot).or_default().push(format!(
                "{} {} [{}]",
                result.revision,
                dump(&result.spans, source, &kinds),
                unresolved.join(",")
            ));
        };
        // Both first answers, then the first request's once JavaScript lands.
        for _ in 0..3 {
            take(worker.recv().unwrap());
        }
        drop(held);
        for _ in 0..2 {
            take(worker.recv().unwrap());
        }

        assert_eq!(
            (&results[&1], &results[&2]),
            (
                &vec![
                    "0  [javascript,css]".to_owned(),
                    "1 function:f [css]".to_owned(),
                    "2 function:f property:color []".to_owned(),
                ],
                &vec!["0  [css]".to_owned(), "1 property:color []".to_owned()]
            )
        );
    }

    // Catches a partial file of an older pack being resumed into a newer
    // one: published files change with their content while a pack keeps
    // its version, and appending the new file's tail to the old file's
    // head would fail its SHA-256 and leave the language plain.
    #[test]
    fn changed_pack_downloads_afresh_over_an_old_partial_file() {
        if testing::store_with("css").is_none() {
            return;
        }
        let server = Server::start();
        let root = testing::pack_root();
        serve_packs(&server, &root, &["css"]);
        let new_index = server.files.lock().unwrap()[&index_path()].clone();
        // The old pack: the same version and size, other library bytes.
        let mut manifest: PackManifest = serde_json::from_slice(
            &std::fs::read(root.join(pack::TARGET).join("css").join(pack::MANIFEST)).unwrap(),
        )
        .unwrap();
        let library_path = format!("/packs/{}/css/{}", pack::TARGET, manifest.library.path);
        let new_library = server.files.lock().unwrap()[&library_path].clone();
        let old_library: Vec<u8> = new_library.iter().rev().copied().collect();
        manifest.library.sha256 = sha(&old_library);
        let old_index = index(vec![serde_json::to_value(&manifest).unwrap()]);
        server.serve(&index_path(), sign(&old_index).into_bytes());
        server.serve(&library_path, old_library);
        server
            .cut
            .lock()
            .unwrap()
            .insert(library_path.clone(), new_library.len() / 2);
        let cache = tempfile::tempdir().unwrap();
        // The first session's download of the old library breaks off.
        {
            let (store, resolved) = downloading_store(&server, cache.path());
            store.status(&language("css"));
            wait(&resolved);
        }
        server.cut.lock().unwrap().clear();
        server.serve(&index_path(), new_index);
        server.serve(&library_path, new_library);

        let (store, resolved) = downloading_store(&server, cache.path());
        store.status(&language("css"));
        wait(&resolved);

        assert_eq!(store.status(&language("css")), LanguageStatus::Ready);
    }
}

/// Small windows, so a few kilobytes of source take several.
const SMALL_WINDOWS: crate::engine::Windowing = crate::engine::Windowing {
    window: 1024,
    max_window: 4096,
    margin: 128,
    focus_before: 64,
    focus_after: 64,
};

/// JavaScript whose template strings and block comments run across many
/// lines, longer than a small window's margin.
fn long_constructs() -> String {
    let mut out = String::new();
    for i in 0..12 {
        out.push_str(&format!("const a{i} = `first line {i}\n"));
        for k in 0..12 {
            out.push_str(&format!(
                "  if (x) {{ return {k}; }} // inside the string\n"
            ));
        }
        out.push_str("`;\n/* a comment\n");
        for k in 0..8 {
            out.push_str(&format!("   let y{k} = \"in the comment\";\n"));
        }
        out.push_str("*/\nfunction f() { return 1; }\n");
    }
    out
}

/// The parts a windowed highlight of `source` emits.
fn windowed_parts(
    store: &GrammarStore,
    source: &str,
    focus: Option<usize>,
) -> Vec<crate::store::Part> {
    let mut parts = Vec::new();
    store.highlight_windowed(
        &language("js"),
        source,
        SMALL_WINDOWS,
        &crate::store::Slice::WHOLE,
        &|| false,
        &|| focus,
        &mut |part| parts.push(part),
    );
    parts
}

// Catches windows cut inside a construct (a template string or block
// comment spanning lines), which would color what follows the cut as
// code: the exact windows, end to end, color the source exactly as one
// parse of the whole does.
#[test]
fn exact_windows_color_a_long_source_like_one_parse() {
    let Some(store) = testing::store_with("javascript") else {
        return;
    };
    let source = long_constructs();

    let parts = windowed_parts(&store, &source, None);
    let ranges: Vec<_> = parts.iter().map(|p| p.range.clone()).collect();
    let joined: Vec<HighlightSpan> = parts.into_iter().flat_map(|p| p.spans).collect();

    assert!(ranges.len() > 3, "{ranges:?}");
    assert!(ranges.windows(2).all(|w| w[0].end == w[1].start));
    assert_eq!(
        (ranges[0].start, ranges.last().unwrap().end),
        (0, source.len() as u32)
    );
    assert_eq!(joined, highlight(&store, &language("js"), &source));
}

// Catches the visible region waiting for the exact pass to reach it: the
// first part is the focus, colored on its own, before any exact window.
#[test]
fn the_focus_is_colored_before_the_exact_pass_reaches_it() {
    let Some(store) = testing::store_with("javascript") else {
        return;
    };
    let source = long_constructs();
    let focus = source.len() * 9 / 10;

    let parts = windowed_parts(&store, &source, Some(focus));
    let first = &parts[0];

    assert!(
        !first.exact && first.range.contains(&(focus as u32)),
        "{:?}",
        first.range
    );
    assert!(!first.spans.is_empty());
    assert!(parts[1..].iter().all(|p| p.exact));
}

/// About `bytes` bytes of plausible JavaScript, from a fixed seed.
fn javascript(bytes: usize) -> String {
    use std::fmt::Write;
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut out = String::with_capacity(bytes + 128);
    while out.len() < bytes {
        let n = next();
        let _ = match n % 5 {
            0 => writeln!(
                out,
                "const value{} = compute({}, \"label {}\");",
                n % 9973,
                n % 101,
                n % 7
            ),
            1 => writeln!(
                out,
                "function step{}(a, b) {{ return a + b * {}; }}",
                n % 99991,
                n % 13
            ),
            2 => writeln!(out, "// note {} about the cache", n % 1_000_003),
            3 => writeln!(
                out,
                "if (state.count > {}) {{ state.count -= 1; }}",
                n % 5000
            ),
            _ => writeln!(
                out,
                "items.push({{ id: {}, name: `item ${{{}}}` }});",
                n % 77777,
                n % 9
            ),
        };
    }
    out
}

/// Peak resident memory since the last reset, in MiB (Linux).
fn peak_rss_mib() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        .map_or(0.0, |kib| kib / 1024.0)
}

/// Time and peak memory of highlighting large JavaScript sources. Run in
/// release with `--ignored --nocapture` and the JavaScript pack built.
#[test]
#[ignore = "measurement, prints a report"]
fn report_large_highlights() {
    let Some(store) = testing::store_with("javascript") else {
        return;
    };
    let mib: usize = std::env::var("DIFFY_SYNTAX_FIXTURE_MIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(63);
    let source = javascript(mib << 20);
    let _ = std::fs::write("/proc/self/clear_refs", "5");
    let before = peak_rss_mib();
    let started = std::time::Instant::now();
    let focus = source.len() * 9 / 10;
    let (mut first, mut focused, mut parts, mut spans, mut inexact) = (None, None, 0, 0, 0);
    store.highlight_streamed(
        &language("js"),
        &source,
        &crate::store::Slice::WHOLE,
        &|| false,
        &|| Some(focus),
        &mut |part| {
            parts += 1;
            spans += part.spans.len();
            inexact += usize::from(!part.exact);
            first.get_or_insert(started.elapsed());
            if part.range.contains(&(focus as u32)) {
                focused.get_or_insert(started.elapsed());
            }
        },
    );
    eprintln!(
        "{mib} MiB JavaScript, windowed: first part {first:.2?}, focus (90%) {focused:.2?}, \
         all {:.2?}, {parts} parts ({inexact} inexact), {spans} spans, peak RSS {:.0} MiB (+{:.0})",
        started.elapsed(),
        peak_rss_mib(),
        peak_rss_mib() - before
    );
    if std::env::var_os("DIFFY_SYNTAX_WHOLE").is_some() {
        let _ = std::fs::write("/proc/self/clear_refs", "5");
        let before = peak_rss_mib();
        let started = std::time::Instant::now();
        let whole = highlight(&store, &language("js"), &source);
        eprintln!(
            "{mib} MiB JavaScript, whole: {:.2?}, {} spans, peak RSS {:.0} MiB (+{:.0})",
            started.elapsed(),
            whole.len(),
            peak_rss_mib(),
            peak_rss_mib() - before
        );
    }
}
