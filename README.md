# diffy

Native Qt/C++ Git diff viewer with a modern PR-style interface.

## Current Status
Initial v1 scaffold with:
- Repository open + ref listing
- Compare modes: `..`, `...`, single commit
- GitHub PR URL compare support for open local clones
- Renderer abstraction with built-in renderer and optional difftastic renderer
- QML desktop UI shell

## Development

### Nix
```bash
nix develop
cmake -S . -B build -G Ninja
cmake --build build
./build/diffy
```

### Sync reference repos
```bash
./scripts/sync-reference-repos.sh
```

Reference repositories are cloned under `.docs/refs/` and ignored by git.
