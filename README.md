# diffy

diffy is a native Rust + Qt Git diff viewer for local repositories.

It currently supports:
- read-only diff browsing
- branch range compares with `..` and `...`
- single-commit diffs
- a modern PR-style desktop UI
- GitHub pull request loading

## Build

```bash
nix develop
cargo build
cargo test
cargo run
```

The app loads QML from the repository checkout during local development, so run it from the repo root or set `DIFFY_REPO_ROOT`.

## Smoke Run

```bash
QT_QPA_PLATFORM=offscreen \
DIFFY_START_REPO="$PWD" \
DIFFY_START_LEFT=HEAD~1 \
DIFFY_START_RIGHT=HEAD \
DIFFY_START_COMPARE=1 \
DIFFY_REQUIRE_RESULTS=1 \
DIFFY_EXIT_AFTER_MS=800 \
cargo run
```

## Windows

Preinstall these tools first:

- Rust via `rustup`
- Visual Studio 2022 Community or Build Tools with the C++ workload
- Git
- `aqtinstall` for downloading Qt

You can install the command-line tools with `winget`:

```powershell
winget install --id Rustlang.Rustup -e
winget install --id Git.Git -e
winget install --id miurahr.aqtinstall -e
winget install --id Microsoft.VisualStudio.2022.Community -e
```

Then install Qt:

```powershell
aqt install-qt -O C:\Qt windows desktop 6.8.3 win64_msvc2022_64 -m qtshadertools
```

Build and run from a Visual Studio Developer PowerShell so the MSVC toolchain is already on `PATH`, then point `qmetaobject` at the Qt install with `QT_ROOT`.

```powershell
$env:QT_ROOT = 'C:\Qt\6.8.3\msvc2022_64'
cargo build
cargo test
cargo run
```
