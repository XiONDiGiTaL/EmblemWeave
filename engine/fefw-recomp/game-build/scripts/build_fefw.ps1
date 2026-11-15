# Configure + build fefw.exe into game-build\fefw\build\<config> (runtime sources are compiled in; the six lifted libraries are imported
# from work\build\fefw). -Target fefw_obj compiles fefw_main.cpp only (a check that needs no lifted libraries). -BuildDir puts the build
# elsewhere (a path relative to game-build\fefw, or absolute), so that several builds can run side by side.
#
# The environment is set up by one call to runtime\scripts\env.cmd, and the compiler is named by full path: whatever CC, CXX, INCLUDE
# or clang-cl an earlier shell left behind cannot change the toolchain this build uses. Configure and build are separate steps so a
# failure names which one it was.
param([string]$Config = "Release", [switch]$Clean, [string]$LibDir = $env:FEFW_LIB_DIR, [string]$Target = "fefw",
      [int]$Jobs = $(if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { 10 }), [string]$CMakeArgs = "", [string]$BuildDir = "")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root "fefw"
$bd = if ($BuildDir) { if ([System.IO.Path]::IsPathRooted($BuildDir)) { $BuildDir } else { Join-Path $src $BuildDir } }
      else { Join-Path $src ("build\" + $Config.ToLower()) }
if ($Clean -and (Test-Path $bd)) { Remove-Item -Recurse -Force $bd }
$env_cmd = Join-Path $root "..\runtime\scripts\env.cmd"

# The environment the run commands need, read in a child cmd and returned as a list of SET lines: vcvars64.bat defines INCLUDE and
# LIB, which a PowerShell process cannot inherit from a `call`.
$dump = & cmd /c "call `"$env_cmd`" >nul 2>&1 && set"
if ($LASTEXITCODE -ne 0) {
    throw "the developer environment could not be set up (runtime\scripts\env.cmd exited $LASTEXITCODE). Its message above names the missing piece; Visual Studio 2022 Build Tools with `"Desktop development with C++`" and LLVM are what it needs."
}
$envLines = @($dump | Where-Object { $_ -match '^[A-Za-z_][A-Za-z0-9_()]*=' })
$envMap = @{}
foreach ($line in $envLines) {
    $k, $v = $line -split '=', 2
    $envMap[$k] = $v
}
foreach ($needed in 'INCLUDE', 'LIB', 'NX_LLVM_BIN') {
    if (-not $envMap.ContainsKey($needed) -or [string]::IsNullOrWhiteSpace($envMap[$needed])) {
        throw "the developer environment has no $needed, so no compiler can find headers or libraries. `"Desktop development with C++`" in Visual Studio 2022 Build Tools is what defines it."
    }
}
$clang = Join-Path $envMap['NX_LLVM_BIN'] 'clang-cl.exe'
if (-not (Test-Path $clang)) { throw "clang-cl.exe is not in $($envMap['NX_LLVM_BIN']); install LLVM 23 or newer, or set NX_LLVM_BIN" }

$sccache = (Get-Command sccache -ErrorAction SilentlyContinue).Source
if (-not $sccache -and $env:NX_TOOLCHAIN -and (Test-Path (Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe'))) { $sccache = Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe' }

$cfg = "cmake -S `"$src`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=$Config -DCMAKE_C_COMPILER=`"$clang`" -DCMAKE_CXX_COMPILER=`"$clang`""
if ($sccache) {
    # one cache for every build folder, kept small (the disk is nearly full)
    if (-not $env:SCCACHE_DIR) { $env:SCCACHE_DIR = Join-Path $root "..\work\build\sccache" }
    if (-not $env:SCCACHE_CACHE_SIZE) { $env:SCCACHE_CACHE_SIZE = '4G' }
    $sccache = $sccache.Replace('\', '/')
    $cfg += " -DCMAKE_C_COMPILER_LAUNCHER=`"$sccache`" -DCMAKE_CXX_COMPILER_LAUNCHER=`"$sccache`""
}
if ($LibDir) { $cfg += " -DFEFW_LIB_DIR=`"$LibDir`"" }
if ($CMakeArgs) { $cfg += " $CMakeArgs" }

# One environment dump, applied to both commands, so configure and build cannot see different toolchains.
$prefix = "call `"$env_cmd`" >nul 2>&1 && "
$tgt = if ($Target -eq "fefw_obj") { "CMakeFiles/fefw.dir/src/fefw_main.cpp.obj" } else { $Target }

& cmd /c "$prefix$cfg"
if ($LASTEXITCODE -ne 0) {
    throw "CMake could not configure `"$bd`" (exit $LASTEXITCODE). A cache written by another compiler or an older CMake is the usual cause: delete its CMakeCache.txt, or pass -Clean. The output above names the actual error."
}
& cmd /c "$prefix cmake --build `"$bd`" --target $tgt -j $Jobs"
if ($LASTEXITCODE -ne 0) {
    throw "compiling $tgt failed (exit $LASTEXITCODE). The first `"error:`" line above names the file and the reason."
}
exit 0
