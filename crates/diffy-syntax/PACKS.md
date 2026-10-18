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
- Quark's public packs (until Diffy publishes its own): index
  `https://quark.seated.ro/v1/{target}/index.json`, key
  `2194429b3227f613ac19401deddbb0bdc2b4b283e1ecec0d0d38892c28d63955` (what
  `diff_demo` trusts).

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

### Legacy-format packs

Diffy's published grammars use the older Phosphor format, which
difftastic also reads. `StoreConfig::legacy_packs(LegacyPacks::new(dir,
&keys).downloads(url))` loads them (feature `download`); see
[legacy_pack.rs](src/legacy_pack.rs).

- Languages: the 13 in `legacy_pack::LANGUAGES` (Bash, C, C++, Go,
  JavaScript, JSON, Nix, Python, Rust, TOML, TypeScript, TSX, Zig). Tags
  resolve by name or extension; `js`/`mjs` use TypeScript and `jsx` uses
  TSX.
- Cache: `<dir>/<language>/<version>/<platform>/` with `manifest.json`,
  `parser.<dylib|so|dll>`, `highlights.scm`, optional `injections.scm`;
  `<platform>` is `macos-aarch64`, `linux-x86_64`, and so on. Diffy uses
  `<data-local dir>/diffy/phosphor/languages`. The verified index is kept at
  `<dir>/.index/<platform>.json`.
- Index: `{"payload": {"schema_version": 1, "platform", "tree_sitter_abi",
  "packs": [...]}, "signature"}`, Ed25519 over the canonical payload.
- Trust: a cached pack loads only when its `manifest.json` matches the
  SHA-256 in an index that verifies against a configured key, and every
  file matches its SHA-256. Without a verified index nothing loads.
- Queries: `; inherits: ecma` adds the built-in ECMAScript query and TSX
  adds TypeScript's; other inherits are ignored, as Phosphor did.
- A download installs the manifest last and removes this platform's other
  versions of the language, so difftastic reads the same pack.
- A failed download, or a pack whose queries do not compile, leaves the
  language failed until `GrammarStore::retry`, which refetches the index
  and downloads again.
- Publishing: `.github/workflows/update-phosphor-index.mjs` builds each
  parser and query repository at its latest release tag when the registry
  marks it `*_semver`, else at its default branch. Before signing, CI runs
  `cargo run -p syntax-pack -- legacy-check --out .phosphor-pack-out`,
  which loads every pack with this crate's tree-sitter and fails when one
  of the 13 languages does not load.

## Building packs

[tools/syntax-pack](../../tools/syntax-pack/src/main.rs) builds packs from
the pinned sources in
[grammars.toml](../../tools/syntax-pack/grammars.toml).

- Languages: Bash, C, C++, CSS, Go, HTML, JavaScript, JSON, Markdown
  (`markdown` plus `markdown_inline`, which its injection query names),
  Python, Rust, TOML, TSX, TypeScript, YAML.
- Every grammar is MIT licensed; `syntax-pack list` prints license,
  repository, and commit.

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
- `library`, `highlights`, and optional `injections` each hold `path`,
  `sha256`, `size`.
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
