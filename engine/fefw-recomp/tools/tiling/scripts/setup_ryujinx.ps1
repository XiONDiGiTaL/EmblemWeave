# MIT License (this script). Copyright notice for the staged code: Ryujinx, MIT (see LICENSE.txt copied below).
# Stages the Ryujinx projects that the tiling reference needs into tools\tiling\third_party\ryubing (git-ignored).
# Source: the read-only checkout refs\ryubing. We build from this copy and never touch refs\.
# Dependency chain (from the csproj files): Ryujinx.Graphics.Texture -> Ryujinx.Common (no further ProjectReferences).
param([string]$Ryubing = (Join-Path $PSScriptRoot '..\..\..\refs\ryubing'))
$ErrorActionPreference = 'Stop'
$dst = Join-Path $PSScriptRoot '..\third_party\ryubing'
New-Item -ItemType Directory -Force "$dst\src" | Out-Null
foreach ($p in 'Ryujinx.Common','Ryujinx.Graphics.Texture') {
    robocopy "$Ryubing\src\$p" "$dst\src\$p" /MIR /XD obj bin /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy $p failed ($LASTEXITCODE)" }
}
foreach ($f in 'Directory.Build.props','Directory.Packages.props','global.json','nuget.config','LICENSE.txt') {
    Copy-Item "$Ryubing\$f" "$dst\$f" -Force
}
Write-Host "staged Ryujinx sources in $dst"
$global:LASTEXITCODE = 0
