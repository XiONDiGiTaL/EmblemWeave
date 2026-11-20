# Packs the player zip described in docs/PACKAGE.md: the launcher, the README, the package manifest and the engine
# sources (GPL-3.0-or-later), with the shader translator published alongside so no player needs the .NET SDK.
#
#   pwsh -File tools/package_release.ps1                 # version from Cargo.toml, builds what is missing
#   pwsh -File tools/package_release.ps1 -Version 0.1.12
#   pwsh -File tools/package_release.ps1 -SkipBuild      # reuse target\release\EmblemWeave.exe
#   pwsh -File tools/package_release.ps1 -SkipConfigureCheck  # do not configure the extracted zip
#
# Writes target\pkg\EmblemWeave-v<version>-windows-x64.zip and prints its size and sha256.
param(
    [string]$Version = '',
    [switch]$SkipBuild,
    [switch]$SkipConfigureCheck
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
    # The site, README, changelog index and release.json are edited by hand. Check them before building anything, so a
    # release cannot be packed with a download button or an updater pointer that still names the previous version.
    $pyCheck = Get-Command python -ErrorAction SilentlyContinue
    if (-not $pyCheck) { $pyCheck = Get-Command py -ErrorAction SilentlyContinue }
    if (-not $pyCheck) { throw 'python is needed to check the release metadata (tools/check_release.py)' }
    & $pyCheck.Source 'tools/check_release.py' --version $Version
    if ($LASTEXITCODE -ne 0) { throw "release metadata does not agree on $Version; fix it before packing" }
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
    # A fresh download is portable without anyone finding the About page: every
    # path, save and build cache lives in `data` beside this file. An existing
    # profile is left alone by the launcher's own first-run check.
    New-Item -ItemType File -Force (Join-Path $pkg 'portable.txt') | Out-Null
    foreach ($f in 'LICENSE', 'README.md') { Copy-Item (Join-Path $repo $f) $pkg }
    # The README links docs/PACKAGE.md and the changelog, so both travel at the paths it names. Copying
    # `docs\PACKAGE.md` into the package root used to leave it as `PACKAGE.md`, where nothing looks for it;
    # tools/verify_archive.py is what noticed.
    New-Item -ItemType Directory -Force (Join-Path $pkg 'docs') | Out-Null
    # Every page the README links, not only PACKAGE: the new guide pages would
    # otherwise work on GitHub and 404 inside the downloaded zip.
    foreach ($doc in (Get-ChildItem (Join-Path $repo 'docs') -Filter '*.md' -File)) {
        Copy-Item $doc.FullName (Join-Path (Join-Path $pkg 'docs') $doc.Name)
    }
    Copy-Item (Join-Path $repo 'docs\changelog') (Join-Path $pkg 'docs\changelog') -Recurse
    Copy-Item (Join-Path $repo 'docs\screenshots') (Join-Path $pkg 'docs\screenshots') -Recurse

    # The dump, the build output and the fetched third-party sources stay out.
    $xd = @(
        (Join-Path $repo 'engine\fefw-recomp\game'),
        (Join-Path $repo 'engine\fefw-recomp\work'),
        # The Linux translator ships as its own tarball: a Windows player cannot run 41 MB of ELF.
        (Join-Path $repo 'engine\fefw-recomp\tools\shaders\prebuilt\linux-x64'),
        # The bare name `third_party` stripped `runtime/third_party` too (dav1d, FSR 1, NIS, Streamline, the
        # ffx API): every build from a zip then died at `add_subdirectory .../runtime/cmake/../third_party/dav1d`
        # (issues #16 and #17). The two fetched trees are named instead, and the runtime's own is shipped.
        (Join-Path $repo 'engine\fefw-recomp\tools\textures\third_party'),
        (Join-Path $repo 'engine\fefw-recomp\tools\shaders\third_party'),
        # The fetched toolchain archives (LLVM alone is ~900 MB) are inputs to a local build, not runtime files. The
        # launcher carries the extracted tools in tools\bin; shipping the downloads would double the zip for no player.
        (Join-Path $repo 'engine\fefw-recomp\tools\download'),
        'build', 'node_modules', 'shader_cache', 'bc7_cache', '.venv', 'refs', '.git', 'obj', 'bin',
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
    # Every entry sits inside `EmblemWeave-v<version>/`: the folder every release zip has extracted to since
    # 0.1.5. Dropping it would scatter the tree into whatever folder a player unpacks beside an older one.
    $prefix = "$name/"
    $nFiles = 0
    $za = [System.IO.Compression.ZipFile]::Open($zip, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        # The wrapper itself gets an entry too, so an extractor creates it in order.
        $e = $za.CreateEntry($prefix, [System.IO.Compression.CompressionLevel]::NoCompression)
        $e.LastWriteTime = Get-Item -LiteralPath $pkg | Select-Object -ExpandProperty LastWriteTime
        foreach ($d in (Get-ChildItem -LiteralPath $pkg -Recurse -Directory | Sort-Object FullName)) {
            $rel = $prefix + $d.FullName.Substring($rootLen).Replace('\', '/') + '/'
            $e = $za.CreateEntry($rel, [System.IO.Compression.CompressionLevel]::NoCompression)
            $e.LastWriteTime = $d.LastWriteTime
        }
        foreach ($f in (Get-ChildItem -LiteralPath $pkg -Recurse -File | Sort-Object FullName)) {
            $rel = $prefix + $f.FullName.Substring($rootLen).Replace('\', '/')
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
        'EmblemWeave.exe', 'portable.txt', 'README.md', 'LICENSE', 'docs/PACKAGE.md', 'docs/INSTALL.md',
        'docs/USAGE.md', 'docs/BUILDING.md', 'docs/SECURITY.md', 'docs/TERMS.md', 'docs/PRIVACY.md',
        'docs/LEGAL.md', 'docs/DMCA.md',
        'docs/screenshots/overview.png', 'docs/changelog/README.md',
        'engine/fefw-recomp/runtime/CMakeLists.txt', 'engine/fefw-recomp/game-build/fefw/CMakeLists.txt',
        'engine/fefw-recomp/recomp/requirements.txt', 'engine/fefw-recomp/tools/shaders/harness/Program.cs',
        # The CMake configure reads this before it builds anything; a zip without it failed every build (issues #16, #17).
        'engine/fefw-recomp/runtime/third_party/dav1d/CMakeLists.txt',
        # The optional FFmpeg movie decoder compiles against these; the licence must travel with them.
        'engine/fefw-recomp/runtime/third_party/ffmpeg/include/libavcodec/avcodec.h',
        'engine/fefw-recomp/runtime/third_party/ffmpeg/COPYING.LGPLv2.1',
        'engine/fefw-recomp/tools/shaders/ampere/extract_corpus.py',
        'engine/fefw-recomp/tools/shaders/prebuilt/win-x64/spirv_harness.exe'
    )
    $unpacked = Join-Path $repo 'target\pkg\verify-unpacked'
    if (Test-Path $unpacked) { Remove-Item -Recurse -Force $unpacked }
    $verify = @('tools/verify_archive.py', $zip, '--extract', $unpacked)
    foreach ($r in $required) { $verify += @('--require', "$name/$r") }
    & $py.Source @verify
    if ($LASTEXITCODE -ne 0) { throw "the zip failed verification: $nFiles files written, tools/verify_archive.py is unhappy" }
    $unpackedCount = (Get-ChildItem -LiteralPath $unpacked -Recurse -File).Count
    if ($unpackedCount -ne $nFiles) { throw "the zip holds $nFiles files but extracted $unpackedCount" }
    Write-Host "zip verified: $nFiles files, extracted and compared in $unpacked"

    # 6. The zip is configured, not only listed. The engine's CMake configure is the first thing that a missing source
    # tree or a wrong path kills, and it is what issues #16 and #17 hit: dav1d was not in the download, and every
    # build from a zip stopped at `add_subdirectory ... third_party/dav1d`. Configure only: no lifted libraries, no
    # game data and no compile, just the step that proves the tree the player gets is the tree the build reads.
    # Skipped with its reason on a machine that has no Visual Studio C++ tools and LLVM to configure with, because
    # that says nothing about the zip; -SkipConfigureCheck turns it off deliberately.
    if ($SkipConfigureCheck) {
        Write-Host 'configure check: skipped (-SkipConfigureCheck)'
    } else {
        $engine = Join-Path $unpacked "$name\engine\fefw-recomp"
        $src = Join-Path $engine 'game-build\fefw'
        $envCmd = Join-Path $engine 'runtime\scripts\env.cmd'
        $bd = Join-Path $unpacked "$name\configure-check"
        $checkLog = Join-Path $repo 'target\pkg\configure-check.log'
        $envDump = & cmd /c "call `"$envCmd`" >nul 2>&1 && set" 2>$null
        $envOk = $LASTEXITCODE -eq 0
        $llvm = @($envDump | Where-Object { $_ -like 'NX_LLVM_BIN=*' })
        $llvm = if ($llvm.Count -gt 0) { $llvm[0].Substring('NX_LLVM_BIN='.Length) } else { '' }
        if (-not $envOk -or -not $llvm -or -not (Test-Path (Join-Path $llvm 'clang-cl.exe'))) {
            Write-Warning 'configure check skipped: runtime\scripts\env.cmd found no Visual Studio C++ tools and LLVM on this machine (that says nothing about the zip)'
        } else {
            # One child shell, so the configure sees the same INCLUDE, LIB and clang-cl the engine's own build script gives it.
            $clang = (Join-Path $llvm 'clang-cl.exe').Replace('\', '/')
            $cfg = "call `"$envCmd`" >nul 2>&1 && cmake -S `"$src`" -B `"$bd`" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=`"$clang`" -DCMAKE_CXX_COMPILER=`"$clang`""
            # CMake writes warnings to stderr; Windows PowerShell 5.1 turns a
            # native command's stderr into an error record, and $ErrorActionPreference
            # is Stop here. The exit code is the result, not the warning text.
            $oldErrorAction = $ErrorActionPreference
            $ErrorActionPreference = 'Continue'
            & cmd /c $cfg *> $checkLog
            $configureExit = $LASTEXITCODE
            $ErrorActionPreference = $oldErrorAction
            if ($configureExit -ne 0) {
                $tail = @(Get-Content -LiteralPath $checkLog -Tail 30)
                Write-Host ($tail -join [Environment]::NewLine)
                throw "the zip does not configure: $bd (log: $checkLog). A source tree the build reads is missing from the package, or a path in it is wrong; do not publish this zip."
            }
            Write-Host "configure check: the extracted zip configures (engine CMake, log: $checkLog)"
            Remove-Item -Recurse -Force $bd -ErrorAction SilentlyContinue
        }
    }

    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    $mb = [math]::Round((Get-Item $zip).Length / 1MB, 1)
    Write-Host "$zip ($mb MB)"
    Write-Host "sha256: $hash"

    # 5. What was built, for the release script: names and hashes, so the notes and the upload cannot disagree. The
    # Linux translator is a separate job on an Ubuntu runner (`.github/workflows/release.yml`): it is the only place
    # that can write the executable bit without lying about it, and a Windows package run must not pretend to.
    $manifest = [ordered]@{
        version = $Version
        zip     = [System.IO.Path]::GetFileName($zip)
        zipSha  = $hash
        zipSize = (Get-Item $zip).Length
    }
    $manifest | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path (Split-Path $zip) 'manifest.json')
    Write-Host "manifest: $(Join-Path (Split-Path $zip) 'manifest.json')"
} finally {
    Pop-Location
}
