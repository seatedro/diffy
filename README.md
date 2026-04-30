<p align="center">
  <img width="256" height="256" alt="image" src="https://github.com/user-attachments/assets/7f67d543-fcd5-4156-affe-f741c781a803" />
</p>

<h1 align="center">Diffy</h1>

<p align="center">A native GPU-accelerated Git diff viewer.</p>

## Features

- Fast
- Good

## Install

### macOS / Linux

```bash
curl -fsSL https://diffygui.com/install | bash
```

### Windows

```powershell
powershell -c "irm https://diffygui.com/install.ps1 | iex"
```

## Build

Clone repo with submodules

```bash
git clone git@github.com:seatedro/diffy.git --recursive
```

For an existing or fresh checkout without initialized submodules, run:

```bash
git submodule update --init --recursive
```

Linux builds may also need the D-Bus development package used by the existing secret-service/keyring dependency path:

```bash
sudo apt-get install libdbus-1-dev pkg-config
```

```bash
cargo build
cargo run
```

To verify a self-compare against the latest two tags, run:

```bash
scripts/compare-latest-tags.sh
```

The direct equivalent is:

```bash
cargo run -- --repo . --left v0.1.3 --right v0.1.4 --compare-mode two-dot
```

This workflow was verified with `v0.1.3` → `v0.1.4`: Diffy loaded 14 changed files and rendered `Cargo.lock` in unified diff.

## Development

Hot reload is supported through [Dioxus/Subsecond](https://lib.rs/crates/dioxus-cli) when the `hot-reload` feature is enabled:

```bash
dx serve --hot-patch --features hot-reload
```

## License

GPL-3.0
