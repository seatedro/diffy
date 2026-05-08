{
  description = "Diffy - Rust native diff viewer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      lib = nixpkgs.lib;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAllSystems = lib.genAttrs supportedSystems;
      pkgsFor = system: import nixpkgs { inherit system; };
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      packageSrc =
        if self.sourceInfo ? rev then
          builtins.fetchGit {
            url = "https://github.com/seatedro/diffy.git";
            rev = self.sourceInfo.rev;
            submodules = true;
          }
        else
          self;
      mkDevCommand =
        pkgs:
        pkgs.writeShellScriptBin "dev" ''
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
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
          diffy = pkgs.rustPlatform.buildRustPackage {
            pname = "diffy";
            version = cargoToml.workspace.package.version;
            src = packageSrc;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            doCheck = false;

            postInstall =
              lib.optionalString isLinux ''
                install -Dm644 ${./assets/packaging/png/diffy-16.png} \
                  "$out/share/icons/hicolor/16x16/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-24.png} \
                  "$out/share/icons/hicolor/24x24/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-32.png} \
                  "$out/share/icons/hicolor/32x32/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-48.png} \
                  "$out/share/icons/hicolor/48x48/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-64.png} \
                  "$out/share/icons/hicolor/64x64/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-128.png} \
                  "$out/share/icons/hicolor/128x128/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-256.png} \
                  "$out/share/icons/hicolor/256x256/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-512.png} \
                  "$out/share/icons/hicolor/512x512/apps/diffy.png"
                install -Dm644 ${./assets/packaging/png/diffy-1024.png} \
                  "$out/share/icons/hicolor/1024x1024/apps/diffy.png"

                mkdir -p "$out/share/applications"
                cat > "$out/share/applications/io.github.seatedro.diffy.desktop" <<'EOF'
                [Desktop Entry]
                Type=Application
                Name=Diffy
                GenericName=Git Diff Viewer
                Comment=Native GPU-accelerated Git diff viewer
                Exec=diffy %F
                Icon=diffy
                Terminal=false
                Categories=Development;RevisionControl;
                Keywords=git;diff;review;
                StartupWMClass=diffy
                EOF
              ''
              + lib.optionalString pkgs.stdenv.isDarwin ''
                app="$out/Applications/Diffy.app"
                mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

                cat > "$app/Contents/Info.plist" <<EOF
                <?xml version="1.0" encoding="UTF-8"?>
                <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
                <plist version="1.0">
                <dict>
                  <key>CFBundleExecutable</key>
                  <string>Diffy</string>
                  <key>CFBundleIdentifier</key>
                  <string>io.github.seatedro.diffy</string>
                  <key>CFBundleName</key>
                  <string>Diffy</string>
                  <key>CFBundleDisplayName</key>
                  <string>Diffy</string>
                  <key>CFBundlePackageType</key>
                  <string>APPL</string>
                  <key>CFBundleIconFile</key>
                  <string>diffy.png</string>
                  <key>CFBundleShortVersionString</key>
                  <string>${cargoToml.workspace.package.version}</string>
                  <key>CFBundleVersion</key>
                  <string>${cargoToml.workspace.package.version}</string>
                  <key>NSHighResolutionCapable</key>
                  <true/>
                  <key>LSMinimumSystemVersion</key>
                  <string>11.0</string>
                </dict>
                </plist>
                EOF

                cat > "$app/Contents/MacOS/Diffy" <<EOF
                #!/bin/sh
                exec "$out/bin/diffy" "\$@"
                EOF
                chmod +x "$app/Contents/MacOS/Diffy"
                cp ${./assets/packaging/png/diffy-macos-hires.png} "$app/Contents/Resources/diffy.png"
              '';

            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.git
              pkgs.lld
            ]
            ++ lib.optionals isLinux [
              pkgs.makeWrapper
            ];

            buildInputs = [
              pkgs.openssl
            ]
            ++ lib.optionals isLinux [
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libGL
              pkgs.libx11
              pkgs.libxi
              pkgs.libxcursor
              pkgs.libxrandr
              pkgs.dbus
            ];

            preFixup = lib.optionalString isLinux ''
              wrapProgram "$out/bin/diffy" \
                --prefix LD_LIBRARY_PATH : "${
                  lib.makeLibraryPath [
                    pkgs.dbus
                    pkgs.libGL
                    pkgs.libxkbcommon
                    pkgs.wayland
                  ]
                }"
            '';
          };
        in
        {
          default = diffy;
          inherit diffy;
        }
      );

      apps = forAllSystems (system: {
        default = self.apps.${system}.diffy;
        diffy = {
          type = "app";
          program = "${self.packages.${system}.diffy}/bin/diffy";
        };
      });

      overlays.default = final: _prev: {
        diffy = self.packages.${final.stdenv.hostPlatform.system}.diffy;
      };

      nixosModules.default =
        { pkgs, ... }:
        {
          environment.systemPackages = [
            self.packages.${pkgs.stdenv.hostPlatform.system}.diffy
          ];
        };
      nixosModules.diffy = self.nixosModules.default;

      darwinModules.default =
        { pkgs, ... }:
        {
          environment.systemPackages = [
            self.packages.${pkgs.stdenv.hostPlatform.system}.diffy
          ];
        };
      darwinModules.diffy = self.darwinModules.default;

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          isLinux = pkgs.stdenv.isLinux;
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
              pkgs.jujutsu
              pkgs.jq
              pkgs.lldb
              pkgs.lld
              pkgs.watchexec
              (mkDevCommand pkgs)
            ]
            ++ pkgs.lib.optionals isLinux [
              pkgs.gcc
              pkgs.gdb
              pkgs.rr
              pkgs.strace
            ];

            shellHook =
              (pkgs.lib.optionalString isLinux ''
                export LD_LIBRARY_PATH="${
                  pkgs.lib.makeLibraryPath [
                    pkgs.libxkbcommon
                    pkgs.wayland
                    pkgs.libGL
                    pkgs.vulkan-loader
                    pkgs.dbus
                  ]
                }''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              '')
              + ''
                echo "Diffy dev shell ready"
                echo "Build: cargo build"
                echo "Test: cargo test"
                echo "Run: cargo run"
                echo "Debug binary: gdb ./target/debug/diffy | lldb ./target/debug/diffy | rr record ./target/debug/diffy"
                echo "Loop: dev once | dev watch"
              '';
          };
        }
      );
    };
}
