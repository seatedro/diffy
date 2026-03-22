@echo off
setlocal EnableExtensions

pushd "%~dp0" >nul || exit /b 1

set "ACTION=build"
set "PRESET=Release"
set "FRESH="

:parse_args
if "%~1"=="" goto parsed_args

if /I "%~1"=="build" (
  set "ACTION=build"
  shift
  goto parse_args
)

if /I "%~1"=="configure" (
  set "ACTION=configure"
  shift
  goto parse_args
)

if /I "%~1"=="test" (
  set "ACTION=test"
  shift
  goto parse_args
)

if /I "%~1"=="run" (
  set "ACTION=run"
  shift
  goto parse_args
)

if /I "%~1"=="--preset" (
  if "%~2"=="" goto missing_preset
  set "PRESET=%~2"
  shift
  shift
  goto parse_args
)

if /I "%~1"=="--fresh" (
  set "FRESH=--fresh"
  shift
  goto parse_args
)

if /I "%~1"=="-h" goto usage
if /I "%~1"=="--help" goto usage
if /I "%~1"=="help" goto usage

echo Unknown argument: %~1
echo.
goto usage_error

:parsed_args
call :ensure_windows_env || goto fail

if /I "%ACTION%"=="configure" goto do_configure
if /I "%ACTION%"=="build" goto do_build
if /I "%ACTION%"=="test" goto do_test
if /I "%ACTION%"=="run" goto do_run

echo Unknown action: %ACTION%
goto fail

:ensure_windows_env
if /I not "%OS%"=="Windows_NT" (
  echo build.bat is only for Windows.
  exit /b 1
)

if not defined VCPKG_ROOT (
  set "VCPKG_ROOT=%USERPROFILE%\vcpkg"
)

if not exist "%VCPKG_ROOT%\scripts\buildsystems\vcpkg.cmake" (
  echo VCPKG_ROOT is not set to a valid vcpkg checkout.
  echo Expected: %%VCPKG_ROOT%%\scripts\buildsystems\vcpkg.cmake
  exit /b 1
)

if not defined QT_ROOT (
  if exist "C:\Qt\6.8.3\msvc2022_64\lib\cmake\Qt6\Qt6Config.cmake" (
    set "QT_ROOT=C:\Qt\6.8.3\msvc2022_64"
  )
)

if not defined QT_ROOT (
  echo QT_ROOT is not set.
  echo Set QT_ROOT to your Qt install root, for example C:\Qt\6.8.3\msvc2022_64
  exit /b 1
)

if not exist "%QT_ROOT%\lib\cmake\Qt6\Qt6Config.cmake" (
  echo QT_ROOT is invalid.
  echo Expected: %%QT_ROOT%%\lib\cmake\Qt6\Qt6Config.cmake
  exit /b 1
)

if defined VSCMD_VER (
  exit /b 0
)

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
  echo Could not find vswhere.exe.
  echo Install Visual Studio 2022 Community or Build Tools with the C++ workload.
  exit /b 1
)

set "VSINSTALL="
for /f "usebackq delims=" %%I in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do (
  set "VSINSTALL=%%I"
)

if not defined VSINSTALL (
  echo Could not find a Visual Studio installation with C++ tools.
  exit /b 1
)

set "VSDEVCMD=%VSINSTALL%\Common7\Tools\VsDevCmd.bat"
if not exist "%VSDEVCMD%" (
  echo Could not find VsDevCmd.bat.
  exit /b 1
)

set "DIFFY_SAVED_QT_ROOT=%QT_ROOT%"
set "DIFFY_SAVED_VCPKG_ROOT=%VCPKG_ROOT%"
call "%VSDEVCMD%" -host_arch=x64 -arch=x64 >nul
if errorlevel 1 (
  echo Failed to initialize the Visual Studio build environment.
  exit /b 1
)
set "QT_ROOT=%DIFFY_SAVED_QT_ROOT%"
set "VCPKG_ROOT=%DIFFY_SAVED_VCPKG_ROOT%"
set "DIFFY_SAVED_QT_ROOT="
set "DIFFY_SAVED_VCPKG_ROOT="

exit /b 0

:do_configure
echo [diffy] Configuring %PRESET%...
cmake --preset "%PRESET%" %FRESH%
exit /b %errorlevel%

:do_build
call :do_configure
if errorlevel 1 exit /b %errorlevel%
echo [diffy] Building %PRESET%...
cmake --build --preset "%PRESET%"
exit /b %errorlevel%

:do_test
call :do_build
if errorlevel 1 exit /b %errorlevel%
echo [diffy] Testing %PRESET%...
ctest --preset "%PRESET%"
exit /b %errorlevel%

:do_run
set "DIFFY_EXE=build\%PRESET%\bin\diffy.exe"
if not exist "%DIFFY_EXE%" (
  echo Could not find %DIFFY_EXE%
  echo Run build.bat --preset %PRESET% first.
  exit /b 1
)
echo [diffy] Running %DIFFY_EXE%...
"%DIFFY_EXE%"
exit /b %errorlevel%

:missing_preset
echo --preset requires a value.
echo.
goto usage_error

:usage
echo Usage:
echo   build.bat [--preset Debug^|Release] [--fresh]
echo   build.bat configure [--preset Debug^|Release] [--fresh]
echo   build.bat test [--preset Debug^|Release] [--fresh]
echo   build.bat run [--preset Debug^|Release] [--fresh]
echo.
echo Notes:
echo   - Defaults to Release.
echo   - Defaults VCPKG_ROOT to %%USERPROFILE%%\vcpkg when unset.
echo   - Defaults QT_ROOT to C:\Qt\6.8.3\msvc2022_64 when unset and present.
echo   - Auto-initializes the Visual Studio build environment when needed.
goto success

:usage_error
exit /b 1

:fail
popd >nul
exit /b 1

:success
popd >nul
exit /b 0
