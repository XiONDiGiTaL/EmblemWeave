# Fetch the pinned third-party sources into tools\textures\third_party (git-ignored). Versions: THIRD_PARTY.md.
#
# Everything here is written to be safe to interrupt and safe to run twice:
#   * a download goes to `<name>.part`, is hashed, and only then is moved into place;
#   * a checkout goes to `<name>.partial` and is moved into place only once the
#     files the build needs are there;
#   * a folder that exists without those files is thrown away and fetched again,
#     so a build never fails on sources a half-finished run left behind;
#   * a lock file keeps two builds from fetching into the same tree at once.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$tp = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\third_party'))
New-Item -ItemType Directory -Force $tp | Out-Null
$attempts = 4

# --- one fetch at a time -----------------------------------------------------
$lockPath = Join-Path $tp '.fetch.lock'
$lock = $null
for ($i = 0; $i -lt 150; $i++) {
    try { $lock = [IO.File]::Open($lockPath, 'CreateNew', 'Write', 'None'); break }
    catch { Start-Sleep -Milliseconds 200 }
}
if (-not $lock) {
    throw "another build is fetching the texture sources (lock: $lockPath). Wait for it, or delete that file if no build is running."
}
try {
    function Get-PinnedFile([string]$Url, [string]$Dest, [string]$Sha256) {
        if (Test-Path $Dest) { return }
        $part = "$Dest.part"
        for ($n = 1; $n -le $script:attempts; $n++) {
            try {
                Remove-Item $part -Force -ErrorAction SilentlyContinue
                & curl.exe -sS -L --fail --retry 2 --retry-delay 3 -o $part $Url
                if ($LASTEXITCODE -ne 0) { throw "curl exited $LASTEXITCODE" }
                $hash = (Get-FileHash $part -Algorithm SHA256).Hash
                if ($hash -ne $Sha256) {
                    throw "sha256 of the download is $hash, and the pinned one is $Sha256"
                }
                Move-Item -Force $part $Dest
                Write-Host "downloaded $([IO.Path]::GetFileName($Dest)) (verified)"
                return
            } catch {
                Remove-Item $part -Force -ErrorAction SilentlyContinue
                if ($n -eq $script:attempts) {
                    throw "could not download $Url after $($script:attempts) attempts. Last error: $($_.Exception.Message)"
                }
                Write-Warning "download of $Url failed (attempt $n of $($script:attempts)): $($_.Exception.Message)"
                Start-Sleep -Seconds (2 * $n)
            }
        }
    }

    function Get-PinnedRepo([string]$Url, [string]$Rev, [string]$Dir, [string]$Required, [switch]$Tag) {
        if (Test-Path (Join-Path $Dir $Required)) { return }
        if (Test-Path $Dir) {
            Write-Host "$([IO.Path]::GetFileName($Dir)) is incomplete (no $Required); discarding it and fetching again"
            Remove-Item -Recurse -Force $Dir
        }
        $tmp = "$Dir.partial"
        for ($n = 1; $n -le $script:attempts; $n++) {
            try {
                Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
                $depth = if ($Tag) { @('--depth', '1', '--branch', $Rev) } else { @() }
                & git clone --quiet @depth $Url $tmp
                if ($LASTEXITCODE -ne 0) { throw "git clone exited $LASTEXITCODE" }
                if (-not $Tag) {
                    & git -C $tmp -c advice.detachedHead=false checkout --quiet $Rev
                    if ($LASTEXITCODE -ne 0) { throw "git checkout $Rev exited $LASTEXITCODE" }
                }
                if (-not (Test-Path (Join-Path $tmp $Required))) {
                    throw "the checkout has no $Required, so it is not the source this build needs"
                }
                Move-Item $tmp $Dir
                Write-Host "fetched $([IO.Path]::GetFileName($Dir)) at $Rev"
                return
            } catch {
                Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
                if ($n -eq $script:attempts) {
                    throw "could not fetch $Url at $Rev after $($script:attempts) attempts. Last error: $($_.Exception.Message)"
                }
                Write-Warning "fetch of $Url failed (attempt $n of $($script:attempts)): $($_.Exception.Message)"
                Start-Sleep -Seconds (2 * $n)
            }
        }
    }

    Get-PinnedRepo 'https://github.com/ARM-software/astc-encoder.git' '5.7.0' (Join-Path $tp 'astc-encoder') 'CMakeLists.txt' -Tag
    Get-PinnedRepo 'https://github.com/richgel999/bc7enc_rdo.git' 'b9438627eef73a1157e84201b6fa6eb2ffd6d9f0' (Join-Path $tp 'bc7enc_rdo') 'CMakeLists.txt'

    $ispcDir = Join-Path $tp 'ispc'
    $ispcExe = Join-Path $ispcDir 'ispc-v1.32.0-windows\bin\ispc.exe'
    if (-not (Test-Path $ispcExe)) {
        Remove-Item -Recurse -Force $ispcDir -ErrorAction SilentlyContinue
        $zip = Join-Path $tp 'ispc-v1.32.0-windows.zip'
        Get-PinnedFile 'https://github.com/ispc/ispc/releases/download/v1.32.0/ispc-v1.32.0-windows.zip' $zip '148D5CD96ABE7501C730D5FBD00F1F0C893B96BFEADA38D1811C9B6251E86A5C'
        $unpacked = "$ispcDir.partial"
        Remove-Item -Recurse -Force $unpacked -ErrorAction SilentlyContinue
        Expand-Archive $zip $unpacked
        if (-not (Test-Path (Join-Path $unpacked 'ispc-v1.32.0-windows\bin\ispc.exe'))) {
            Remove-Item -Recurse -Force $unpacked -ErrorAction SilentlyContinue
            throw "the ispc archive unpacked without ispc-v1.32.0-windows\bin\ispc.exe; run this again, and delete $zip if it repeats"
        }
        Move-Item $unpacked $ispcDir
        Write-Host 'unpacked ispc (verified)'
    }
    Write-Output "third_party ready in $tp"
} finally {
    if ($lock) { $lock.Dispose() }
    Remove-Item $lockPath -Force -ErrorAction SilentlyContinue
}
