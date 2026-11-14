@echo off
rem Sets up the build environment: MSVC x64 (vcvars64), LLVM clang-cl 23, CMake, Ninja, sccache.
rem Usage from cmd:  scripts\nx_env.cmd cmake --build build\x
rem                  scripts\nx_env.cmd            (no arguments: just prints the tool versions)
rem Tools installed elsewhere: NX_VCVARS (vcvars64.bat), NX_LLVM (LLVM bin folder, also read by cmake\clang-cl.cmake),
rem NX_TOOLCHAIN (a folder with llvm\bin, used when NX_LLVM is unset), NX_TOOLS (folders with cmake and ninja, ;-separated). Unset NX_VCVARS: the newest Visual Studio or Build Tools with the
rem x64 C++ tools (2022 or newer).
rem Visual Studio's own vcvars64.bat runs `vswhere.exe` by bare name, so the installer folder has to be on PATH before it is called; off a normal desktop shell it is not, and every build logged "'vswhere.exe' is not recognized" before carrying on.
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
if not defined NX_VCVARS for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "NX_VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"
if not defined NX_LLVM if defined NX_TOOLCHAIN if exist "%NX_TOOLCHAIN%\llvm\bin\clang-cl.exe" set "NX_LLVM=%NX_TOOLCHAIN%\llvm\bin"
if not defined NX_LLVM set "NX_LLVM=C:\Program Files\LLVM\bin"
if not defined NX_TOOLS set "NX_TOOLS=C:\Program Files\CMake\bin"
call "%NX_VCVARS%" >nul
set "PATH=%NX_LLVM%;%NX_TOOLS%;%LOCALAPPDATA%\Microsoft\WinGet\Links;%~dp0..\..\tools\.venv\Scripts;%PATH%"
if "%~1"=="" (
  clang-cl --version & cmake --version & ninja --version & sccache --version
  exit /b 0
)
%*
