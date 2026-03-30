# diffy

A native GPU-accelerated Git diff viewer built in Rust.

## Features

- Split and unified diff views
- Syntax highlighting (tree-sitter)
- Word-level inline diffs
- Branch comparison — two-dot, three-dot, and single-commit modes
- GitHub PR loading via device flow auth
- File tree sidebar with search, status badges, and per-language icons
- Dark and light themes with perceptual Oklch color system
- Builtin and difftastic diff engines

## Build

```bash
cargo build
cargo run
```

## License

GPL-3.0
