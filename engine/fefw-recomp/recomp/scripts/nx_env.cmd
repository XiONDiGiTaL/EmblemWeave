@echo off
rem Sets up the build environment: MSVC x64 (vcvars64), LLVM clang-cl 23, CMake, Ninja, sccache.
rem Usage from cmd:  scripts\nx_env.cmd cmake --build build\x
rem                  scripts\nx_env.cmd            (no arguments: just prints the tool versions)
rem Tools installed elsewhere: NX_VCVARS (vcvars64.bat), NX_LLVM (LLVM bin folder, also read by cmake\clang-cl.cmake),
rem NX_TOOLCHAIN (a folder with llvm\bin, used when NX_LLVM is unset), NX_TOOLS (folders with cmake and ninja, ;-separated).
rem Unset NX_VCVARS: the newest Visual Studio or Build Tools with the x64 C++ tools (2022 or newer).
rem
rem Every check stops the build with a named reason instead of letting it run and fail on a header twenty minutes later.
rem Exit codes: 3 no vcvars64.bat, 4 vcvars64.bat failed, 5 no INCLUDE from it (the C++ workload is missing),
rem 6 no clang-cl, 7 no lld-link, 8 no cmake, 9 no ninja. No parenthesized blocks here on purpose: a path that contains
rem parentheses, like Program Files (x86), ends a block early and turns the rest of the script into stray commands.

rem A shell that already ran another vcvars, or a compiler named through CC/CXX/CL, would override the toolchain chosen here.
set "CC="
set "CXX="
set "CL="
set "INCLUDE="
set "LIB="
set "LIBPATH="
set "CMAKE_C_COMPILER="
set "CMAKE_CXX_COMPILER="

rem vcvars64.bat runs `vswhere.exe` by bare name, so the installer folder has to be on PATH before it is called.
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
if not defined NX_VCVARS for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "NX_VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"
if not defined NX_LLVM if defined NX_TOOLCHAIN if exist "%NX_TOOLCHAIN%\llvm\bin\clang-cl.exe" set "NX_LLVM=%NX_TOOLCHAIN%\llvm\bin"
if not defined NX_LLVM set "NX_LLVM=C:\Program Files\LLVM\bin"
if not defined NX_TOOLS set "NX_TOOLS=C:\Program Files\CMake\bin"

if not exist "%NX_VCVARS%" goto no_vcvars
call "%NX_VCVARS%" > "%TEMP%\emblemweave-vcvars.log" 2>&1
if not defined INCLUDE goto no_include
if not exist "%NX_LLVM%\clang-cl.exe" goto no_clang
if not exist "%NX_LLVM%\lld-link.exe" goto no_lld

set "PATH=%NX_LLVM%;%NX_TOOLS%;%LOCALAPPDATA%\Microsoft\WinGet\Links;%~dp0..\..\tools\.venv\Scripts;%PATH%"
where cmake >nul 2>&1
if errorlevel 1 goto no_cmake
where ninja >nul 2>&1
if errorlevel 1 goto no_ninja

if "%~1"=="" goto versions
%*
exit /b %errorlevel%

:versions
echo clang-cl: %NX_LLVM%\clang-cl.exe
clang-cl --version | findstr /i "clang version"
cmake --version | findstr /i "version"
ninja --version
where sccache >nul 2>&1
if not errorlevel 1 sccache --version
exit /b 0

:no_vcvars
echo [emblemweave] Visual Studio's vcvars64.bat was not found, or vswhere reported no install with the x64 C++ tools.
echo [emblemweave] Install Visual Studio 2022 Build Tools with "Desktop development with C++", or set NX_VCVARS to the
echo [emblemweave] full path of a vcvars64.bat.
exit /b 3

:no_include
echo [emblemweave] vcvars64.bat ran but set no INCLUDE, so it brings no C++ headers: the C++ workload is missing from
echo [emblemweave] that Visual Studio install. Its output is in %TEMP%\emblemweave-vcvars.log.
echo [emblemweave] Open Visual Studio Installer, modify the install, and add "Desktop development with C++".
exit /b 5

:no_clang
echo [emblemweave] clang-cl.exe was not found. Install LLVM 23 or newer, or set NX_LLVM to the folder that holds
echo [emblemweave] clang-cl.exe.
exit /b 6

:no_lld
echo [emblemweave] lld-link.exe was not found beside clang-cl, and it is the linker CMake uses for it.
echo [emblemweave] Reinstall LLVM so its bin folder is complete.
exit /b 7

:no_cmake
echo [emblemweave] cmake.exe is not on PATH. Install CMake 3.25 or newer.
exit /b 8

:no_ninja
echo [emblemweave] ninja.exe is not on PATH. Install Ninja, or put its folder on PATH.
exit /b 9
