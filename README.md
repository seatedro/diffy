# diffy

A native GPU-accelerated Git diff viewer built in Rust.

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

Diffy takes advantage of dioxus' binary patching to enable hot reloads!

```bash
dx serve --hot-patch --features hot-reload
```

## License

GPL-3.0
