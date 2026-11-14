@echo off
rem Builds the NXRT loader test fixtures into ..\data\ (relative to this script).
rem Output: fixture_aarch64.elf (full image) and fixture_nosections.elf (same image,
rem section header table removed via llvm-objcopy --strip-sections).
rem Intermediate object goes to %TEMP%\nxrt_fixture_build and is deleted at the end.
setlocal EnableExtensions

set "HERE=%~dp0"
set "LLVM=C:\Program Files\LLVM\bin"
set "SRC=%HERE%fixture.c"
set "OUTDIR=%HERE%..\data"
set "WORK=%TEMP%\nxrt_fixture_build"

if not exist "%LLVM%\clang.exe" goto :nollvm
if not exist "%OUTDIR%" mkdir "%OUTDIR%"
if exist "%WORK%" rmdir /s /q "%WORK%"
mkdir "%WORK%"
if errorlevel 1 goto :fail

"%LLVM%\clang.exe" --target=aarch64-linux-gnu -O1 -fPIC -ffreestanding -nostdlib -fno-stack-protector -fno-unwind-tables -fno-asynchronous-unwind-tables -ftls-model=initial-exec -c "%SRC%" -o "%WORK%\fixture.o"
if errorlevel 1 goto :fail

rem -shared (not -pie): lld only keeps undefined imports as dynamic relocations in a shared link.
rem -Bsymbolic: exported definitions become R_AARCH64_RELATIVE instead of symbol-based ABS64.
rem -z relro: fn_table_ro lands in PT_GNU_RELRO (read-only after relocation).
rem local-exec TLS is rejected by lld for -shared, so initial-exec is used.
"%LLVM%\ld.lld.exe" -shared -Bsymbolic -z relro -z max-page-size=4096 -z separate-code -e fixture_entry --hash-style=both "%WORK%\fixture.o" -o "%OUTDIR%\fixture_aarch64.elf"
if errorlevel 1 goto :fail

"%LLVM%\llvm-objcopy.exe" --strip-sections "%OUTDIR%\fixture_aarch64.elf" "%OUTDIR%\fixture_nosections.elf"
if errorlevel 1 goto :fail

rem fixture_relr.elf: the same object with packed relative relocations (DT_RELR), as SDK 20 modules have them.
"%LLVM%\ld.lld.exe" -shared -Bsymbolic -z relro -z pack-relative-relocs -z max-page-size=4096 -z separate-code -e fixture_entry --hash-style=both "%WORK%\fixture.o" -o "%OUTDIR%\fixture_relr.elf"
if errorlevel 1 goto :fail

rmdir /s /q "%WORK%"

for %%F in ("%OUTDIR%\fixture_aarch64.elf" "%OUTDIR%\fixture_nosections.elf" "%OUTDIR%\fixture_relr.elf") do echo %%~nxF %%~zF bytes
exit /b 0

:nollvm
echo error: clang.exe not found in "%LLVM%" 1>&2
exit /b 1

:fail
echo error: fixture build failed 1>&2
exit /b 1
