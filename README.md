# diffy

diffy is a native Rust Git diff viewer for local repositories.

It currently supports:
- read-only diff browsing
- branch range compares with `..` and `...`
- single-commit diffs
- a modern PR-style desktop UI
- GitHub pull request loading

## Build

```powershell
cargo build
cargo test
cargo run
```

## Direct Compare

```powershell
cargo run -- --repo (Get-Location).Path --left HEAD~1 --right HEAD --compare-mode two-dot --layout split
```

## Smoke Run

```powershell
$env:DIFFY_START_REPO = (Get-Location).Path
$env:DIFFY_START_LEFT = 'HEAD~1'
$env:DIFFY_START_RIGHT = 'HEAD'
$env:DIFFY_START_COMPARE_MODE = 'two-dot'
$env:DIFFY_START_LAYOUT = 'split'
$env:DIFFY_EXIT_AFTER_MS = '800'
cargo run
```

## Dev Loop

Put your default compare in `.diffy-dev.env` at the repo root:

```bash
DIFFY_DEV_REPO=/path/to/repo
DIFFY_DEV_LEFT=main
DIFFY_DEV_RIGHT=feature/my-branch
DIFFY_DEV_COMPARE_MODE=three-dot
DIFFY_DEV_LAYOUT=split
DIFFY_DEV_RENDERER=builtin
```

Then use:

```bash
scripts/dev-loop.sh open
scripts/dev-loop.sh watch-open
```

`open` launches straight into the configured compare. `watch-open` rebuilds and relaunches the visible app on file changes and works best with `watchexec` installed.

## Windows

Preinstall these tools first:

- Rust via `rustup`
- Git
- Visual Studio 2022 Community or Build Tools

You can install the command-line tools with `winget`:

```powershell
winget install --id Rustlang.Rustup -e
winget install --id Git.Git -e
winget install --id Microsoft.VisualStudio.2022.Community -e
```

Build and run from PowerShell:

```powershell
cargo build
cargo test
cargo run
```
