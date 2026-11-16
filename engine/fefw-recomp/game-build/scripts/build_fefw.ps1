# Configure + build fefw.exe into game-build\fefw\build\<config> (runtime sources are compiled in; the six lifted libraries are imported
# from work\build\fefw). -Target fefw_obj compiles fefw_main.cpp only (a check that needs no lifted libraries). -BuildDir puts the build
# elsewhere (a path relative to game-build\fefw, or absolute), so that several builds can run side by side.
param([string]$Config = "Release", [switch]$Clean, [string]$LibDir = $env:FEFW_LIB_DIR, [string]$Target = "fefw",
      [int]$Jobs = $(if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { 10 }), [string]$CMakeArgs = "", [string]$BuildDir = "")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root "fefw"
$bd = if ($BuildDir) { if ([System.IO.Path]::IsPathRooted($BuildDir)) { $BuildDir } else { Join-Path $src $BuildDir } }
      else { Join-Path $src ("build\" + $Config.ToLower()) }
if ($Clean -and (Test-Path $bd)) { Remove-Item -Recurse -Force $bd }
$env_cmd = Join-Path $root "..\runtime\scripts\env.cmd"
# Both compilers and the resource compiler go into the first configure, by full path when they are there: a cache that gets one of
# them afterwards makes CMake wipe itself and configure again with its defaults (Debug, the clang++ driver), which is issue #22.
$llvm_bin = if ($env:NX_LLVM_BIN) { $env:NX_LLVM_BIN } else { 'C:\Program Files\LLVM\bin' }
$clang_cl = if (Test-Path (Join-Path $llvm_bin 'clang-cl.exe')) { (Join-Path $llvm_bin 'clang-cl.exe').Replace('\', '/') } else { 'clang-cl' }
# A build folder configured by an older release was configured for the clang++ driver, and CMake keeps the MSVC flags it wrote from then
# when the compiler is swapped on it later: the flags reach clang++ as `-clang:-fmacro-prefix-map=...` and `/W3`, which it rejects (issue
# #25). Nothing in that folder is worth keeping, so it is started over when the compiler it recorded is not the one this run uses.
$cache = Join-Path $bd 'CMakeCache.txt'
if (Test-Path $cache) {
    $recorded = @(Select-String -Path $cache -Pattern '^CMAKE_C(XX)?_COMPILER:[^=]*=(.+)$' | ForEach-Object { $_.Matches[0].Groups[2].Value.Trim() })
    $stale = @($recorded | Where-Object { $_ -and ((Split-Path -Leaf $_).ToLower() -ne (Split-Path -Leaf $clang_cl).ToLower()) })
    if ($stale.Count -gt 0) {
        Write-Host "this build folder was configured for $($stale -join ', '); starting it over for $(Split-Path -Leaf $clang_cl)"
        Remove-Item -Recurse -Force $bd
    }
}
$llvm_rc = if (Test-Path (Join-Path $llvm_bin 'llvm-rc.exe')) { (Join-Path $llvm_bin 'llvm-rc.exe').Replace('\', '/') } else { '' }
# The compilers are named twice: as typed cache entries for this configure, and in the environment. `launcher.rc` enables the RC
# language part-way through the configure, and a value that arrived untyped on the command line is re-typed when that happens;
# CMake then says "you have changed variables that require your cache to be deleted", configures again without these -D flags,
# and the build continues with whatever clang.exe is on PATH (the GNU frontend, which rejects /W3 and -clang: flags) in Debug
# (issues 9, 33). CC/CXX/RC survive that announcement, the typed entries make it unnecessary.
$env:CC = $clang_cl
$env:CXX = $clang_cl
if ($llvm_rc) { $env:RC = $llvm_rc }
# CMAKE_NINJA_FORCE_RESPONSE_FILE: an absolute source path with `&` (for example `PC Ports & Recomps`) is split by cmd.exe when
# Ninja puts an object path on the command line (issue #38). A response file passes spaces, `&`, parentheses, Unicode and long
# object lists as one argument without changing the compiler or linker flags.
$cfg = "cmake -S `"$src`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=$Config -DCMAKE_C_COMPILER:FILEPATH=`"$clang_cl`" -DCMAKE_CXX_COMPILER:FILEPATH=`"$clang_cl`" -DCMAKE_SUPPRESS_REGENERATION=ON -DCMAKE_NINJA_FORCE_RESPONSE_FILE=ON"
if ($llvm_rc) { $cfg += " -DCMAKE_RC_COMPILER:FILEPATH=`"$llvm_rc`"" }
$sccache = (Get-Command sccache -ErrorAction SilentlyContinue).Source
if (-not $sccache -and $env:NX_TOOLCHAIN -and (Test-Path (Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe'))) { $sccache = Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe' }
if ($sccache) {
    # one cache for every build folder, kept small (the disk is nearly full)
    if (-not $env:SCCACHE_DIR) { $env:SCCACHE_DIR = Join-Path $root "..\work\build\sccache" }
    if (-not $env:SCCACHE_CACHE_SIZE) { $env:SCCACHE_CACHE_SIZE = '4G' }
    $sccache = $sccache.Replace('\', '/')
    $cfg += " -DCMAKE_C_COMPILER_LAUNCHER=`"$sccache`" -DCMAKE_CXX_COMPILER_LAUNCHER=`"$sccache`""
}
if ($LibDir) { $cfg += " -DFEFW_LIB_DIR=`"$LibDir`"" }
if ($CMakeArgs) { $cfg += " $CMakeArgs" }
$tgt = if ($Target -eq "fefw_obj") { "CMakeFiles/fefw.dir/src/fefw_main.cpp.obj" } else { $Target }
# What the cache says after the configure, which is what the build actually uses. A configure that ended in the middle of a cache
# reset leaves this folder configured with another compiler or another configuration, and its object files cannot be mixed with
# this run's, so it is configured once more (the flags are explicit, so the second pass cannot be reset again) and checked again.
function CacheValue([string]$name) {
    $line = Select-String -Path (Join-Path $bd 'CMakeCache.txt') -Pattern "^$([regex]::Escape($name)):[^=]*=(.*)$" | Select-Object -First 1
    if ($line) { $line.Matches[0].Groups[1].Value.Trim() } else { '' }
}
function CacheMatches {
    $c = CacheValue 'CMAKE_C_COMPILER'
    $t = CacheValue 'CMAKE_BUILD_TYPE'
    ((Split-Path -Leaf $c).ToLower() -eq (Split-Path -Leaf $clang_cl).ToLower()) -and ($t -eq $Config)
}
if ($Target -ne "fefw_obj") {
    cmd /c "call `"$env_cmd`" && $cfg"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if (-not (CacheMatches)) {
        # An object file made by clang++ is not one clang-cl can link with, and a Debug object is not a Release one, so the folder
        # goes rather than being half-reconfigured: CMake cannot swap a compiler on an existing cache without deleting it anyway.
        Write-Host "this build folder is configured with $(CacheValue 'CMAKE_C_COMPILER') in $(CacheValue 'CMAKE_BUILD_TYPE'); starting it over with $(Split-Path -Leaf $clang_cl) and $Config"
        Remove-Item -Recurse -Force $bd
        cmd /c "call `"$env_cmd`" && $cfg"
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        if (-not (CacheMatches)) {
            Write-Host "the configure still ends with $(CacheValue 'CMAKE_C_COMPILER') in $(CacheValue 'CMAKE_BUILD_TYPE') after a fresh folder; the log above names the configure that changed it"
            exit 2
        }
    }
}
cmd /c "call `"$env_cmd`" && $cfg && cmake --build `"$bd`" --target $tgt -j $Jobs"
exit $LASTEXITCODE
