<p align="center">
  <img width="256" height="256" alt="image" src="https://github.com/user-attachments/assets/7f67d543-fcd5-4156-affe-f741c781a803" />
</p>

<h1 align="center">Diffy</h1>

<p align="center">A native GPU-accelerated Git/JJ diff viewer.</p>

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

```bash
cargo build
cargo run
```

## Development

Hot reload is supported through [Dioxus/Subsecond](https://lib.rs/crates/dioxus-cli) when the `hot-reload` feature is enabled:

```bash
dx serve --hot-patch --features hot-reload
```

## License

The Diffy app is GPL-3.0. The headless crates the compare chain is built
from (`diffy-diff`, `diffy-semantic`, `diffy-syntax`, `diffy-compare`) and
the Node binding are MIT OR Apache-2.0; see the `LICENSE-MIT` and
`LICENSE-APACHE` files in each.
