# AGENTS.md

Guidance for agents working in `crates/phosphor`. The root `AGENTS.md` still
applies; this node captures the syntax and pack-loading contracts that are not
obvious from individual call sites.

## Purpose And Scope

Phosphor is Diffy's tree-sitter-backed syntax analysis crate. It owns language
metadata, parser-pack discovery/loading, pack installation and verification, and
conversion from tree-sitter captures into compact highlight spans.

Phosphor does not own UI state, worker scheduling, Git file reads, viewport
policy, or rendering. The app decides when to request syntax and how stale
results are guarded.

## Related Context

- Root async/performance rules: `../../AGENTS.md`
- Carbon text stores and byte ranges consumed here: `../carbon/AGENTS.md`
- App syntax state/effects: `../../src/ui/state/syntax.rs`
- Runtime pack install and syntax workers: `../../src/apprt/runtime.rs` and
  `../../src/apprt/services.rs`
- Pack index publishing: `../../.github/workflows/update-phosphor-index.yml`
  and `../../.github/workflows/update-phosphor-index.mjs`

## Core Contracts

- Syntax is best-effort. Path-based high-level helpers return empty spans when
  the language is unknown or its parser pack is unavailable.
- Direct language helpers return `PhosphorError::MissingParser` when a caller
  explicitly asks for a language without an installed pack.
- `TextStore` inputs may be non-UTF-8. Return `PhosphorError::InvalidUtf8`
  instead of lossy conversion.
- Highlight offsets and lengths are byte offsets in Carbon coordinates.
  `HighlightLineBuffer` stores line-local spans clipped from global spans.
- Range highlighting still parses the full source, then uses tree-sitter query
  byte ranges for visible windows. This preserves context while keeping query
  work bounded.
- Parser and query state are thread-local caches. Loaded dynamic libraries must
  stay alive for as long as their tree-sitter language is used.
- Language registry updates must keep `LanguageId::name`,
  `LanguageId::from_name`, extension metadata, common-language status, and pack
  index generation in sync.
- Pack indexes are signed in release unless debug mode or
  `DIFFY_PHOSPHOR_ALLOW_UNSIGNED_PACKS` explicitly allows unsigned packs.
- `PHOSPHOR_PACK_INDEX_PUBLIC_KEY` is a compile-time trust input. `build.rs`
  must continue to rerun when it changes.
- Pack installation must verify index signature, platform triple,
  tree-sitter ABI, safe path segments, manifest language/platform/ABI, and
  SHA-256 for downloaded and local files.
- `PackInstaller::ensure_packs_for_paths` is the batch warmup path. It dedupes
  language lookup and fetches the index once for a batch of discovered paths.
- Default storage is under the platform data-local directory at
  `diffy/phosphor/languages/<language>/<version>/<platform>/`.

## Usage Patterns

- Warm packs from discovered diff/status paths as early as file lists are known;
  keep single-path selection installs as a retry/fallback path.
- Keep syntax work off the UI thread. Use Diffy's effect/runtime/event path for
  installs and file highlighting.
- Prefer `highlight_text_store_path_ranges` or
  `highlight_text_store_language_lines` for viewport-driven callers that already
  hold Carbon text and byte ranges.
- Add languages by updating the registry, enum name conversions, pack generation
  metadata, and tests together.
- Treat capture-name mapping as a compatibility layer with tree-sitter query
  conventions. Unknown captures should fall back to `HighlightKind::Normal`.

## Anti-Patterns

- Do not block app open, file selection, or scroll on pack downloads or full-file
  highlighting.
- Do not bypass signature, checksum, ABI, platform, or path-safety validation to
  make a pack load.
- Do not assume a missing parser is an error for path-based highlighting.
- Do not duplicate Carbon text or convert byte coordinates to character
  coordinates in this crate.
- Do not make tests depend on this machine having no locally installed packs.

## Validation

- Focused crate tests: `cargo test -p phosphor`
- Pack installer changes: `cargo test -p phosphor pack::tests`
- App contract fallout: run a focused `cargo check` or syntax-state test that
  covers `LoadFileSyntax`, `FileSyntaxReady`, and pack warmup events.

## Maintenance

Keep app scheduling guidance in the root/app nodes and pack mechanics here. If a
pack format, trust policy, storage layout, or language-registry invariant
changes, update this node in the same commit as the code.
