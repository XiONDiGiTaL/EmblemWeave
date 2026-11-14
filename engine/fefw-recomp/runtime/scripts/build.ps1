# Configure + build (+ test) the runtime. Run from anywhere:   pwsh scripts\build.ps1 [-Test] [-Config Release] [-Clean] [-Filter <regex>] [-Jobs N] [-CMakeArgs "-DX=Y ..."]
# Works relative to the checkout the script lives in, so git worktrees build on their own. Output: <checkout>\build\<config>
# sccache is used as the compiler launcher when it is on PATH. -Jobs (default NX_JOBS, else 10) caps the compile processes.
param([string]$Config = "Release", [switch]$Test, [switch]$Clean, [string]$Filter = "", [switch]$Verbose,
      [int]$Jobs = $(if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { 10 }), [string]$CMakeArgs = "")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$bd = Join-Path $root ("build\" + $Config.ToLower())
if ($Clean -and (Test-Path $bd)) { Remove-Item -Recurse -Force $bd }
$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$cfg = "cmake -S `"$root`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=$Config -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl"
$sccache = (Get-Command sccache -ErrorAction SilentlyContinue).Source
if (-not $sccache -and $env:NX_TOOLCHAIN -and (Test-Path (Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe'))) { $sccache = Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe' }
if ($sccache) {
    # one cache for every build folder, kept small (the disk is nearly full)
    if (-not $env:SCCACHE_DIR) { $env:SCCACHE_DIR = Join-Path $root "..\work\build\sccache" }
    if (-not $env:SCCACHE_CACHE_SIZE) { $env:SCCACHE_CACHE_SIZE = '4G' }
    $sccache = $sccache.Replace('\', '/')
    $cfg += " -DCMAKE_C_COMPILER_LAUNCHER=`"$sccache`" -DCMAKE_CXX_COMPILER_LAUNCHER=`"$sccache`""
}
if ($CMakeArgs) { $cfg += " $CMakeArgs" }
$bld = "cmake --build `"$bd`" -j $Jobs"
if ($Verbose) { $bld += " --verbose" }
$cmd = "call `"$env_cmd`" && $cfg && $bld"
if ($Test) { $cmd += " && ctest --test-dir `"$bd`" --output-on-failure"; if ($Filter) { $cmd += " -R $Filter" } }
cmd /c $cmd
exit $LASTEXITCODE
