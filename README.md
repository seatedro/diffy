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

## Windows

Preinstall these tools first:

- Visual Studio 2022 Community or Build Tools.
  Install either `Microsoft.VisualStudio.2022.Community` or `Microsoft.VisualStudio.2022.BuildTools`, then make sure the install includes:
  `Desktop development with C++`, MSVC v143 build tools, a recent Windows SDK, and `C++ Clang tools for Windows` so `clang-cl` is available from the Visual Studio developer shell.
- CMake 3.29 or newer.
- Ninja.
- Git.
- `aqtinstall` for downloading Qt.

You can install the command-line tools with `winget`:

```powershell
winget install --id Kitware.CMake -e
winget install --id Ninja-build.Ninja -e
winget install --id Git.Git -e
winget install --id miurahr.aqtinstall -e
winget install --id Microsoft.VisualStudio.2022.Community -e
```

Then install Qt and bootstrap vcpkg:

```powershell
aqt install-qt -O C:\Qt windows desktop 6.8.3 win64_msvc2022_64 -m qtshadertools
git clone https://github.com/microsoft/vcpkg.git "$env:USERPROFILE\vcpkg"
& "$env:USERPROFILE\vcpkg\bootstrap-vcpkg.bat" -disableMetrics
```

After that, the easiest path is `build.bat`. It auto-enters the Visual Studio build environment when needed, defaults `VCPKG_ROOT` to `%USERPROFILE%\vcpkg`, and uses `QT_ROOT` if you set it.

```powershell
$env:QT_ROOT = 'C:\Qt\6.8.3\msvc2022_64'
.\build.bat --preset Release
.\build.bat test --preset Release
.\build.bat run --preset Release
```

If Qt is installed in the default location above, `QT_ROOT` is optional. If Qt is installed somewhere else, set `QT_ROOT` to the Qt install root before running `build.bat`.

If you prefer using CMake directly, the normal `Debug` and `Release` presets still work on Windows:

- `QT_ROOT` points at the Qt install root, for example `C:\Qt\6.8.3\msvc2022_64`
- `VCPKG_ROOT` points at your vcpkg checkout
- you are running from a Visual Studio Developer PowerShell so `clang-cl`, `link.exe`, the MSVC libraries, and the Windows SDK are already configured
- `clang-cl` is selected automatically
- the Visual Studio linker is selected automatically
- the vcpkg manifest is used automatically with the `x64-windows-static-md` triplet

After `QT_ROOT` and `VCPKG_ROOT` are set, you should not need any extra `-D...` configure arguments on Windows.
The Windows build now places the app and test executables in `build\Release\bin` and copies the required Qt runtime pieces there automatically. It also auto-deploys the required Qt DLLs, plugins, QML imports, and `qt.conf`, so the produced `diffy.exe` is runnable directly from that folder.
`libcurl`, `libgit2`, and `tree-sitter` do not need separate manual installs on Windows; vcpkg provides them for the build through CMake.
