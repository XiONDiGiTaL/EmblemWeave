@echo off
rem Each call starts from the environment its caller had. build_game.cmd calls this script once to configure and once to
rem build; without its own scope, the second call's vcvars64.bat ran with the first call's INCLUDE, LIB and PATH still set,
rem and the expanded environment crossed cmd.exe's 8191-character line limit ("The input line is too long.", exit 255).
setlocal EnableDelayedExpansion
rem Delayed expansion (!VAR!) carries the PATH work below. cmd.exe expands a line before running it and refuses one over
rem 8191 characters, and a caller's PATH can be most of that on its own, so the long values are never pasted into a line.
rem Sets up the build environment: MSVC x64 (vcvars64), LLVM clang-cl 23, CMake, Ninja, sccache.
rem Usage from cmd:  scripts\nx_env.cmd cmake --build build\x
rem                  scripts\nx_env.cmd            (no arguments: just prints the tool versions)
rem The launcher (apps/emblemweave/src/toolchain.rs) resolves the same toolchain, runs vcvars64 once, and starts the build
rem with its INCLUDE, LIB and PATH, setting NX_ENV_READY: this script then keeps that environment instead of running
rem vswhere and vcvars64 a second time (~2 s per call, and the PATH it appends is what crosses the 8191 limit).
rem --structured-events was removed: it emitted one build_started event with an empty build_id, then (after three
rem shifts) either printed the tool versions or ran the flag itself as a command. Nothing called it, and the launcher
rem reports the game build as its recompile stage, not this script.
if "%~1"=="--structured-events" (
    echo nx_env.cmd: --structured-events was removed; the launcher reports the game build as its recompile stage.
    exit /b 2
)
rem Tools installed elsewhere: NX_VCVARS (vcvars64.bat), NX_LLVM (LLVM bin folder, also read by cmake\clang-cl.cmake),
rem NX_TOOLCHAIN (a folder with llvm\bin, used when NX_LLVM is unset), NX_TOOLS (folders with cmake and ninja, ;-separated). Unset NX_VCVARS: the newest Visual Studio or Build Tools with the
rem x64 C++ tools (2022 or newer).
rem Visual Studio's own vcvars64.bat runs vswhere.exe by bare name, so the installer folder has to be on PATH before it is called; off a normal desktop shell it is not, and every build logged "'vswhere.exe' is not recognized" before carrying on.
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;!PATH!"
if defined NX_ENV_READY goto :have_vcvars
if not defined NX_VCVARS for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "NX_VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"
:have_vcvars
if not defined NX_LLVM if defined NX_TOOLCHAIN if exist "%NX_TOOLCHAIN%\llvm\bin\clang-cl.exe" set "NX_LLVM=%NX_TOOLCHAIN%\llvm\bin"
if not defined NX_LLVM set "NX_LLVM=%ProgramFiles%\LLVM\bin"
if not defined NX_TOOLS set "NX_TOOLS=%ProgramFiles%\CMake\bin"
rem vcvars64.bat appends the MSVC folders to PATH and expands the whole value on one line, so it hits the same 8191
rem limit once the caller's PATH is long. Give it a short PATH, keep what it defined aside, and put the caller's back.
if defined NX_ENV_READY goto :after_vcvars
set "NX_KEEP_PATH=!PATH!"
set "PATH=%SystemRoot%\system32;%SystemRoot%;%ProgramFiles(x86)%\Microsoft Visual Studio\Installer"
call "!NX_VCVARS!" >nul
set "NX_VCVARS_PATH=!PATH!"
set "PATH=!NX_KEEP_PATH!"
:after_vcvars
rem Build the PATH for the build environment: LLVM, CMake/Ninja, winget links, venv Scripts first.
rem Explicitly exclude the portable tools\bin (which has python312.dll) to avoid DLL version conflicts
rem with the venv Python (3.14). The venv Python must load its own extension modules, not the 3.12 ones.
set "TOOLS_BIN=%~dp0..\..\tools\bin"
set "CLEAN_PATH=%NX_LLVM%;%NX_TOOLS%;%LOCALAPPDATA%\Microsoft\WinGet\Links;%~dp0..\..\tools\.venv\Scripts;"
rem Remove tools\bin from the inherited PATH if present
set "INHERITED_PATH=!PATH!"
if defined INHERITED_PATH (
    set "INHERITED_PATH=;!INHERITED_PATH!;"
    set "INHERITED_PATH=!INHERITED_PATH:;%TOOLS_BIN%;=;!"
    if "!INHERITED_PATH:~0,1!"==";" set "INHERITED_PATH=!INHERITED_PATH:~1!"
    if "!INHERITED_PATH:~-1!"==";" set "INHERITED_PATH=!INHERITED_PATH:~0,-1!"
)
set "PATH=!CLEAN_PATH!!NX_VCVARS_PATH!!INHERITED_PATH!"

if "%~1"=="" (
  clang-cl --version & cmake --version & ninja --version & sccache --version
  exit /b 0
)
%*
