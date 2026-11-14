# Build the nxrt_tests target of this checkout and run the named test groups directly (no CTest, no regex quoting problems).
#   pwsh scripts\t.ps1 svc_fs svc_time          build, then run those groups
#   pwsh scripts\t.ps1 -All                     build, then run nxrt_tests --all
# Exit code = number of failed groups (or the build's exit code).
[CmdletBinding(PositionalBinding = $false)]
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Groups = @(), [switch]$All, [switch]$NoBuild, [string]$Config = "Release")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$bd = Join-Path $root ("build\" + $Config.ToLower())
if (-not $NoBuild) {
    cmd /c "call `"$(Join-Path $PSScriptRoot 'env.cmd')`" && cmake --build `"$bd`" --target nxrt_tests"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
$exe = Join-Path $bd "nxrt_tests.exe"
Push-Location $bd
$failed = 0
if ($All) { & $exe --all; if ($LASTEXITCODE -ne 0) { $failed++ } }
foreach ($g in $Groups) { & $exe $g; if ($LASTEXITCODE -ne 0) { $failed++ } }
Pop-Location
exit $failed
