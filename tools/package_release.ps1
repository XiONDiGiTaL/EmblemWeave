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
    foreach ($f in 'LICENSE', 'README.md') { Copy-Item (Join-Path $repo $f) $pkg }
    # The README links docs/PACKAGE.md and the changelog, so both travel at the paths it names. Copying
    # `docs\PACKAGE.md` into the package root used to leave it as `PACKAGE.md`, where nothing looks for it;
    # tools/verify_archive.py is what noticed.
    New-Item -ItemType Directory -Force (Join-Path $pkg 'docs') | Out-Null
    Copy-Item (Join-Path $repo 'docs\PACKAGE.md') (Join-Path $pkg 'docs\PACKAGE.md')
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

    # 4. The zip, written entry by entry. `Compress-Archive` decides its own separators, and Windows' own zippers have
    # written `engine\recomp\foo.cpp` for years: an archive of those unpacks on Linux and macOS as a flat pile of files
    # whose names contain backslashes. Every name here is relative, made from the package tree, with `/` separators;
    # folders get their own entries so an empty one survives and an extractor can create the tree in order.
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $rootLen = $pkg.TrimEnd('\').Length + 1
    $nFiles = 0
    $za = [System.IO.Compression.ZipFile]::Open($zip, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($d in (Get-ChildItem -LiteralPath $pkg -Recurse -Directory | Sort-Object FullName)) {
            $rel = $d.FullName.Substring($rootLen).Replace('\', '/') + '/'
            $e = $za.CreateEntry($rel, [System.IO.Compression.CompressionLevel]::NoCompression)
            $e.LastWriteTime = $d.LastWriteTime
        }
        foreach ($f in (Get-ChildItem -LiteralPath $pkg -Recurse -File | Sort-Object FullName)) {
            $rel = $f.FullName.Substring($rootLen).Replace('\', '/')
            $e = $za.CreateEntry($rel, [System.IO.Compression.CompressionLevel]::Optimal)
            $e.LastWriteTime = $f.LastWriteTime
            $out = $e.Open()
            try {
                $in = [System.IO.File]::OpenRead($f.FullName)
                try { $in.CopyTo($out) } finally { $in.Dispose() }
            } finally { $out.Dispose() }
            $nFiles++
        }
    } finally { $za.Dispose() }

    # 5. The archive is checked, not assumed: plain relative `/` paths, no `..`, no drive letters, no two names that
    # differ only in case, every file the launcher needs at the path it looks for, and a round-trip extraction whose
    # sizes match. tools/verify_archive.py is the same check a Linux release step runs, so both agree on what "ok" is.
    $py = Get-Command python -ErrorAction SilentlyContinue
    if (-not $py) { $py = Get-Command py -ErrorAction SilentlyContinue }
    if (-not $py) { throw 'python is needed to verify the release archive (tools/verify_archive.py)' }
    $required = @(
        'EmblemWeave.exe', 'README.md', 'LICENSE', 'docs/PACKAGE.md', 'docs/changelog/README.md',
        'engine/fefw-recomp/runtime/CMakeLists.txt', 'engine/fefw-recomp/game-build/fefw/CMakeLists.txt',
        'engine/fefw-recomp/recomp/requirements.txt', 'engine/fefw-recomp/tools/shaders/harness/Program.cs',
        'engine/fefw-recomp/tools/shaders/ampere/extract_corpus.py',
        'engine/fefw-recomp/tools/shaders/prebuilt/win-x64/spirv_harness.exe'
    )
    $unpacked = Join-Path $repo 'target\pkg\verify-unpacked'
    if (Test-Path $unpacked) { Remove-Item -Recurse -Force $unpacked }
    $verify = @('tools/verify_archive.py', $zip, '--extract', $unpacked)
    foreach ($r in $required) { $verify += @('--require', $r) }
    & $py.Source @verify
    if ($LASTEXITCODE -ne 0) { throw "the zip failed verification: $nFiles files written, tools/verify_archive.py is unhappy" }
    $unpackedCount = (Get-ChildItem -LiteralPath $unpacked -Recurse -File).Count
    if ($unpackedCount -ne $nFiles) { throw "the zip holds $nFiles files but extracted $unpackedCount" }
    Write-Host "zip verified: $nFiles files, extracted and compared in $unpacked"

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
        # The files are named one by one rather than `-C dir .`, which would write `./spirv_harness`: every entry is a
        # plain relative path, the same rule the zip follows, so one spelling extracts everywhere.
        $items = @(Get-ChildItem -LiteralPath $linuxDir -Force | ForEach-Object { $_.Name })
        Push-Location (Split-Path $zip)
        try {
            # Windows has no executable bit to record, so stamp one in: `--mode` is GNU tar, and bsdtar (which lacks it)
            # gets a plain archive with the `chmod +x` note in PACKAGE.md.
            tar --mode=u+rwx,go+rx -czf (Split-Path $tar -Leaf) -C $linuxDir @items
            if ($LASTEXITCODE -ne 0) { tar -czf (Split-Path $tar -Leaf) -C $linuxDir @items }
            if ($LASTEXITCODE -ne 0) { throw "tar failed ($LASTEXITCODE)" }
        }
        finally { Pop-Location }
        # The tarball gets the same check: relative `/` names, no `..`, the translator at the root and executable (the
        # bit a Windows-built tar has to be given, since NTFS has none to record).
        & $py.Source @('tools/verify_archive.py', $tar, '--require', 'spirv_harness')
        if ($LASTEXITCODE -ne 0) { throw 'the translator tarball failed verification (tools/verify_archive.py)' }
        $tmb = [math]::Round((Get-Item $tar).Length / 1MB, 1)
        Write-Host "$tar ($tmb MB), verified"
        Write-Host "sha256: $((Get-FileHash $tar -Algorithm SHA256).Hash.ToLower())"
    } else {
        Write-Host "no linux-x64 translator published; skipping its tarball"
    }

    # 7. What was built, for the release script: names and hashes, so the notes and the upload cannot disagree.
    $tarPath = Join-Path (Split-Path $zip) "$name-shader-translator-linux-x64.tar.gz"
    $manifest = [ordered]@{
        version = $Version
        zip     = [System.IO.Path]::GetFileName($zip)
        zipSha  = $hash
        zipSize = (Get-Item $zip).Length
        tar     = if (Test-Path $tarPath) { [System.IO.Path]::GetFileName($tarPath) } else { $null }
        tarSha  = if (Test-Path $tarPath) { (Get-FileHash $tarPath -Algorithm SHA256).Hash.ToLower() } else { $null }
        tarSize = if (Test-Path $tarPath) { (Get-Item $tarPath).Length } else { $null }
    }
    $manifest | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path (Split-Path $zip) 'manifest.json')
    Write-Host "manifest: $(Join-Path (Split-Path $zip) 'manifest.json')"
} finally {
    Pop-Location
}
