@echo off
rem Sets up the MSVC + LLVM + CMake + Ninja environment for the runtime build (call it, then run cmake/ninja).
rem The tool folders can be set beforehand: NX_VCVARS (vcvars64.bat), NX_LLVM_BIN, NX_CMAKE_BIN, NX_NINJA_BIN. Unset, the local toolchain
rem folder NX_TOOLCHAIN (llvm\bin, cmakepy\cmake\data\bin under it) is used when it is set and exists, else the usual install
rem paths (Visual Studio through vswhere, LLVM and CMake under Program Files, ninja on PATH or from WinGet).
set "NX_TC=%NX_TOOLCHAIN%"
if not defined NX_TOOLCHAIN set "NX_TC=%~dp0..\..\work\no-toolchain"
if not defined NX_VCVARS if exist "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" set "NX_VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
rem Visual Studio's own vcvars64.bat runs `vswhere.exe` by bare name, so the installer folder has to be on PATH before it is called.
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
call "%NX_VCVARS%" >nul
rem ninja's folder goes last: a folder that holds ninja may also hold another compiler or an older cmake
set "PATH=%NX_LLVM_BIN%;%NX_CMAKE_BIN%;%PATH%;%NX_NINJA_BIN%"
