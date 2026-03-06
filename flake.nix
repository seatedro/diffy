{
  description = "Diffy - native Qt/C++ diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      grammars = pkgs.tree-sitter-grammars;
      grammarPackages = [
        grammars.tree-sitter-c
        grammars.tree-sitter-cpp
        grammars.tree-sitter-rust
        grammars.tree-sitter-python
        grammars.tree-sitter-javascript
        grammars.tree-sitter-go
        grammars.tree-sitter-bash
        grammars.tree-sitter-json
        grammars.tree-sitter-toml
        grammars.tree-sitter-zig
        grammars.tree-sitter-nix
      ];
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.cmake
          pkgs.ninja
          pkgs.pkg-config
          pkgs.git
          pkgs.gcc
          pkgs.libgit2
          pkgs.tree-sitter
          pkgs.qt6.qtbase
          pkgs.qt6.qtdeclarative
        ] ++ grammarPackages;

        shellHook = ''
          echo "Diffy dev shell ready"
          echo "Build: cmake -S . -B build -G Ninja && cmake --build build"
        '';

        DIFFY_GRAMMAR_PATHS = builtins.concatStringsSep ":" (map (g: "${g}") grammarPackages);
      };
    };
}
