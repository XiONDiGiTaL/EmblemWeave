<#
.SYNOPSIS
    EmblemWeave Performance Benchmark Suite
.DESCRIPTION
    Runs fefw.exe through standardized gameplay scenarios and collects
    performance metrics from engine logs and perf_status.json.
    Results are saved as JSON for analysis.
.NOTES
    Requires: fefw.exe built and deployed to E:\EmblemWeave\data\play\
    Game data must be configured in fefw.ini
#>

param(
    [string]$FefwPath = "E:\EmblemWeave\data\play\fefw.exe",
    [string]$PlayDir = "E:\EmblemWeave\data\play",
    [string]$OutputDir = "E:\EmblemWeave\benchmarks",
    [string]$Scenario = "All",  # All, Menu, Hub, Crowded, Combat, Cinematic, Transition, Soak
    [int]$Duration = 0,         # 0 = use scenario default
    [switch]$Clean,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Scenario definitions with durations (seconds)
$Scenarios = @{
    "Menu" = @{
        Name = "Main Menu (Idle)"
        Duration = 60
        Description = "Sit at main menu for 60s"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent")
    }
    "Hub" = @{
        Name = "Hub Exploration"
        Duration = 120
        Description = "Walk through Somniel/hub area"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent", "scene")
    }
    "Crowded" = @{
        Name = "Dagsion Crowd"
        Duration = 120
        Description = "Dagsion with maximum NPCs on screen"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent", "scene", "spikes")
    }
    "Combat" = @{
        Name = "Character-Heavy Combat"
        Duration = 120
        Description = "Combat map with many units/effects"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent", "scene")
    }
    "Cinematic" = @{
        Name = "Cutscene"
        Duration = 0  # Full clip
        Description = "Full cinematic playback"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent", "scene")
    }
    "Transition" = @{
        Name = "Area Transition + Shader Compile"
        Duration = 180
        Description = "First visit to new area (shader compilation)"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "shader_backlog", "pipelines_created")
    }
    "Soak" = @{
        Name = "30-Minute Soak"
        Duration = 1800
        Description = "Extended stability test"
        Metrics = @("avg_fps", "low1_fps", "low01_fps", "cpu_ms", "gpu_ms", "vram_percent", "memory_mb", "stability")
    }
}

function Write-Log {
    param([string]$Message, [string]$Level = "INFO")
    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $color = switch ($Level) {
        "ERROR" { "Red" }
        "WARN"  { "Yellow" }
        "OK"    { "Green" }
        default { "Cyan" }
    }
    Write-Host "[$timestamp] [$Level] $Message" -ForegroundColor $color
}

function Check-Prerequisites {
    Write-Log "Checking prerequisites..."

    if (-not (Test-Path $FefwPath)) {
        throw "fefw.exe not found at $FefwPath"
    }
    Write-Log "Found fefw.exe: $FefwPath"

    if (-not (Test-Path (Join-Path $PlayDir "fefw.ini"))) {
        throw "fefw.ini not found in $PlayDir"
    }
    Write-Log "Found fefw.ini"

    if (-not (Test-Path $OutputDir)) {
        New-Item -ItemType Directory -Path $OutputDir | Out-Null
        Write-Log "Created output directory: $OutputDir"
    }

    # Check for game data
    $ini = Get-Content (Join-Path $PlayDir "fefw.ini")
    $romfs = ($ini | Select-String 'romfs\s*=').Line -replace '.*romfs\s*=\s*', ''
    if (-not (Test-Path $romfs.Trim())) {
        Write-Log "WARNING: RomFS path not accessible: $romfs" -Level "WARN"
    } else {
        Write-Log "RomFS accessible: $romfs"
    }
}

function Run-Scenario {
    param(
        [string]$ScenarioName,
        [int]$DurationSec,
        [string]$Description
    )

    Write-Log "============================================"
    Write-Log "Starting scenario: $ScenarioName"
    Write-Log "Description: $Description"
    Write-Log "Duration: $DurationSec seconds"
    Write-Log "============================================"

    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $scenarioDir = Join-Path $OutputDir "$timestamp-$ScenarioName"
    New-Item -ItemType Directory -Path $scenarioDir | Out-Null

    # Backup current logs
    $logFile = Join-Path $PlayDir "fefw.log"
    $perfFile = Join-Path $PlayDir "perf_status.json"
    $prevLog = Join-Path $scenarioDir "fefw.log.prev"
    $prevPerf = Join-Path $scenarioDir "perf_status.json.prev"
    if (Test-Path $logFile) { Copy-Item $logFile $prevLog -Force }
    if (Test-Path $perfFile) { Copy-Item $perfFile $prevPerf -Force }

    # Clear current log for clean measurement
    Clear-Content $logFile -ErrorAction SilentlyContinue

    # Environment variables for detailed logging
    $env:NXRT_LAUNCHER_DIR = "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp"
    $env:NXRT_DIAG_WATCHDOG_SECONDS = "30"
    $env:NXRT_GFX_LOG_FPS = "1"
    $env:NXRT_GFX_LOG_HITCHES = "1"

    # Start the game
    Write-Log "Launching fefw.exe..."
    $process = Start-Process -FilePath $FefwPath -WorkingDirectory $PlayDir -PassThru -WindowStyle Hidden

    # Wait for game to initialize (give it time to reach menu/gameplay)
    Write-Log "Waiting for game initialization (15s)..."
    Start-Sleep -Seconds 15

    if ($process.HasExited) {
        Write-Log "Process exited early with code $($process.ExitCode)" -Level "ERROR"
        $exitCode = $process.ExitCode
        return @{ Success = $false; ExitCode = $exitCode }
    }

    # Measure for duration
    if ($DurationSec -gt 0) {
        Write-Log "Measuring for $DurationSec seconds..."
        $startTime = Get-Date
        $elapsed = 0
        while ($elapsed -lt $DurationSec) {
            if ($process.HasExited) {
                Write-Log "Process exited during measurement" -Level "WARN"
                break
            }
            $elapsed = (Get-Date) - $startTime | Select-Object -ExpandProperty TotalSeconds
            $remaining = [math]::Ceiling($DurationSec - $elapsed)
            if ($Verbose) { Write-Progress -Activity "Benchmark: $ScenarioName" -Status "Remaining: ${remaining}s" -PercentComplete (100 - ($remaining * 100 / $DurationSec)) }
            Start-Sleep -Seconds 5
        }
    } else {
        Write-Log "Running until manual stop (Cinematic mode)..."
        Write-Log "Press Ctrl+C to stop"
        try {
            $process.WaitForExit()
        } catch {
            Write-Log "Stopped by user"
        }
    }

    # Graceful shutdown attempt
    if (-not $process.HasExited) {
        Write-Log "Stopping game..."
        $process.CloseMainWindow()
        $process.WaitForExit(5000)
        if (-not $process.HasExited) {
            Write-Log "Force killing..." -Level "WARN"
            $process.Kill()
        }
    }

    # Collect results
    Write-Log "Collecting metrics..."
    Start-Sleep -Seconds 2  # Let logs flush

    $result = @{
        Scenario = $ScenarioName
        Timestamp = $timestamp
        DurationSec = $DurationSec
        ExitCode = $process.ExitCode
        Success = ($process.ExitCode -eq 0 -or $process.ExitCode -eq -1)  # -1 = Ctrl+C
    }

    # Parse perf_status.json
    if (Test-Path $perfFile) {
        $perf = Get-Content $perfFile -Raw | ConvertFrom-Json
        $result.Performance = @{}
        $perf.PSObject.Properties | ForEach-Object {
            $result.Performance[$_.Name] = $_.Value
        }
        Copy-Item $perfFile (Join-Path $scenarioDir "perf_status.json") -Force
    } else {
        Write-Log "No perf_status.json found" -Level "WARN"
    }

    # Copy logs
    if (Test-Path $logFile) {
        Copy-Item $logFile (Join-Path $scenarioDir "fefw.log") -Force
        # Extract FPS lines
        $fpsLines = Get-Content $logFile | Select-String 'gfx: fps'
        $result.FpsSamples = $fpsLines.Count
        if ($fpsLines.Count -gt 0) {
            $fpsValues = $fpsLines | ForEach-Object {
                if ($_ -match 'fps\s+([\d.]+)') { [double]$matches[1] }
            }
            $result.FpsStats = @{
                Min = ($fpsValues | Measure-Object -Minimum).Minimum
                Max = ($fpsValues | Measure-Object -Maximum).Maximum
                Average = ($fpsValues | Measure-Object -Average).Average
                Count = $fpsValues.Count
            }
        }
    }

    # Extract hitch data
    if (Test-Path $logFile) {
        $hitchLines = Get-Content $logFile | Select-String 'hitch:'
        $result.HitchCount = $hitchLines.Count
        if ($hitchLines.Count -gt 0) {
            $hitchFrames = $hitchLines | ForEach-Object {
                if ($_ -match 'frame\s+(\d+)\s+took\s+([\d.]+)\s+ms') {
                    @{ Frame = [int]$matches[1]; Ms = [double]$matches[2] }
                }
            }
            $result.Hitches = $hitchFrames
        }
    }

    # Save result JSON
    $resultFile = Join-Path $scenarioDir "result.json"
    $result | ConvertTo-Json -Depth 10 | Set-Content $resultFile
    Write-Log "Results saved to $resultFile"

    return $result
}

function Generate-Report {
    param([array]$Results)

    $reportFile = Join-Path $OutputDir "benchmark-report-$(Get-Date -Format 'yyyyMMdd-HHmmss').md"

    $md = @()
    $md += "# EmblemWeave Performance Benchmark Report"
    $md += ""
    $md += "**Date:** $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
    $md += "**fefw.exe:** $FefwPath"
    $md += "**Engine:** $(Get-Content (Join-Path $PlayDir 'fefw.ini') | Select-String '\[engine\]' -Context 0,5 | ForEach-Object { $_.Context.PostContext })"
    $md += ""
    $md += "## Summary"
    $md += ""
    $md += "| Scenario | Avg FPS | 1% Low | 0.1% Low | CPU (ms) | GPU (ms) | VRAM % | Hitches |"
    $md += "|----------|---------|--------|----------|----------|----------|--------|---------|"

    foreach ($r in $Results) {
        if ($r.Success -and $r.Performance) {
            $p = $r.Performance
            $md += "| $($r.Scenario) | $($p.avg_fps) | $($p.low1_fps) | $($p.low01_fps) | $($p.cpu_ms) | $($p.gpu_ms) | $($p.vram_percent) | $($r.HitchCount) |"
        } else {
            $md += "| $($r.Scenario) | FAILED | - | - | - | - | - | - |"
        }
    }

    $md += ""
    $md += "## Detailed Results"
    $md += ""

    foreach ($r in $Results) {
        $md += "### $($r.Scenario)"
        $md += ""
        if ($r.Success) {
            $md += "- **Exit Code:** $($r.ExitCode)"
            $md += "- **Duration:** $($r.DurationSec)s"
            $md += "- **FPS Samples:** $($r.FpsSamples)"
            if ($r.FpsStats) {
                $md += "- **FPS Stats:** Min: $($r.FpsStats.Min), Max: $($r.FpsStats.Max), Avg: $([math]::Round($r.FpsStats.Average, 1))"
            }
            $md += "- **Hitches:** $($r.HitchCount)"
            if ($r.Hitches) {
                $worst = $r.Hitches | Sort-Object Ms -Descending | Select-Object -First 3
                $md += "- **Worst Hitches:**"
                foreach ($h in $worst) {
                    $md += "  - Frame $($h.Frame): $($h.Ms) ms"
                }
            }
            if ($r.Performance) {
                $md += "- **Performance Metrics:**"
                $r.Performance.PSObject.Properties | ForEach-Object {
                    $md += "  - $($_.Name): $($_.Value)"
                }
            }
        } else {
            $md += "- **FAILED** (Exit Code: $($r.ExitCode))"
        }
        $md += ""
    }

    $md += "## Acceptance Criteria Check"
    $md += ""
    $md += "| Metric | Target | Status |"
    $md += "|--------|--------|--------|"

    $crowded = $Results | Where-Object { $_.Scenario -eq "Crowded" -and $_.Success }
    if ($crowded) {
        $p = $crowded.Performance
        $avgOk = $p.avg_fps -ge 144
        $low1Ok = $p.low1_fps -ge 60
        $low01Ok = $p.low01_fps -ge 30
        $cpuOk = $p.cpu_ms -le 6.9
        $gpuOk = $p.gpu_ms -ge 4.0

        $avgStatus = if ($avgOk) { '✅ PASS' } else { '❌ FAIL' }
        $low1Status = if ($low1Ok) { '✅ PASS' } else { '❌ FAIL' }
        $low01Status = if ($low01Ok) { '✅ PASS' } else { '❌ FAIL' }
        $cpuStatus = if ($cpuOk) { '✅ PASS' } else { '❌ FAIL' }
        $gpuStatus = if ($gpuOk) { '✅ PASS' } else { '❌ FAIL' }

        $md += "| Avg FPS (Crowded) ≥ 144 | $([math]::Round($p.avg_fps, 1)) | $avgStatus |"
        $md += "| 1% Low FPS ≥ 60 | $([math]::Round($p.low1_fps, 1)) | $low1Status |"
        $md += "| 0.1% Low FPS ≥ 30 | $([math]::Round($p.low01_fps, 1)) | $low01Status |"
        $md += "| CPU Frame Time ≤ 6.9ms | $([math]::Round($p.cpu_ms, 2)) | $cpuStatus |"
        $md += "| GPU Frame Time ≥ 4.0ms | $([math]::Round($p.gpu_ms, 3)) | $gpuStatus |"
    }

    $md | Set-Content $reportFile -Encoding UTF8
    Write-Log "Report generated: $reportFile"
}

# Main
Write-Log "EmblemWeave Performance Benchmark Suite"
Write-Log "======================================="

Check-Prerequisites

$results = @()
$scenariosToRun = @()

if ($Scenario -eq "All") {
    $scenariosToRun = $Scenarios.Keys | Sort-Object
} else {
    $scenariosToRun = @($Scenario)
}

foreach ($s in $scenariosToRun) {
    if (-not $Scenarios.ContainsKey($s)) {
        Write-Log "Unknown scenario: $s" -Level "ERROR"
        continue
    }

    $def = $Scenarios[$s]
    $dur = if ($Duration -gt 0) { $Duration } else { $def.Duration }

    $result = Run-Scenario -ScenarioName $s -DurationSec $dur -Description $def.Description
    $results += $result

    # Cool down between scenarios
    if ($s -ne $scenariosToRun[-1]) {
        Write-Log "Cool down (10s)..."
        Start-Sleep -Seconds 10
    }
}

Generate-Report -Results $results

Write-Log "Benchmark suite complete!"
Write-Log "Results directory: $OutputDir"