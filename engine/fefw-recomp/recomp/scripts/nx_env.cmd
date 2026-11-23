@echo off
rem Sets up the build environment: MSVC x64 (vcvars64), LLVM clang-cl 23, CMake, Ninja, sccache.
rem Usage from cmd:  scripts\nx_env.cmd cmake --build build\x
rem                  scripts\nx_env.cmd            (no arguments: just prints the tool versions)
rem                  scripts\nx_env.cmd --structured-events <build_id> [toolchain_lock_path] (emit JSON Lines events)
rem Tools installed elsewhere: NX_VCVARS (vcvars64.bat), NX_LLVM (LLVM bin folder, also read by cmake\clang-cl.cmake),
rem NX_TOOLCHAIN (a folder with llvm\bin, used when NX_LLVM is unset), NX_TOOLS (folders with cmake and ninja, ;-separated). Unset NX_VCVARS: the newest Visual Studio or Build Tools with the
rem x64 C++ tools (2022 or newer).
rem Visual Studio's own vcvars64.bat runs vswhere.exe by bare name, so the installer folder has to be on PATH before it is called; off a normal desktop shell it is not, and every build logged "'vswhere.exe' is not recognized" before carrying on.
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
if not defined NX_VCVARS for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "NX_VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"
if not defined NX_LLVM if defined NX_TOOLCHAIN if exist "%NX_TOOLCHAIN%\llvm\bin\clang-cl.exe" set "NX_LLVM=%NX_TOOLCHAIN%\llvm\bin"
if not defined NX_LLVM set "NX_LLVM=%ProgramFiles%\LLVM\bin"
if not defined NX_TOOLS set "NX_TOOLS=%ProgramFiles%\CMake\bin"
call "%NX_VCVARS%" >nul
rem Build the PATH for the build environment: LLVM, CMake/Ninja, winget links, venv Scripts first.
rem Explicitly exclude the portable tools\bin (which has python312.dll) to avoid DLL version conflicts
rem with the venv Python (3.14). The venv Python must load its own extension modules, not the 3.12 ones.
set "TOOLS_BIN=%~dp0..\..\tools\bin"
set "CLEAN_PATH=%NX_LLVM%;%NX_TOOLS%;%LOCALAPPDATA%\Microsoft\WinGet\Links;%~dp0..\..\tools\.venv\Scripts;"
rem Remove tools\bin from the inherited PATH if present
set "INHERITED_PATH=%PATH%"
if defined INHERITED_PATH (
    set "INHERITED_PATH=;%INHERITED_PATH%;"
    set "INHERITED_PATH=%INHERITED_PATH:;%TOOLS_BIN%;=;%"
    if "%INHERITED_PATH:~0,1%"==";" set "INHERITED_PATH=%INHERITED_PATH:~1%"
    if "%INHERITED_PATH:~-1%"==";" set "INHERITED_PATH=%INHERITED_PATH:~0,-1%"
)
set "PATH=%CLEAN_PATH%%INHERITED_PATH%"

rem Check for structured events mode
if "%~1"=="--structured-events" (
    rem Structured events mode: delegate to PowerShell module
    set "BUILD_ID=%~2"
    set "TOOLCHAIN_LOCK=%~3"
    if "%BUILD_ID%"=="" set "BUILD_ID=%~dp0..\..\tools\build_events.psm1"
    powershell -NoProfile -ExecutionPolicy Bypass -Command ^
        "Import-Module '%~dp0..\..\runtime\scripts\BuildEvents.psm1' -ArgumentList '%BUILD_ID%'; Initialize-EventWriter; Start-BuildEvent -ToolchainFingerprint '' -Config @{}"
    shift & shift & shift
)

if "%~1"=="" (
  clang-cl --version & cmake --version & ninja --version & sccache --version
  exit /b 0
)
%*
