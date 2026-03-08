# diffy

diffy is a native Qt/C++ Git diff viewer for local repositories.

It aims to support:
- read-only diff browsing
- branch range compares with `..` and `...`
- single-commit diffs
- a modern PR-style desktop UI
- remote repositories
- PR review and merge tools
- speed

## Build

The configure step materializes the pinned tree-sitter grammar sources if they are missing.

```bash
nix develop
cmake --preset Release
cmake --build --preset Release
ctest --preset Release
./build/Release/diffy
```
