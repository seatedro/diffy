{
  description = "Diffy - Rust + Qt diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
      mkDevCommand = pkgs: pkgs.writeShellScriptBin "dev" ''
        set -euo pipefail
        repo_root="''${DIFFY_REPO_ROOT:-$PWD}"
        if [ ! -x "$repo_root/scripts/dev-loop.sh" ]; then
          echo "dev: expected DIFFY_REPO_ROOT or current directory to point at the diffy repo" >&2
          exit 1
        fi
        exec "$repo_root/scripts/dev-loop.sh" "$@"
      '';
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          qt = pkgs.qt6;
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "diffy";
            version = "0.1.0";
            src = self;
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes = {
                "qmetaobject-0.2.10" = "sha256-nFg/FrbBRSTaWZRBXevoJ02OfFCKdiEE1qAytGG0V5A=";
                "qttypes-0.2.12" = "sha256-nFg/FrbBRSTaWZRBXevoJ02OfFCKdiEE1qAytGG0V5A=";
              };
            };

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.git
              qt.wrapQtAppsHook
              qt.qtbase
            ];

            buildInputs = [
              qt.qtdeclarative
            ];

            preBuild = ''
              export PATH="${qt.qtbase}/bin:$PATH"
              export QMAKE="${pkgs.lib.getExe' qt.qtbase "qmake"}"
              qt_declarative_prefix="${qt.qtdeclarative}"
              qt_declarative_include="$qt_declarative_prefix/include"
              qt_declarative_lib="$qt_declarative_prefix/lib"
              export QT_ADDITIONAL_PACKAGES_PREFIX_PATH="$qt_declarative_prefix''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH:+:''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH}}"
              export CXXFLAGS="-F$qt_declarative_lib -I$qt_declarative_include''${CXXFLAGS:+ ''${CXXFLAGS}}"
              export RUSTFLAGS="-L framework=$qt_declarative_lib''${RUSTFLAGS:+ ''${RUSTFLAGS}}"
            '';

            postInstall = ''
              cp -r qml "$out/qml"
            '';
          };
        });

      devShells = forAllSystems (system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
          qt = pkgs.qt6;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];

            packages = [
              pkgs.cargo
              pkgs.rustc
              pkgs.rustfmt
              pkgs.clippy
              pkgs.nodejs_22
              pkgs.uv
              pkgs.git
              pkgs.jq
              pkgs.lldb
              pkgs.watchexec
              (mkDevCommand pkgs)
            ] ++ pkgs.lib.optionals isLinux [
              pkgs.gcc
              pkgs.gdb
              pkgs.rr
              pkgs.strace
            ];

            shellHook = ''
              qt_declarative_prefix="${qt.qtdeclarative}"
              qt_declarative_include="$qt_declarative_prefix/include"
              qt_declarative_lib="$qt_declarative_prefix/lib"

              export DIFFY_REPO_ROOT="$PWD"
              export QMAKE="${pkgs.lib.getExe' qt.qtbase "qmake"}"
              export QT_ADDITIONAL_PACKAGES_PREFIX_PATH="$qt_declarative_prefix''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH:+:''${QT_ADDITIONAL_PACKAGES_PREFIX_PATH}}"
              export CXXFLAGS="-F$qt_declarative_lib -I$qt_declarative_include''${CXXFLAGS:+ ''${CXXFLAGS}}"
              export RUSTFLAGS="-L framework=$qt_declarative_lib''${RUSTFLAGS:+ ''${RUSTFLAGS}}"
              echo "Diffy dev shell ready"
              echo "Build: cargo build"
              echo "Test: cargo test"
              echo "Run: cargo run"
              echo "Debug binary: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
              echo "Loop: dev once | dev watch"
            '';
          };
        });
    };
}
