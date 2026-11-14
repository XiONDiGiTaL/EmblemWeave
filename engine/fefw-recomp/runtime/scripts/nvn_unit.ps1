# Build and run ONE NVN unit test without CMake (several people can do this at once, each with its own output folder).
#   pwsh scripts\nvn_unit.ps1 -Name sampler -Test tests\gfx\test_nvn_sampler.cpp -Src src\gfx\nvn\api_sampler.cpp [-Src more.cpp ...] [-Group gfx_nvn_sampler]
# Always linked in: src\gfx\nvn\nvn_loader.cpp, nvn_abi.cpp, tests\test_main.cpp and build\release\nxrt.lib (the OS layer, built by the
# normal build; this script never touches build\release). Output: build\unit\<Name>\unit.exe.
param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Test, [string[]]$Src = @(), [string]$Group = "", [string[]]$Lib = @(), [string[]]$Inc = @(), [switch]$NoRun)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root "build\unit\$Name"
New-Item -ItemType Directory -Force $out | Out-Null
. (Join-Path $PSScriptRoot "proj.ps1")
$srcs = @($Test) + $Src + @("src\gfx\nvn\nvn_loader.cpp", "src\gfx\nvn\nvn_abi.cpp", "tests\test_main.cpp") | ForEach-Object { if ([System.IO.Path]::IsPathRooted($_)) { $_ } else { Join-Path $root $_ } }
$srcList = ($srcs | ForEach-Object { '"' + $_ + '"' }) -join " "
$lib = Join-Path $root "build\release\nxrt.lib"
$libs = ($Lib | ForEach-Object { '"' + $_ + '"' }) -join " "
$incs = ($Inc | ForEach-Object { '/I"' + $_ + '"' }) -join " "
$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$cl = "clang-cl /nologo /std:c++20 /O1 /EHsc /MD /W3 -Wno-unused-parameter -Wno-missing-field-initializers -Wno-cast-function-type-mismatch -Wno-cast-function-type-strict -Wno-language-extension-token " +
      "/DNOMINMAX /DWIN32_LEAN_AND_MEAN /D_CRT_SECURE_NO_WARNINGS /D_WIN32_WINNT=0x0A00 " +
      "/I`"$root\include`" /I`"$root\src\gfx`" /I`"$root\src\gfx\nvn`" /I`"$root\src\gfx\nvn\third_party`" /I`"$root\tests`" /I`"$root\build\release\_deps\volk-src`" /I`"$root\build\release\_deps\vulkan_headers-src\include`" /I`"$recomp`" $incs " +
      "$srcList /Fe`"$out\unit.exe`" /Fo`"$out/`" /link `"$nxrtlib`" $libs psapi.lib /STACK:8388608 /HIGHENTROPYVA:NO"
cmd /c "call `"$env_cmd`" && $cl"
if ($LASTEXITCODE -ne 0) { Write-Host "BUILD FAILED"; exit 1 }
if ($NoRun) { exit 0 }
$g = if ($Group) { $Group } else { "--all" }
& "$out\unit.exe" $g
exit $LASTEXITCODE
