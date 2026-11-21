<#
.SYNOPSIS
    Configure + build (+ test) the fefw-recomp runtime with structured build events.
.DESCRIPTION
    Builds the C++ runtime using clang-cl, CMake, and Ninja.
    Emits JSON Lines structured build events for monitoring and CI integration.
    Uses adaptive memory-aware concurrency calculation.
.OUTPUTS
    <checkout>\build\<config>
    JSON Lines events on stdout when -StructuredEvents is specified.
#>

param(
    [string]$Config = "Release",
    [switch]$Test,
    [switch]$Clean,
    [string]$Filter = "",
    [switch]$Verbose,
    [int]$Jobs = 0,
    [string]$CMakeArgs = "",
    [ValidateSet('conservative','aggressive','max')][string]$ConcurrencyMode = 'conservative',
    [switch]$StructuredEvents,
    [string]$BuildId = $([guid]::NewGuid().ToString()),
    [string]$ToolchainLockPath = "$PSScriptRoot\..\tools.lock"
)

$ErrorActionPreference = "Stop"

# Import build events module if structured events requested
if ($StructuredEvents) {
    Import-Module "$PSScriptRoot\BuildEvents.psm1" -ArgumentList $BuildId
    Initialize-EventWriter
}

function Get-AdaptiveJobCount {
    param(
        [ValidateSet('conservative','aggressive','max')][string]$Mode = 'conservative',
        [int]$ManualOverride = 0
    )
    if ($ManualOverride -gt 0) { return $ManualOverride }

    # Get logical processor count
    $cores = (Get-CimInstance Win32_Processor | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum
    if (-not $cores -or $cores -lt 1) { $cores = [Environment]::ProcessorCount }

    # Get available physical memory in GB
    $memGB = [math]::Floor((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)

    # Estimate memory per compile job (clang-cl with debug info can use 1-2 GB)
    $memPerJobGB = 1.5

    switch ($Mode) {
        'conservative' {
            # Leave 1-2 cores free for system, limit by RAM with 4GB headroom
            $byCores = [math]::Max(1, $cores - 2)
            $byMem = [math]::Max(1, [math]::Floor(($memGB - 4) / $memPerJobGB))
            return [math]::Min($byCores, $byMem)
        }
        'aggressive' {
            # Use all cores, limit by RAM with 2GB headroom
            $byCores = $cores
            $byMem = [math]::Max(1, [math]::Floor(($memGB - 2) / $memPerJobGB))
            return [math]::Min($byCores, $byMem)
        }
        'max' {
            # No RAM limit, all cores
            return $cores
        }
    }
}

function Get-ToolchainFingerprint {
    $vswhere = "$env:ProgramFiles(x86)\Microsoft Visual Studio\Installer\vswhere.exe"
    $vsPath = if (Test-Path $vswhere) {
        & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    } else { "" }

    $msvcVer = if (Test-Path "$vsPath\VC\Tools\MSVC") {
        Get-ChildItem "$vsPath\VC\Tools\MSVC" | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty Name
    } else { "" }

    $clangVer = ""
    $lldPresent = $false
    $clangPaths = @("$env:NX_LLVM", "$env:ProgramFiles\LLVM\bin", "$env:LOCALAPPDATA\Microsoft\WinGet\Links")
    foreach ($p in $clangPaths) {
        if (Test-Path "$p\clang-cl.exe") {
            $clangVer = (& "$p\clang-cl.exe" --version 2>$null | Select-String -Pattern "clang version ([\d.]+)" | ForEach-Object { $_.Matches[0].Groups[1].Value })
            $lldPresent = Test-Path "$p\lld-link.exe"
            break
        }
    }

    $cmakeVer = (cmake --version 2>$null | Select-String -Pattern "cmake version ([\d.]+)" | ForEach-Object { $_.Matches[0].Groups[1].Value })
    $ninjaVer = (ninja --version 2>$null)

    return "$vsPath|$msvcVer|$clangVer|$lldPresent|$cmakeVer|$ninjaVer"
}

$root = Split-Path -Parent $PSScriptRoot
$bd = Join-Path $root ("build\" + $Config.ToLower())

# Emit build started event
if ($StructuredEvents) {
    $fingerprint = Get-ToolchainFingerprint
    $buildConfig = @{
        jobs = 0  # Will be updated after job calculation
        memory_limit_mb = $null
        sccache_enabled = $false
        incremental = -not $Clean
        target = $Config
        features = @()
    }
    Start-BuildEvent -ToolchainFingerprint $fingerprint -Config $buildConfig
}

if ($Clean -and (Test-Path $bd)) {
    if ($StructuredEvents) { Warning-Event "Cleaning build directory: $bd" }
    Remove-Item -Recurse -Force $bd
}

# Resolve job count: explicit -Jobs > NX_JOBS env > adaptive
if ($Jobs -le 0) {
    $Jobs = if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { Get-AdaptiveJobCount -Mode $ConcurrencyMode }
}

if ($StructuredEvents) {
    $cores = (Get-CimInstance Win32_Processor | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum
    $memGB = [math]::Floor((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
    Write-Host "Build concurrency: $Jobs jobs (mode: $ConcurrencyMode, cores: $cores, RAM: $memGB GB)" -ForegroundColor Cyan
}

# Update build config with actual jobs
if ($StructuredEvents) {
    $buildConfig.jobs = $Jobs
}

$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$cfg = "cmake -S `"$root`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=$Config -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl"

# Configure sccache
$sccache = (Get-Command sccache -ErrorAction SilentlyContinue).Source
if (-not $sccache -and $env:NX_TOOLCHAIN -and (Test-Path (Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe'))) {
    $sccache = Join-Path $env:NX_TOOLCHAIN 'bin\sccache.exe'
}
if ($sccache) {
    if ($StructuredEvents) { $buildConfig.sccache_enabled = $true }
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

# Stage 1: Configure
if ($StructuredEvents) {
    Start-StageEvent -StageId "configure" -StageName "Configure (CMake)" -StageIndex 1 -TotalStages 3
}

try {
    if ($StructuredEvents) { Progress-StageEvent -StageId "configure" -Progress 0.1 -Message "Running CMake configure" }
    $result = cmd /c "call `"$env_cmd`" && $cfg" 2>&1
    if ($LASTEXITCODE -ne 0) {
        if ($StructuredEvents) { Fail-StageEvent -StageId "configure" -Error $result }
        throw "CMake configure failed with exit code $LASTEXITCODE"
    }
    if ($StructuredEvents) {
        Progress-StageEvent -StageId "configure" -Progress 1.0 -Message "CMake configure completed"
        Complete-StageEvent -StageId "configure" -Success $true -Output "CMake configure successful"
    }
}
catch {
    if ($StructuredEvents) {
        Fail-StageEvent -StageId "configure" -Error $_.ToString()
        Complete-BuildEvent -Success $false -Summary @{ stages_completed = 0; stages_failed = 1; total_warnings = 0; total_errors = 1; cache_hits = 0; cache_misses = 0; peak_memory_mb = 0 }
        Close-EventWriter
    }
    exit $LASTEXITCODE
}

# Stage 2: Build
if ($StructuredEvents) {
    Start-StageEvent -StageId "build" -StageName "Build (Ninja)" -StageIndex 2 -TotalStages 3
}

try {
    if ($StructuredEvents) { Progress-StageEvent -StageId "build" -Progress 0.1 -Message "Starting Ninja build with $Jobs jobs" }
    $result = cmd /c $bld 2>&1
    if ($LASTEXITCODE -ne 0) {
        if ($StructuredEvents) { Fail-StageEvent -StageId "build" -Error $result }
        throw "Ninja build failed with exit code $LASTEXITCODE"
    }
    if ($StructuredEvents) {
        Progress-StageEvent -StageId "build" -Progress 1.0 -Message "Ninja build completed"
        Complete-StageEvent -StageId "build" -Success $true -Output "Build successful"
    }
}
catch {
    if ($StructuredEvents) {
        Fail-StageEvent -StageId "build" -Error $_.ToString()
        Complete-BuildEvent -Success $false -Summary @{ stages_completed = 1; stages_failed = 1; total_warnings = 0; total_errors = 1; cache_hits = 0; cache_misses = 0; peak_memory_mb = 0 }
        Close-EventWriter
    }
    exit $LASTEXITCODE
}

# Stage 3: Test (if requested)
$testSuccess = $true
$testOutput = ""
if ($Test) {
    if ($StructuredEvents) {
        Start-StageEvent -StageId "test" -StageName "Test (CTest)" -StageIndex 3 -TotalStages 3
        Progress-StageEvent -StageId "test" -Progress 0.1 -Message "Running CTest"
    }
    try {
        $testCmd = "call `"$env_cmd`" && ctest --test-dir `"$bd`" --output-on-failure"
        if ($Filter) { $testCmd += " -R $Filter" }
        $testResult = cmd /c $testCmd 2>&1
        if ($LASTEXITCODE -ne 0) {
            $testSuccess = $false
            $testOutput = $testResult
            if ($StructuredEvents) { Fail-StageEvent -StageId "test" -Error $testResult }
        } else {
            $testOutput = $testResult
            if ($StructuredEvents) {
                Progress-StageEvent -StageId "test" -Progress 1.0 -Message "Tests completed"
                Complete-StageEvent -StageId "test" -Success $true -Output "Tests passed"
            }
        }
    }
    catch {
        $testSuccess = $false
        $testOutput = $_.ToString()
        if ($StructuredEvents) { Fail-StageEvent -StageId "test" -Error $_.ToString() }
    }
}

# Finalize
if ($StructuredEvents) {
    $stagesCompleted = if ($Test) { if ($testSuccess) { 3 } else { 2 } } else { 2 }
    $stagesFailed = if ($Test -and -not $testSuccess) { 1 } else { 0 }
    $summary = @{
        stages_completed = $stagesCompleted
        stages_failed = $stagesFailed
        total_warnings = 0
        total_errors = if ($testSuccess) { 0 } else { 1 }
        cache_hits = 0  # Would need sccache stats
        cache_misses = 0
        peak_memory_mb = 0
    }
    Complete-BuildEvent -Success ($testSuccess -and $LASTEXITCODE -eq 0) -Summary $summary
    Close-EventWriter
}

exit $LASTEXITCODE