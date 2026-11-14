# Dot-sourced by the scripts that need the folders around the checkout. Sets $proj to the project folder (the one that holds runtime\, work\,
# game-build\ and recomp\): $env:NXRT_ROOT if set, else the nearest parent of this checkout with both a runtime\ and a work\ folder, else the
# folder that contains the checkout. $recomp is the recompiler's include folder: $env:NXRT_RECOMP_INCLUDE, else $proj\recomp\include.
$_checkout = Split-Path -Parent $PSScriptRoot
$proj = $env:NXRT_ROOT
if (-not $proj) {
    $d = $_checkout
    while ($d -and -not ((Test-Path (Join-Path $d "runtime")) -and (Test-Path (Join-Path $d "work")))) { $d = Split-Path -Parent $d }
    $proj = $d
}
if (-not $proj) { $proj = Split-Path -Parent $_checkout }
$recomp = if ($env:NXRT_RECOMP_INCLUDE) { $env:NXRT_RECOMP_INCLUDE } else { Join-Path $proj "recomp\include" }
