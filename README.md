<p align="center">
  <img width="256" height="256" alt="image" src="https://github.com/user-attachments/assets/7f67d543-fcd5-4156-affe-f741c781a803" />
</p>

<h1 align="center">Diffy</h1>

<p align="center">A native GPU-accelerated Git diff viewer built in Rust.</p>

## Features

- Fast
- Good

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

Diffy keeps developer tooling separate from the normal app binary. Capture, hidden automation,
and in-binary perf harness flags are intentionally not part of the CLI surface.

Hot reload is still supported through Dioxus/Subsecond when the `hot-reload` feature is enabled:

```bash
dx serve --hot-patch --features hot-reload
```

## License

GPL-3.0
