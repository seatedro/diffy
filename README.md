# diffy

A native GPU-accelerated Git diff viewer built in Rust.

## Features

- Split and unified diff views
- Syntax highlighting (tree-sitter)
- Word-level inline diffs
- Branch comparison — diff, merge, and commit modes
- GitHub PR loading via device flow auth
- File tree sidebar with search, status badges, and per-language icons
- Dark and light themes with perceptual Oklch color system
- Builtin and difftastic diff engines

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
