# Builds the launcher and installs it into a local EmblemWeave folder — the hand copy, without the EmblemWeave.exe.pre-*
# files a hand copy leaves beside the launcher.
#
#   pwsh -File tools/install_local.ps1                         # build, then install into E:\EmblemWeave
#   pwsh -File tools/install_local.ps1 -InstallDir D:\EmblemWeave
#   pwsh -File tools/install_local.ps1 -SkipBuild              # reuse target\release\EmblemWeave.exe
#
# The launcher it replaces is saved as <install>\backups\EmblemWeave.exe.pre-install-<date>-<time>. Only those
# pre-install copies are pruned, and only to the three newest: backups\EmblemWeave.exe.old (the launcher's Restore
# target) and every other file in backups\ are left alone.
param(
    [string]$InstallDir = 'E:\EmblemWeave',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$exeName = 'EmblemWeave.exe'
$built = Join-Path $repo "target\release\$exeName"

if (-not $SkipBuild) {
    Push-Location $repo
    try {
        & cargo build -p emblemweave --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}
if (-not (Test-Path -LiteralPath $built -PathType Leaf)) {
    throw "no launcher at $built; run without -SkipBuild, or build it first"
}

if (-not (Test-Path -LiteralPath $InstallDir -PathType Container)) { throw "no install folder at $InstallDir" }
$install = (Resolve-Path -LiteralPath $InstallDir).Path
$target = Join-Path $install $exeName
$backups = Join-Path $install 'backups'
New-Item -ItemType Directory -Force -Path $backups | Out-Null

$saved = $null
if (Test-Path -LiteralPath $target -PathType Leaf) {
    $saved = Join-Path $backups ("$exeName.pre-install-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    Copy-Item -LiteralPath $target -Destination $saved -Force
}

$builtHash = (Get-FileHash -LiteralPath $built -Algorithm SHA256).Hash
$stage = Join-Path $install (".$exeName.new-" + [guid]::NewGuid().ToString('N'))
try {
    # Copy beside the target first, verify it, then replace. A failed copy leaves the installed launcher untouched; a
    # failed replace restores the pre-install copy instead of leaving a truncated executable.
    Copy-Item -LiteralPath $built -Destination $stage -Force
    $stageHash = (Get-FileHash -LiteralPath $stage -Algorithm SHA256).Hash
    if ($stageHash -ne $builtHash) { throw "the staged launcher does not match the build at $built" }
    if (Test-Path -LiteralPath $target -PathType Leaf) {
        # File.Replace's three-argument overload needs a real backup path, not $null. $saved is the verified
        # pre-install copy; replacing through it atomically keeps the old launcher available for rollback.
        if (-not $saved) { throw "no pre-install backup path for $target" }
        [System.IO.File]::Replace($stage, $target, $saved)
    } else {
        Move-Item -LiteralPath $stage -Destination $target
    }
} catch {
    Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue
    if ($saved) { Copy-Item -LiteralPath $saved -Destination $target -Force -ErrorAction SilentlyContinue }
    throw "could not replace $target (is EmblemWeave still open?): $($_.Exception.Message)"
}
$targetHash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
if ($builtHash -ne $targetHash) {
    if ($saved) { Copy-Item -LiteralPath $saved -Destination $target -Force }
    throw "the installed launcher does not match the build at $built; restored $saved"
}

$mine = @(Get-ChildItem -LiteralPath $backups -File |
          Where-Object { $_.Name -match '^EmblemWeave\.exe\.pre-install-\d{8}-\d{6}$' } |
          Sort-Object Name -Descending)
$pruned = 0
if ($mine.Count -gt 3) {
    foreach ($old in $mine[3..($mine.Count - 1)]) {
        Remove-Item -LiteralPath $old.FullName -Force
        $pruned++
    }
}

Write-Host "installed $target"
if ($saved) { Write-Host "replaced launcher saved as $saved" }
Write-Host ("install backups kept: {0}; pruned: {1}" -f [Math]::Min($mine.Count, 3), $pruned)
$rollback = Join-Path $backups "$exeName.old"
if (Test-Path -LiteralPath $rollback -PathType Leaf) { Write-Host "rollback copy kept: $rollback" }
