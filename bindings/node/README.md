# @diffy/node

Diffy's file diff for Node. You read the files (from Git or anywhere else)
and pass in their contents. You get back a JSON document to render: a
semantic diff where a grammar exists, a line diff otherwise, with optional
syntax spans. The output format is specified in
[FORMAT.md](https://github.com/seatedro/diffy/blob/master/crates/diffy-compare/FORMAT.md),
and `index.d.ts` types every field.

```js
const { diff, expand, languages, FORMAT_VERSION } = require('@diffy/node')

const doc = await diff(
  {
    files: [{ status: 'modified', old_path: 'src/a.rs', new_path: 'src/a.rs', old: oldBytes, new: newBytes }],
    syntax: true,
    budget_ms: 2000,
  },
  { signal: request.signal },
)

// Later, when the user expands a collapsed gap:
const rows = await expand(sameFileInput, doc.files[0].gap_after, { syntax: true })
```

- `diff(request, { signal })` resolves to a `Document`, with files in the
  order given. Contents are `Uint8Array`s (including `Buffer`s) or strings,
  which are taken as UTF-8.
  - `budget_ms` caps the whole request, counted from the call. Files that
    start after it get line diffs, with `fallback.kind === "request_budget"`.
    With `syntax`, files projected after it also come back without syntax
    spans or scope headers.
  - `file_budget_ms` (default 400) caps each file's semantic diff.
  - `syntax` adds syntax spans. `syntax_max_bytes` (default 1 MiB) skips
    them for larger sides.
- `expand(file, gap, { syntax })` returns the context rows of a `Gap`, or of
  any sub-range of one. `file` must have the contents the document was
  made from.
- `languages()` lists the grammars compiled into this build. Every other
  language gets a line diff.
- `FORMAT_VERSION` is the format version of the returned documents.

Errors:

- Aborting `signal` rejects at once with an `AbortError` whose `cause` is
  the signal's reason. If the signal is already aborted, the call rejects
  without starting any work.
- Malformed input rejects with a `TypeError` whose `code` is `"InvalidArg"`.

## How it runs

- Each call copies its input on the JS thread. The work then runs on a new
  thread of its own (the diff fans out to a few more), not on libuv's
  threadpool, so a long diff never holds up the server's file system, DNS,
  or zlib calls.
- The result crosses into JS as a single JSON string, which is parsed
  there. On an 11 MB document this cost about 75 ms on the JS thread (29 ms
  to create the string, 45 ms to parse it). Building the same objects
  through Node-API cost about 120 ms.
- Grammars are compiled in. Nothing is downloaded and nothing is read from
  disk.
- Renames of identifiers are detected but not vetted against the rest of
  the repository, which the binding never sees.
- The binding uses only Node-API (version 4), so one binary loads in Node
  18 and later and in Electron.
- A Rust panic rejects the call instead of crashing the process. A crash
  inside a grammar's C parser or scanner still takes the process down.

## Packages

`@diffy/node` has no dependencies of its own. It pulls in one prebuilt
binary as an optional dependency:

| Package | Platform |
| --- | --- |
| `@diffy/node-darwin-arm64` | macOS 11+, Apple silicon |
| `@diffy/node-darwin-x64` | macOS 11+, Intel |
| `@diffy/node-linux-x64-gnu` | Linux x64, glibc 2.28+ |
| `@diffy/node-linux-arm64-gnu` | Linux arm64, glibc 2.28+ |
| `@diffy/node-win32-x64-msvc` | Windows x64 |

There is no musl build. Installing with `--omit=optional` leaves the binary
out. Each platform package ships `THIRD_PARTY_NOTICES.md`, which lists the
grammars compiled in and their licenses.

For Electron, unpack `*.node` from the asar archive (for example with
electron-builder's `asarUnpack`).

## Development

From the repository root:

```sh
# Grammars to compile in; rust and typescript are what the tests need.
cargo run -p syntax-pack -- bundle --out "$PWD/target/syntax-bundle" rust typescript tsx
cd bindings/node
DIFFY_SYNTAX_BUNDLE="$PWD/../../target/syntax-bundle" npm run build   # writes diffy.<platform>.node
npm test
```

The build uses the workspace's `node` profile: `release`, but with
`panic = "unwind"` so that panics can be caught. Set
`DIFFY_NODE_BINARY=path/to/diffy.node` to load a specific binary.

`.github/workflows/node.yml` builds and tests every platform. Linux builds
run inside manylinux_2_28, so their binaries need glibc 2.28 at most, and
CI checks this. Pushing a `node-v<version>` tag publishes the platform
packages and then `@diffy/node`. Before tagging, set that version in
`package.json`, in every `npm/*/package.json`, and in the
`optionalDependencies`.
