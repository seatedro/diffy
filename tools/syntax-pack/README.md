# syntax-pack

Builds, checks, signs, and publishes diffy-syntax grammar packs. App-facing
usage (building packs, the pack format, the store) is in the
[guide](../../crates/diffy-syntax/PACKS.md); `src/main.rs` documents every
subcommand.

## Building

- Fetches each language's pinned commit with git and hashes every C source,
  header, and query file it reads against the pinned `sha256`.
- Files are hashed with LF line endings, so a Windows checkout that
  converted them hashes the same. Query files ship with LF on every target.
- Compiles `parser.c`, plus `scanner.c` or `scanner.cc` as the entry's
  `scanner` says, into a shared library exporting the language function.
- A grammar marked `generate` ships `grammar.json` but no `parser.c`; the
  build generates it with tree-sitter CLI 0.26.8 (`$TREE_SITTER`, else
  `tree-sitter` on PATH), and `grammar.json` is hashed in its place.
- Loading the lock checks that every name, alias, and extension belongs
  to one language and is lowercase, and that each expected injection is a
  pinned language's tag.

## Languages

- [grammars.toml](grammars.toml): the hand-pinned core languages, each with
  a sample in [samples](samples) and expected captures. A core language
  that fails to build fails the build.
- [registry.toml](registry.toml): every other grammar in nvim-treesitter's
  [parser registry](https://github.com/neovim-treesitter/treesitter-parser-registry),
  generated, never edited by hand. Queries come from each language's
  `nvim-treesitter-queries-*` repository, with `; inherits:` resolved into
  `[fragment.*]` entries. Extensions come from
  [extensions.toml](extensions.toml); registry filetypes become aliases.
  Tags a core language claims stay with it.
- Queries are adapted for tree-sitter at build time: `#lua-match?` and
  `#vim-match?` become `#match?`, and patterns using Neovim-only predicates
  are dropped.
- A registry language that fails on some target is left out of that
  target's index and listed in its `skipped.json`.
- A pack carries the `locals` query files its entry lists (`registry`
  pins a language's `locals.scm` when it has one) and
  `crates/diffy-semantic/queries/<language>/semantic.scm` when that
  exists; a `semantic.scm` that does not compile fails the build.

Regenerate after a registry update:

```sh
curl -fsSLo /tmp/registry.json https://raw.githubusercontent.com/neovim-treesitter/treesitter-parser-registry/main/registry.json
cargo run -p syntax-pack -- registry --registry /tmp/registry.json
```

Every registry language is built and checked for the host before it is
pinned; failures land in `[skipped]` with the reason.

## Checks

For the host target, `build` and `check` fail when a pack:

- does not load as the store loads it, or as a bare library;
- parses its sample in [samples](samples) with an error;
- misses an expected capture from `captures`;
- has an injection query that does not name every language in `injects`.

The TypeScript and TSX samples hold a type assertion and a generic arrow
function beside an element; each parses only under its own grammar.

Packs for another target need a syntax-pack built for that target, natively
or (Intel macOS on Apple silicon) under Rosetta:

```sh
cargo run -p syntax-pack --target x86_64-apple-darwin -- check
```

## Bumping a grammar

- Change `rev` in grammars.toml and run `build` once: it fails and prints
  the new source hash to pin.
- A changed grammar or query changes the files' SHA-256, and so their
  published URLs.

## Publishing

- Base URL: `https://blob.diffygui.com/syntax-packs/v1`, the
  `syntax-packs/v1/` prefix of Diffy's `diffy` R2 bucket.
- `<base>/<target>/index.json` is each target's signed index; Diffy reads
  it through `src/core/syntax/grammar.rs`.
- The index key is the one `PHOSPHOR_PACK_INDEX_PUBLIC_KEY` names; the
  workflow's `PACK_INDEX_KEY` must match it.
- The [Syntax packs](../../.github/workflows/syntax-packs.yml) workflow runs
  on manual dispatch only.

| Job | Runs | Secrets |
|---|---|---|
| `build` | Always, on a native runner per target: `aarch64-apple-darwin` (`macos-26`), `x86_64-apple-darwin` (`macos-26-intel`), `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`. Builds, loads, and checks every pack; uploads `syntax-packs-unsigned-<target>`. Fails when the runner's host triple is not the target. | None |
| `sign` | Only when `languages` is empty. Signs each index with `--base-url` and verifies it against `PACK_INDEX_KEY`; uploads `syntax-packs-signed`. | `PHOSPHOR_PACK_INDEX_PRIVATE_KEY` |
| `publish` | Only with `publish` checked, `languages` empty, on `master`. Runs [publish.sh](publish.sh). One run at a time. | `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` |
| `smoke` | After `publish`, on every target's runner. `syntax-pack remote-check` downloads each pack through the public index and highlights its sample. | None |

publish.sh:

1. Verifies every target's index (signature, every pinned language not
   skipped, local files, URLs); uploads nothing unless all pass.
2. Uploads each file to the key its URL names with one parallel `s5cmd
   sync` through R2's S3 API, skipping keys already there.
3. Downloads every file through the public URL and checks its SHA-256.
4. Replaces each `index.json` with a single PUT (readers see the old index
   or the new one).

- Pack files get an immutable cache header; indexes get `no-cache`.
- A subset dispatch never reaches the public index: `sign` and `publish`
  skip it, and `syntax-pack index` refuses any language set but the full
  pinned one.

## Running a release

1. Dispatch **Syntax packs** on `master` with `languages` empty and
   `publish` unchecked. Every target builds and checks; the signed artifact
   is ready to inspect.
2. Dispatch again with `publish` checked and wait for `smoke` on every
   target.

- `languages` set to, say, `yaml toml` builds and tests just those on every
  target, for trying a grammar bump.
