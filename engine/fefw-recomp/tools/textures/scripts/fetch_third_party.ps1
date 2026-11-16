# Fetch the pinned third-party sources into tools\textures\third_party (git-ignored). Versions: THIRD_PARTY.md.
# The two sources come down as release archives and are unpacked with Windows' own curl.exe and tar.exe, so a build needs
# no Git install; the archives are pinned by URL and checked against their SHA-256 before anything is unpacked.
$ErrorActionPreference = 'Stop'
$tp = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\third_party'))
New-Item -ItemType Directory -Force $tp | Out-Null

function Get-Source {
    param([string]$Url, [string]$Sha256, [string]$Name, [string]$Inner, [string]$Folder)
    if (Test-Path "$tp\$Folder") { return }
    $archive = Join-Path $env:TEMP "$Name.tar.gz"
    curl.exe -L --fail -o $archive $Url
    if ($LASTEXITCODE -ne 0) { throw "downloading $Url failed ($LASTEXITCODE)" }
    $h = (Get-FileHash $archive -Algorithm SHA256).Hash.ToLower()
    if ($h -ne $Sha256) { throw "$Name archive hash mismatch: $h" }
    $stage = Join-Path $tp "_unpack-$Name"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    tar.exe -xzf $archive -C $stage
    if ($LASTEXITCODE -ne 0) { throw "unpacking $Name failed ($LASTEXITCODE)" }
    Move-Item (Join-Path $stage $Inner) (Join-Path $tp $Folder)
    Remove-Item -Recurse -Force $stage
    Remove-Item -Force $archive
}

Get-Source -Url 'https://github.com/ARM-software/astc-encoder/archive/refs/tags/5.7.0.tar.gz' `
    -Sha256 '7c1b28ece59c9c2737e297123a9910c1c548676c7aed87a09d5734e2fa7bdaf0' `
    -Name 'astc-encoder-5.7.0' -Inner 'astc-encoder-5.7.0' -Folder 'astc-encoder'

Get-Source -Url 'https://github.com/richgel999/bc7enc_rdo/archive/b9438627eef73a1157e84201b6fa6eb2ffd6d9f0.tar.gz' `
    -Sha256 'bbb33d1dbb6178a3a2c044956c3cc0b8ea9fc2f2903824379dd86f87d53b64c6' `
    -Name 'bc7enc_rdo-b9438627' -Inner 'bc7enc_rdo-b9438627eef73a1157e84201b6fa6eb2ffd6d9f0' -Folder 'bc7enc_rdo'

if (-not (Test-Path "$tp\ispc\ispc-v1.32.0-windows\bin\ispc.exe")) {
    $zip = Join-Path $env:TEMP 'ispc-v1.32.0-windows.zip'
    curl.exe -L --fail -o $zip https://github.com/ispc/ispc/releases/download/v1.32.0/ispc-v1.32.0-windows.zip
    if ($LASTEXITCODE -ne 0) { throw "downloading ispc failed ($LASTEXITCODE)" }
    $h = (Get-FileHash $zip -Algorithm SHA256).Hash
    if ($h -ne '148D5CD96ABE7501C730D5FBD00F1F0C893B96BFEADA38D1811C9B6251E86A5C') { throw "ispc zip hash mismatch: $h" }
    Expand-Archive $zip "$tp\ispc"
}
Write-Output "third_party ready in $tp"
