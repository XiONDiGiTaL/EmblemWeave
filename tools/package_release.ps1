# Packs the player zip described in docs/PACKAGE.md: the launcher, the README, the package manifest and the engine
# sources (GPL-3.0-or-later), with the shader translator published alongside so no player needs the .NET SDK.
#
#   pwsh -File tools/package_release.ps1                 # version from Cargo.toml, builds what is missing
#   pwsh -File tools/package_release.ps1 -Version 0.1.12
#   pwsh -File tools/package_release.ps1 -SkipBuild      # reuse target\release\EmblemWeave.exe
#
# Writes target\pkg\EmblemWeave-v<version>-windows-x64.zip and prints its size and sha256.
param(
    [string]$Version = '',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    if (-not $Version) {
        $line = Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
        if (-not $line) { throw 'no version in Cargo.toml' }
        $Version = $line.Matches[0].Groups[1].Value
    }
    $name = "EmblemWeave-v$Version"
    $pkg = Join-Path $repo "target\pkg\$name"
    $zip = Join-Path $repo "target\pkg\$name-windows-x64.zip"
    Write-Host "packing $name"

    # 1. The launcher.
    $exe = Join-Path $repo 'target\release\EmblemWeave.exe'
    if (-not $SkipBuild) {
        cargo build --release -p emblemweave
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    if (-not (Test-Path $exe)) { throw "missing $exe (build it, or drop -SkipBuild)" }

    # 2. The shader translator, published once with its own .NET runtime. Kept out of git (engine .gitignore has *.exe).
    $translator = Join-Path $repo 'engine\fefw-recomp\tools\shaders\prebuilt\win-x64\spirv_harness.exe'
    if (-not (Test-Path $translator)) {
        dotnet publish 'engine\fefw-recomp\tools\shaders\harness\SpirvHarness.csproj' -c Release -r win-x64 `
            --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true `
            -p:EnableCompressionInSingleFile=true -p:DebugType=None `
            -o 'engine\fefw-recomp\tools\shaders\prebuilt\win-x64'
        if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed ($LASTEXITCODE)" }
    }
    if (-not (Test-Path $translator)) { throw "missing $translator" }

    # 3. The tree. Anything not listed here is not in the zip; docs/PACKAGE.md says why.
    if (Test-Path $pkg) { Remove-Item -Recurse -Force $pkg }
    New-Item -ItemType Directory -Force $pkg | Out-Null
    Copy-Item $exe (Join-Path $pkg 'EmblemWeave.exe')
    foreach ($f in 'LICENSE', 'README.md', 'docs\PACKAGE.md') { Copy-Item (Join-Path $repo $f) $pkg }

    # The dump, the build output, the fetched third-party sources and the engine's own unused launcher stay out.
    $xd = @(
        (Join-Path $repo 'engine\fefw-recomp\game'),
        (Join-Path $repo 'engine\fefw-recomp\work'),
        (Join-Path $repo 'engine\fefw-recomp\launcher'),
        'build', 'third_party', 'node_modules', 'shader_cache', 'bc7_cache', '.venv', 'refs', '.git', 'obj', 'bin'
    )
    $xf = @('*.obj', '*.pdb', '*.lib', '*.ilk', '*.exp', '*.map', '*.iobj', '*.ipdb')
    $dst = Join-Path $pkg 'engine\fefw-recomp'
    robocopy 'engine\fefw-recomp' $dst /E /NFL /NDL /NJH /NJS /NP /XD @xd /XF @xf | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE)" }

    # 4. The zip.
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $pkg -DestinationPath $zip -CompressionLevel Optimal
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "$zip ($mb MB)"
    Write-Host "sha256: $hash"
} finally {
    Pop-Location
}
