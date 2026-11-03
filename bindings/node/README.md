# libdiffy

Semantic file diffs for Node. Pass file contents, get back a JSON document
to render. The format is in
[FORMAT.md](https://github.com/seatedro/diffy/blob/master/crates/diffy-compare/FORMAT.md);
`index.d.ts` types it.

```js
const { diff, expand } = require('libdiffy')

const doc = await diff(
  { files: [{ status: 'modified', old_path: 'a.rs', new_path: 'a.rs', old, new: next }], syntax: true, budget_ms: 2000 },
  { signal },
)
const rows = await expand(file, doc.files[0].gap_after, { syntax: true })
```

## API

- `diff(request, { signal })` → `Document`. Contents are bytes or UTF-8 strings.
  - `budget_ms`: whole request, from the call. Late files get line diffs (`request_budget`) and no syntax.
  - `file_budget_ms`: per-file semantic budget, default 400.
  - `syntax`, `syntax_max_bytes` (default 1 MiB): syntax spans.
- `diffJson(request, { signal })` → the same document as a JSON string, for forwarding without parsing.
- `expand(file, gap, { syntax })` → context rows of a gap or part of one.
- `configure({ max_concurrent_requests })`: concurrent `diff` calls, default cores / 4. Extra calls queue in order.
- `languages()`: grammars compiled in. Others get line diffs.
- `FORMAT_VERSION`.

An aborted `signal` rejects with `AbortError`. Bad input rejects with a
`TypeError` whose `code` is `"InvalidArg"`.

## Notes

- Work runs on its own threads, never libuv's pool. `expand` skips the queue.
- No network or disk access: grammars are compiled in.
- Renames are detected but not checked against the rest of the repository.
- Node-API 4 only: one binary works in Node 18+ and Electron (unpack `*.node` from asar).
- A Rust panic rejects the call; a crash in a grammar's C code kills the process.

## Packages

| Package | Platform |
| --- | --- |
| `libdiffy-darwin-arm64` | macOS 11+, Apple silicon |
| `libdiffy-darwin-x64` | macOS 11+, Intel |
| `libdiffy-linux-x64-gnu` | Linux x64, glibc 2.28+ |
| `libdiffy-linux-arm64-gnu` | Linux arm64, glibc 2.28+ |
| `libdiffy-win32-x64-msvc` | Windows x64 (not on npm yet) |

Each ships `THIRD_PARTY_NOTICES.md` (grammars) and `RUST_NOTICES.md` (Rust crates).

## Development

```sh
cargo run -p syntax-pack -- bundle --out "$PWD/target/syntax-bundle" rust typescript tsx
cd bindings/node
DIFFY_SYNTAX_BUNDLE="$PWD/../../target/syntax-bundle" npm run build
npm test
```

- `DIFFY_NODE_BINARY=path/to/diffy.node` loads a specific binary.
- `npm run notices -- --target TRIPLE --out DIR` writes `RUST_NOTICES.md`.
- Release: set the version in `package.json`, `npm/*/package.json` and `optionalDependencies`, then push a `node-v<version>` tag.
