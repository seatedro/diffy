# Syntax highlighting and grammar packs

Load tree-sitter grammars at runtime, from packs, to highlight diffs.

- No grammar is compiled into the app.
- A pack is a shared library exporting the tree-sitter language function,
  plus its highlight query, an optional injection query, and a manifest.
- A `GrammarStore` feeds the diff view; an app adapts it to Quark's
  `CodeHighlighter` for markdown code blocks.
- A block whose grammar is missing or still downloading renders plain, then
  recolors when the grammar arrives.

## Configuring a store

| Source | Configure with | Cargo feature | Trust |
|---|---|---|---|
| Local pack roots | `StoreConfig::local_packs(root)` | `diffy-syntax/engine` | Unsigned; the app opts in |
| Signed index | `StoreConfig::downloads(Downloads::new(app_id, url, &keys))` | `diffy-syntax/download` | Ed25519 signature over the index |

From [diff_demo.rs](../diffy-ui/examples/diff_demo.rs):

```rust
use diffy_syntax::{GrammarStore, StoreConfig};

let store = GrammarStore::new(StoreConfig::new().local_packs(root));
diff.enable_syntax(store); // DiffViewState
```

- Search order: local roots (in the order added), then the download cache,
  then the network.
- No default root and no default index URL.
- A pack root holds `<target triple>/<language>/`, as `syntax-pack` writes
  it.
- `diffy_syntax::pack::TARGET` is this build's triple; use it to check a
  root has packs.
- The examples read `$DIFFY_SYNTAX_PACKS`, else the workspace's
  `target/syntax-packs`.

### Downloads

- `index_url` may contain `{target}`, replaced by the build's target triple.
- `keys` are `diffy_syntax::PublicKey` values: same type and hex format as
  quark-update's.
- Cache: `<data dir>/<app id>/syntax-packs` by default;
  `Downloads::cache_dir` overrides.

| OS | Data dir |
|---|---|
| Linux | `$XDG_DATA_HOME`, else `~/.local/share` |
| macOS | `~/Library/Application Support` |
| Windows | `%LOCALAPPDATA%` |

- `GrammarStore::subscribe` calls back on the fetch thread when a pending
  language resolves. Wake the event loop there so recolored blocks get
  polled.
- Diffy's published packs: index
  `https://blob.diffygui.com/syntax-packs/v1/{target}/index.json`, key
  `1f3d2cb98f99da0321bdfa8549b4a43a36f96d7e25b710fb49739d46065b19f1`.
- `GrammarStore::catalog` lists every language the store knows of (local
  packs and the cached index), each tag's language, and the state of those
  installed, loaded, downloading, or failed.

### Lookup behavior

A fence tag or file extension resolves once per store; the result is kept,
so an unknown language costs one map lookup afterwards.

1. A local pack whose name, alias, or extension matches loads at once.
2. A pack in the cached signed index, with every file present and matching
   its SHA-256, loads at once.
3. Otherwise, with downloads configured, the language is pending. The fetch
   thread downloads the index (at most once per store, cached) and then the
   pack.
4. A language missing from the fresh index, or whose download fails, is
   unavailable until `GrammarStore::retry` (which refetches the index).

- `HighlightWorker` answers a pending language with a plain result marked
  `pending`, then sends the real result for the same slot and generation.

### Enclosing scopes

`scope_table(store, language, source)` parses a source with its grammar
and lists the named scopes (`impl Worker`, `fn new`, `class Store`) with
their line ranges; `ScopeTable::at(line)` returns the ones enclosing a
0-based line, outermost first. `ScopeWorker` does the same on its own
thread: `request(key, generation, language, source)` answers with a
`ScopeResult`, keeps only the newest generation per key, caches tables
per key, and answers again when a pending grammar arrives. A key names
one source at one revision.

Built-in queries cover Rust, TypeScript/TSX/JavaScript, Go, Python,
C/C++, Zig and Nix; each pattern that does not compile against a pack's
grammar is skipped, and other languages get an empty table.

## Building packs

[tools/syntax-pack](../../tools/syntax-pack/src/main.rs) builds packs from
the pinned sources in
[grammars.toml](../../tools/syntax-pack/grammars.toml) and
[registry.toml](../../tools/syntax-pack/registry.toml).

- Core languages (grammars.toml): Bash, C, C++, CSS, Go, HTML, JavaScript,
  JSON, Markdown (`markdown` plus `markdown_inline`, which its injection
  query names), Python, Rust, TOML, TSX, TypeScript, YAML.
- Registry languages (registry.toml): every other grammar in
  nvim-treesitter's parser registry that builds, with its queries.
- `syntax-pack list` prints each language's license, repository, and
  commit.

```sh
cargo run -p syntax-pack -- build                 # every language, for this host
cargo run -p syntax-pack -- build rust            # one language
cargo run -p syntax-pack -- build --target x86_64-apple-darwin
cargo run -p syntax-pack -- build --sources DIR   # read DIR/<language>/ instead of git
cargo run -p syntax-pack -- check                 # re-check built packs for this host
```

- Output: `target/syntax-packs/<target>/<language>/`.
- Needs git and the target's C compiler (MSVC on Windows).
- Each source file is hashed against the pinned `sha256`, with `--sources`
  too.
- Host-target packs are loaded and parse a sample after building; a failure
  fails the build.
- `--sources DIR` (offline and Nix builds): every language reads its own
  directory, even when languages share a repository.
  - `DIR/tsx/`: another copy of tree-sitter-typescript.
  - `DIR/markdown_inline/`: another copy of tree-sitter-markdown.
  - TypeScript and TSX also read JavaScript's queries from
    `DIR/javascript/`.

Maintainer details (pinning, cross-target checks, publishing) are in
[tools/syntax-pack/README.md](../../tools/syntax-pack/README.md).

### Pack format

- `manifest.json` fields: `schema` (1), `language`, `aliases`, `extensions`,
  `version`, `target`, `abi` (the grammar's tree-sitter `LANGUAGE_VERSION`),
  `symbol`, `source` (repository, commit, hash).
- `library`, `highlights`, and optional `injections`, `locals` and
  `semantic` each hold `path`, `sha256`, `size`.
- `locals` is the grammar's `locals.scm`; `semantic` is Diffy's own
  `semantic.scm` from
  [crates/diffy-semantic/queries](../diffy-semantic/queries/README.md).
  `GrammarStore::parser` returns both with the grammar.
- Index:
  `{"payload": {"schema": 1, "target": ..., "packs": [manifest, ...]}, "signature": "<hex>"}`.
- The signature is Ed25519 over the payload's canonical JSON, as for
  quark-update manifests.
- A file's URL is its `url` field, else `<language>/<path>` next to the
  index.
- Published indexes set `url` to
  `<base>/<target>/<language>/<sha256>/<path>`, so a cached old file never
  passes for its replacement.
- [pack.rs](src/pack.rs) documents every field and
  check.

## Threat model

Loading a pack runs native code with the app's privileges. Full model:
[lib.rs](src/lib.rs).

- **Downloaded packs** load only through an index that verifies against a
  compiled-in key.
  - Every file must match the index's SHA-256, at download and at each load
    from cache.
  - No switch accepts unsigned indexes.
  - Manifests must name this target, use plain file names, a C identifier
    for the symbol, this platform's library extension, and a supported ABI.
  - A hostile network can withhold packs or replay an older signed index; it
    cannot get unsigned code loaded.
- **Local packs** are unsigned and load only from roots the app names. Name
  only directories the installer or developer controls.
- **The cache** is not defended against anyone who can already write the
  user's files (including swapping a library between hash check and
  `dlopen`).
- **Parsers** see untrusted text. A crash in a grammar's C code kills the
  process; a panic in highlighting is caught by the worker.
