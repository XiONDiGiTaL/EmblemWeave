# Stage the three Ryujinx projects the translator needs into tools\shaders\third_party\ryubing (git-ignored).
# Source: the read-only checkout refs\ryubing (Ryubing fork, Codeberg archive a82350bb77, MIT). The build writes obj/bin
# next to the sources, so we build from this copy and never touch refs\.
#
# The copy is staged into `ryubing.partial` and moved into place only once every file the .NET build reads is there, so an
# interrupted run cannot leave a tree the next build trusts and then fails on.
param([string]$Ryubing = (Join-Path $PSScriptRoot '..\..\..\refs\ryubing'))
$ErrorActionPreference = 'Stop'
$dst = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\third_party\ryubing'))
$staging = "$dst.partial"

$projects = 'Ryujinx.Common', 'Spv.Generator', 'Ryujinx.Graphics.Shader'
$files = 'Directory.Build.props', 'Directory.Packages.props', 'global.json', 'nuget.config', 'LICENSE.txt'

if (-not (Test-Path $Ryubing)) {
    throw "the Ryujinx checkout is not at $Ryubing. The launcher clones it at its pinned revision; if that clone was interrupted, delete work\refs\ryubing and build the game again."
}
$missing = @($projects | Where-Object { -not (Test-Path (Join-Path $Ryubing "src\$_")) })
if ($missing) {
    throw "the checkout at $Ryubing has no $($missing -join ', ') under src. It is an incomplete clone: delete work\refs\ryubing and build the game again."
}

Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$staging\src" | Out-Null
foreach ($p in $projects) {
    robocopy "$Ryubing\src\$p" "$staging\src\$p" /MIR /XD obj bin /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue; throw "robocopy $p failed ($LASTEXITCODE)" }
}
foreach ($f in $files) {
    if (-not (Test-Path (Join-Path $Ryubing $f))) {
        Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
        throw "$f is missing from $Ryubing; the checkout is incomplete"
    }
    Copy-Item (Join-Path $Ryubing $f) (Join-Path $staging $f) -Force
}

# The .NET build needs a project file in each staged folder; a robocopy that stopped early leaves folders without one.
$proj = @(Get-ChildItem -Recurse -Filter *.csproj "$staging\src" -ErrorAction SilentlyContinue)
if ($proj.Count -lt 3) {
    Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
    throw "staging copied $($proj.Count) project files, and at least 3 are needed. Run the build again; delete $dst if it repeats."
}

if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
Move-Item $staging $dst
Write-Host "staged Ryujinx sources in $dst"
$global:LASTEXITCODE = 0
