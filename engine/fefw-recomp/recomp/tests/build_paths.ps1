# SPDX-License-Identifier: GPL-3.0-or-later
# Build-path regression for issue #38. A CMake/Ninja project is configured and built from source folders
# whose names contain spaces, &, parentheses, non-ASCII text and long components. CMAKE_NINJA_FORCE_RESPONSE_FILE
# is the setting the engine scripts pass; without it cmd.exe splits an object path at & while Ninja runs
# llvm-lib/link. This test checks the setting on a tiny compiler/linker/archiver project, not the full game.
#
# Run from the repository root (or any directory; the script uses its own location):
#   pwsh -NoProfile -File engine/fefw-recomp/recomp/tests/build_paths.ps1
# It needs cmake, ninja and clang-cl on PATH (or beside clang-cl for llvm-lib/lld-link). -Keep leaves the
# generated projects under the temporary folder for inspection.

[CmdletBinding()]
param(
    [string]$Root = (Join-Path ([IO.Path]::GetTempPath()) "EmblemWeave path test"),
    [string]$ClangCl = '',
    [switch]$Keep
)

$ErrorActionPreference = 'Stop'

function Find-Tool([string]$name, [string]$fallbackDir) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    if ($fallbackDir) {
        $beside = Join-Path $fallbackDir ($name + '.exe')
        if (Test-Path -LiteralPath $beside) { return $beside }
    }
    throw "cannot find $name; install CMake, Ninja and LLVM (clang-cl) first"
}

$cmake = Find-Tool 'cmake' $null
$ninja = Find-Tool 'ninja' $null
$clang = $ClangCl
if (-not $clang) {
    $cmd = Get-Command 'clang-cl' -ErrorAction SilentlyContinue
    if ($cmd) { $clang = $cmd.Source }
}
if (-not $clang) {
    $pf = $env:ProgramFiles
    $pf86 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
    $candidates = @()
    if ($pf) { $candidates += (Join-Path $pf 'LLVM\bin\clang-cl.exe') }
    if ($pf86) { $candidates += (Join-Path $pf86 'LLVM\bin\clang-cl.exe') }
    foreach ($p in $candidates) {
        if (Test-Path -LiteralPath $p) { $clang = $p; break }
    }
}
if (-not $clang) {
    throw "cannot find clang-cl; pass -ClangCl or install LLVM's Windows build"
}
$clang = (Resolve-Path -LiteralPath $clang).Path
$llvmBin = Split-Path -Parent $clang
$llvmLib = Find-Tool 'llvm-lib' $llvmBin
$lldLink = Find-Tool 'lld-link' $llvmBin
$env:PATH = $llvmBin + [IO.Path]::PathSeparator + $env:PATH

$cases = @(
    @{ name = 'spaces and ampersand'; leaf = 'A B & C' },
    @{ name = 'parentheses'; leaf = 'Port (x64) [test]' },
    @{ name = 'unicode'; leaf = 'café-Ünïcode-日本' },
    @{ name = 'long component'; leaf = ('long-' + ('x' * 90)) },
    @{ name = 'combination'; leaf = ('A & B (c) café ' + ('y' * 55)) }
)

function Write-Utf8([string]$path, [string]$text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
}

$failed = 0
foreach ($case in $cases) {
    $source = Join-Path (Join-Path $Root $case.leaf) 'source tree'
    $build = Join-Path $source 'ninja output (build)'
    New-Item -ItemType Directory -Force -Path $source | Out-Null
    New-Item -ItemType Directory -Force -Path $build | Out-Null

    Write-Utf8 (Join-Path $source 'CMakeLists.txt') @'
cmake_minimum_required(VERSION 3.25)
project(path_probe LANGUAGES CXX)
add_library(path_probe_lib STATIC lib.cpp)
target_include_directories(path_probe_lib PUBLIC .)
add_executable(path_probe main.cpp)
target_link_libraries(path_probe PRIVATE path_probe_lib)
'@
    Write-Utf8 (Join-Path $source 'lib.h') @'
#pragma once
int path_probe_value();
'@
    Write-Utf8 (Join-Path $source 'lib.cpp') @'
#include "lib.h"
int path_probe_value() { return 42; }
'@
    Write-Utf8 (Join-Path $source 'main.cpp') @'
#include "lib.h"
int main() { return path_probe_value() == 42 ? 0 : 1; }
'@

    $configure = @(
        '-S', $source, '-B', $build, '-G', 'Ninja',
        "-DCMAKE_BUILD_TYPE=Release",
        "-DCMAKE_CXX_COMPILER:FILEPATH=$clang",
        "-DCMAKE_AR:FILEPATH=$llvmLib",
        "-DCMAKE_LINKER:FILEPATH=$lldLink",
        '-DCMAKE_SUPPRESS_REGENERATION=ON',
        '-DCMAKE_NINJA_FORCE_RESPONSE_FILE=ON'
    )
    & $cmake @configure
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL configure: $($case.name)" -ForegroundColor Red
        $failed++
        continue
    }

    & $cmake --build $build --target path_probe
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL build: $($case.name)" -ForegroundColor Red
        $failed++
        continue
    }

    # CMake 4.4 no longer documents CMAKE_NINJA_FORCE_RESPONSE_FILE but uses response files for these
    # rules by default; older supported CMs need the setting. Check both the generated rules and the run.
    $rules = Join-Path $build 'CMakeFiles\rules.ninja'
    if (-not (Test-Path -LiteralPath $rules) -or -not (Select-String -LiteralPath $rules -Pattern '^\s*rspfile\s*=' -Quiet)) {
        Write-Host "FAIL no response-file rule: $($case.name)" -ForegroundColor Red
        $failed++
        continue
    }

    & (Join-Path $build 'path_probe.exe')
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL run: $($case.name)" -ForegroundColor Red
        $failed++
        continue
    }
    Write-Host "OK  $($case.name): $source" -ForegroundColor Green
}

if ($failed -ne 0) {
    throw "$failed of $($cases.Count) special-character build paths failed; projects are under $Root"
}
Write-Host "all $($cases.Count) special-character build paths passed" -ForegroundColor Green
if (-not $Keep) {
    Remove-Item -LiteralPath $Root -Recurse -Force -ErrorAction SilentlyContinue
}
