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
    # The README links the changelog, so it travels with it.
    New-Item -ItemType Directory -Force (Join-Path $pkg 'docs') | Out-Null
    Copy-Item (Join-Path $repo 'docs\changelog') (Join-Path $pkg 'docs\changelog') -Recurse

    # The dump, the build output, the fetched third-party sources and the engine's own unused launcher stay out.
    $xd = @(
        (Join-Path $repo 'engine\fefw-recomp\game'),
        (Join-Path $repo 'engine\fefw-recomp\work'),
        (Join-Path $repo 'engine\fefw-recomp\launcher'),
        # The Linux translator ships as its own tarball: a Windows player cannot run 41 MB of ELF.
        (Join-Path $repo 'engine\fefw-recomp\tools\shaders\prebuilt\linux-x64'),
        'build', 'third_party', 'node_modules', 'shader_cache', 'bc7_cache', '.venv', 'refs', '.git', 'obj', 'bin',
        '.github', '.vscode', '.pytest_cache', '.mypy_cache', '__pycache__', '*.egg-info'
    )
    $xf = @('*.obj', '*.pdb', '*.lib', '*.ilk', '*.exp', '*.map', '*.iobj', '*.ipdb', '*.pyc', '*.pyo')
    $dst = Join-Path $pkg 'engine\fefw-recomp'
    robocopy 'engine\fefw-recomp' $dst /E /NFL /NDL /NJH /NJS /NP /XD @xd /XF @xf | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE)" }

    # 4. The zip.
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $pkg -DestinationPath $zip -CompressionLevel Optimal

    # Every entry must be a relative path with `/` separators. Windows' own zippers can write `\`, and an archive full of
    # `engine\recomp\foo.cpp` extracts as a flat pile of oddly named files on Linux and macOS.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $bad = @()
    $za = [System.IO.Compression.ZipFile]::OpenRead($zip)
    try {
        foreach ($e in $za.Entries) {
            if ($e.FullName.Contains('\') -or $e.FullName.StartsWith('/') -or $e.FullName -match '^[A-Za-z]:') { $bad += $e.FullName }
        }
    } finally { $za.Dispose() }
    if ($bad.Count -gt 0) { throw "$($bad.Count) zip entries are not plain forward-slash relative paths, e.g. $($bad[0])" }

    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "$zip ($mb MB)"
    Write-Host "sha256: $hash"

    # 5. The Linux translator, published from the same sources, as a release asset of its own. The archive keeps the
    # executable bit when it is made on Linux; the notes say `chmod +x` for archives built elsewhere.
    $linuxDir = Join-Path $repo 'engine\fefw-recomp\tools\shaders\prebuilt\linux-x64'
    if (Test-Path (Join-Path $linuxDir 'spirv_harness')) {
        $tar = Join-Path (Split-Path $zip) "$name-shader-translator-linux-x64.tar.gz"
        if (Test-Path $tar) { Remove-Item -Force $tar }
        # Run from the target folder and name the archive bare: GNU tar reads `C:\...` as a remote host otherwise.
        Push-Location (Split-Path $zip)
        try {
            # Windows has no executable bit to record, so stamp one in: `--mode` is GNU tar, and bsdtar (which lacks it)
            # gets a plain archive with the `chmod +x` note in PACKAGE.md.
            tar --mode=u+rwx,go+rx -czf (Split-Path $tar -Leaf) -C $linuxDir '.'
            if ($LASTEXITCODE -ne 0) { tar -czf (Split-Path $tar -Leaf) -C $linuxDir '.' }
            if ($LASTEXITCODE -ne 0) { throw "tar failed ($LASTEXITCODE)" }
        }
        finally { Pop-Location }
        $tmb = [math]::Round((Get-Item $tar).Length / 1MB, 1)
        Write-Host "$tar ($tmb MB)"
        Write-Host "sha256: $((Get-FileHash $tar -Algorithm SHA256).Hash.ToLower())"
    } else {
        Write-Host "no linux-x64 translator published; skipping its tarball"
    }
} finally {
    Pop-Location
}
