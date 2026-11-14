# Configure + build astc2bc7 (library and CLI) with CMake + Ninja + clang-cl, in the environment of runtime\scripts\env.cmd.
#   pwsh -File tools\textures\scripts\build.ps1 [-Clean] [-Ispc] [-BuildDir DIR] [-Jobs N]
# Default: bc7e translated to C++ (src/bc7e_cpp.cpp), astcenc's decoder linked in. -Ispc: the original build, bc7e.ispc through the ISPC
# compiler plus the astcenc decoder DLLs (the runtime links that build from build\release when it exists, else it builds the C++ one itself).
# Needs third_party\ (scripts\fetch_third_party.ps1). Output: tools\textures\build\release\ unless -BuildDir.
param([switch]$Clean, [switch]$Ispc, [string]$BuildDir = "", [int]$Jobs = $(if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { 8 }))
$ErrorActionPreference = 'Stop'
$root  = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$build = if ($BuildDir) { [IO.Path]::GetFullPath($BuildDir) } else { Join-Path $root 'build\release' }
$env_cmd = [IO.Path]::GetFullPath((Join-Path $root '..\..\runtime\scripts\env.cmd'))
if ($Clean -and (Test-Path $build)) { Remove-Item -Recurse -Force $build }
$bc7e = if ($Ispc) { 'ispc' } else { 'cpp' }
cmd /c "call `"$env_cmd`" && cmake -S `"$root`" -B `"$build`" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl -DA2B_BC7E=$bc7e && cmake --build `"$build`" -j $Jobs"
if ($LASTEXITCODE) { throw 'build failed' }
Write-Output "built: $build\astc2bc7.exe ($bc7e)"
