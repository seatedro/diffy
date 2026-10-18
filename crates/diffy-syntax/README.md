# diffy-syntax

Tree-sitter syntax highlighting for [Diffy](../../README.md).
No grammar is compiled in: each language is a pack (a shared library, its
queries, and a manifest) that a `GrammarStore` loads from local pack
directories the app trusts or, with the `download` feature, from a signed
index. Code renders plain until its grammar is available.

Features: `engine` (the tree-sitter runtime and local packs) and `download`
(fetching packs over HTTPS with Ed25519-verified indexes). Without `engine`
the crate has no dependencies and every lookup falls back to plain text.

`highlight` runs synchronously; `HighlightWorker` runs on a background
thread and drops requests a newer generation superseded, which suits code
that is still streaming. Embedded languages (scripts in HTML, fenced blocks
in Markdown) use their own grammars when the host pack has an injection
query; the worker sends a newer revision as their packs arrive.

Build packs with `cargo run -p syntax-pack -- build`. The
[guide](PACKS.md) covers configuration, the pack
format, and the threat model;
[tools/syntax-pack](../../tools/syntax-pack/README.md) covers publishing an
index.
