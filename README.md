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

## Smoke Run

```powershell
$env:DIFFY_START_REPO = (Get-Location).Path
$env:DIFFY_START_LEFT = 'HEAD~1'
$env:DIFFY_START_RIGHT = 'HEAD'
$env:DIFFY_START_COMPARE = '1'
$env:DIFFY_EXIT_AFTER_MS = '800'
cargo run
```

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
