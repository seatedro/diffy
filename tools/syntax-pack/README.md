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
- Compiles `parser.c`, plus `scanner.c` when the entry says `scanner = "c"`,
  into a shared library exporting only the language function.
- Grammars with C++ scanners are rejected; every pinned grammar has a C
  scanner or none.
- Loading grammars.toml checks that every name, alias, and extension belongs
  to one language and is lowercase, and that each expected injection is a
  pinned language's tag.

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

- Base URL: `https://quark.seated.ro/v1`, the `v1/` prefix of the
  `quark-packs` Cloudflare R2 bucket through its custom domain.
- `<base>/<target>/index.json` is each target's signed index.
- The demos trust the public key that the workflow's `PACK_INDEX_KEY` also
  names.
- The [Syntax packs](../../.github/workflows/syntax-packs.yml) workflow runs
  on manual dispatch only.

| Job | Runs | Secrets |
|---|---|---|
| `build` | Always, on a native runner per target: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` (`macos-15`), `x86_64-apple-darwin` (`macos-15-intel`), `x86_64-pc-windows-msvc`. Builds, loads, and checks every pack; uploads `syntax-packs-unsigned-<target>`. Fails when the runner's host triple is not the target, so no pack goes unloaded. | None |
| `sign` | Only when `languages` is empty. Signs each index with `--base-url`, checks it with `syntax-pack verify` against the demos' key, uploads `syntax-packs-signed`. Reads build artifacts as data; restores no cache. | `DIFFY_SYNTAX_SIGNING_KEY` |
| `publish` | Only with `publish` checked, `languages` empty, on `master`. Runs [publish.sh](publish.sh). One run at a time across refs. | `QUARK_PACKS_R2_*` |
| `smoke` | After `publish`, on every target's runner. `syntax-pack remote-check` downloads each pack through the public index and highlights its sample. | None |

publish.sh:

1. Verifies every target's index (signature, every pinned language, local
   files, URLs); uploads nothing unless all pass.
2. Uploads each file to the key its URL names through R2's S3 API, skipping
   existing keys, so a published file is never replaced.
3. Downloads every file through the public URL and checks its SHA-256.
4. Replaces each `index.json` with a single PUT (readers see the old index
   or the new one).

- Pack files get an immutable cache header; indexes get `no-cache`.
- A subset dispatch never reaches the public index: `sign` and `publish`
  skip it, and `syntax-pack index` refuses any language set but the full
  pinned one.

## One-time setup

- Create the `syntax-packs-signing` and `syntax-packs-publish` environments
  (Settings > Environments) with required reviewers and a deployment branch
  rule for `master`.
- Move `DIFFY_SYNTAX_SIGNING_KEY` from repository secrets into
  `syntax-packs-signing`, so only `sign` reads it.
- New key: `cargo run -p quark-update --example manifest_tool keygen`;
  compile the public half (`syntax-pack public-key`) into apps; update
  `PACK_INDEX_KEY` in the workflow.
- Keep it separate from the update key. To rotate, ship the new public key
  beside the old one before signing with it.
- In `syntax-packs-publish`, set `QUARK_PACKS_R2_ACCESS_KEY_ID` and
  `QUARK_PACKS_R2_SECRET_ACCESS_KEY`: S3 credentials of a Cloudflare API
  token limited to object read and write on `quark-packs`. The workflow
  names the bucket and account endpoint.
- Let non-browser clients through Cloudflare for `/v1/*`: apps download with
  ureq, and the publish job checks URLs with curl.

## Running a release

1. Dispatch **Syntax packs** on `master` with `languages` empty and
   `publish` unchecked. Every target builds and checks; the signed artifact
   is ready to inspect.
2. Dispatch again with `publish` checked, approve both environments, and
   wait for `smoke` on all five targets.

- `languages` set to, say, `yaml toml` builds and tests just those on every
  target, for trying a grammar bump.
