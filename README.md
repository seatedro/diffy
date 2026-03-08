# diffy

diffy is a native Qt/C++ Git diff viewer for local repositories.

It aims to support:
- read-only diff browsing
- branch range compares with `..` and `...`
- single-commit diffs
- a modern PR-style desktop UI

## Build

```bash
nix develop
cmake -S . -B build -G Ninja
cmake --build build
./build/diffy
```
