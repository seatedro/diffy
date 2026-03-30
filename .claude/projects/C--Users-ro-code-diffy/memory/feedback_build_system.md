---
name: build-system-is-cargo
description: diffy uses cargo (Rust), not build.bat/CMake — ignore the CLAUDE.md build instructions
type: feedback
---

This is a Cargo/Rust project. Build with `cargo build`, test with `cargo test`.
The CLAUDE.md references build.bat and CMake but those are stale/wrong for the native branch.

**Why:** User corrected when agent tried to use `build.bat`.
**How to apply:** Always use `cargo build`, `cargo test`, `cargo run` — never build.bat or cmake.
