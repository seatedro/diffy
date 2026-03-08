# diffy

Native Qt/C++ Git diff viewer with a modern PR-style interface.

## Current Status
Initial v1 scaffold with:
- Repository open + ref listing
- Compare modes: `..`, `...`, single commit
- GitHub PR URL compare support for open local clones
- Renderer abstraction with built-in renderer and optional difftastic renderer
- QML desktop UI shell

## Development

### Nix
```bash
nix develop
cmake -S . -B build -G Ninja
cmake --build build
./build/diffy
```

For explicit configuration-specific trees:

```bash
cmake --preset Debug
cmake --build --preset Debug
./build/Debug/diffy

cmake --preset Release
cmake --build --preset Release
./build/Release/diffy
```

### Dev Loop
```bash
./scripts/dev-loop.sh once
./scripts/dev-loop.sh watch
./scripts/dev-loop.sh preview
```

The harness uses the existing startup automation to build, run tests, launch an offscreen smoke pass, print the latest `DIFFY_STATE`, and refresh a stable capture at `/tmp/diffy-dev/latest.png`.
`once` and `watch` use `build/dev`. `preview` uses a separate `build/preview` tree so QML debugging does not contaminate the strict smoke/test loop.

Defaults are tuned for the repo from [`AGENTS.md`](./AGENTS.md):
- `DIFFY_DEV_REPO=~/exa/monorepo-master`
- `DIFFY_DEV_LEFT=master`
- `DIFFY_DEV_RIGHT=rohit/apollo-servecontents-cutover`

Override them once in an untracked `.diffy-dev.env` file:

```bash
DIFFY_DEV_REPO="$HOME/exa/monorepo-master"
DIFFY_DEV_LEFT="master"
DIFFY_DEV_RIGHT="rohit/apollo-servecontents-cutover"
DIFFY_DEV_LAYOUT="split"
DIFFY_DEV_FILE_INDEX="0"
```

`preview` configures a debug build that enables QML debugging, disables QML cache generation for faster QML iteration, loads `qml/Main.qml` directly from the source tree, and runs `qmlpreview` against the built app. Use that mode when you are mostly editing the QML shell. Use `watch` for the full C++/renderer/test/smoke loop.

For live Wayland captures under `niri`, use:

```bash
./scripts/capture.sh
./scripts/capture.sh /tmp/diffy-live.png
```

### Sync reference repos
```bash
./scripts/sync-reference-repos.sh
```

Reference repositories are cloned under `.docs/refs/` and ignored by git.
