@echo off
rem Sets up the MSVC + LLVM + CMake + Ninja environment for the runtime build (call it, then run cmake/ninja).
rem The tool folders can be set beforehand: NX_VCVARS (vcvars64.bat), NX_LLVM_BIN, NX_CMAKE_BIN, NX_NINJA_BIN. Unset, the local
rem toolchain folder NX_TOOLCHAIN (llvm\bin, cmakepy\cmake\data\bin under it) is used when it is set and exists, else the usual
rem install paths (Visual Studio through vswhere, LLVM and CMake under Program Files, ninja on PATH or from WinGet).
rem Exit codes: 3 no vcvars64.bat, 4 no INCLUDE from it (the C++ workload is missing), 5 no clang-cl, 6 no lld-link.
rem No parenthesized blocks here on purpose: a path with parentheses, like Program Files (x86), ends a block early.
set "NX_TC=%NX_TOOLCHAIN%"
if not defined NX_TOOLCHAIN set "NX_TC=%~dp0..\..\work\no-toolchain"

rem An inherited INCLUDE/LIB, or a compiler named through CC/CXX/CL, silently overrides everything chosen below.
set "CC="
set "CXX="
set "CL="
set "INCLUDE="
set "LIB="
set "LIBPATH="
set "CMAKE_C_COMPILER="
set "CMAKE_CXX_COMPILER="

if not defined NX_VCVARS if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" set "NX_VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
rem vcvars64.bat runs `vswhere.exe` by bare name, so the installer folder has to be on PATH before it is called.
if exist "%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" set "PATH=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer;%PATH%"
if not defined NX_VCVARS for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "NX_VCVARS=%%i\VC\Auxiliary\Build\vcvars64.bat"
if not defined NX_LLVM_BIN if exist "%NX_TC%\llvm\bin\clang-cl.exe" set "NX_LLVM_BIN=%NX_TC%\llvm\bin"
if not defined NX_LLVM_BIN set "NX_LLVM_BIN=C:\Program Files\LLVM\bin"
if not defined NX_CMAKE_BIN if exist "%NX_TC%\cmakepy\cmake\data\bin\cmake.exe" set "NX_CMAKE_BIN=%NX_TC%\cmakepy\cmake\data\bin"
if not defined NX_CMAKE_BIN set "NX_CMAKE_BIN=C:\Program Files\CMake\bin"
if not defined NX_NINJA_BIN for %%i in (ninja.exe) do if not "%%~$PATH:i"=="" set "NX_NINJA_BIN=%%~dp$PATH:i"
if not defined NX_NINJA_BIN set "NX_NINJA_BIN=%LOCALAPPDATA%\Microsoft\WinGet\Links"
rem the lifted-code tests build their AArch64 guests with the same LLVM (recomp/oracle/e2e/build_prog.py reads LLVM_BIN)
if not defined LLVM_BIN set "LLVM_BIN=%NX_LLVM_BIN%"

if not exist "%NX_VCVARS%" goto no_vcvars
call "%NX_VCVARS%" > "%TEMP%\emblemweave-vcvars.log" 2>&1
if not defined INCLUDE goto no_include
if not exist "%NX_LLVM_BIN%\clang-cl.exe" goto no_clang
if not exist "%NX_LLVM_BIN%\lld-link.exe" goto no_lld

rem ninja's folder goes last: a folder that holds ninja may also hold another compiler or an older cmake
set "PATH=%NX_LLVM_BIN%;%NX_CMAKE_BIN%;%PATH%;%NX_NINJA_BIN%"
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
exit /b 4

:no_clang
echo [emblemweave] clang-cl.exe was not found. Install LLVM 23 or newer, or set NX_LLVM_BIN to its bin folder.
exit /b 5

:no_lld
echo [emblemweave] lld-link.exe was not found beside clang-cl, and it is the linker CMake uses for it.
echo [emblemweave] Reinstall LLVM so its bin folder is complete.
exit /b 6
