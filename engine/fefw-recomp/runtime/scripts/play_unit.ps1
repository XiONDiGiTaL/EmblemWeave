# Build and run ONE play-mode unit test without CMake (several people can do this at once, each with its own output folder).
#   pwsh scripts\play_unit.ps1 -Name config -Test tests\gfx\test_play_config.cpp -Src src\gfx\play\play_types.cpp [-Src more.cpp ...] [-Group play_config]
# Always linked in: tests\test_main.cpp, SDL3 (static, only the scancode name tables are used), the OS layer nxrt.lib and, with -Imgui, ImGui.
# The libraries come from the game-build\build\play tree (build it first with boot_play.sh (local test script) or game-build\scripts\build.ps1); override with
# -Build <dir>. Output: build\unit\<Name>\unit.exe. This script never touches the main build directories.
param([Parameter(Mandatory)][string]$Name, [Parameter(Mandatory)][string]$Test, [string[]]$Src = @(), [string]$Group = "", [string[]]$Lib = @(), [string[]]$Inc = @(), [string]$Build = "", [switch]$NoRun)
$ErrorActionPreference = "Stop"
$Src = @($Src | ForEach-Object { $_ -split "," } | Where-Object { $_ })   # -File passes a comma list as one string
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root "build\unit\$Name"
New-Item -ItemType Directory -Force $out | Out-Null
. (Join-Path $PSScriptRoot "proj.ps1")
if (-not $Build) { $Build = Join-Path $proj "game-build\build\play" }
$srcs = @($Test) + $Src + @("tests\test_main.cpp") | ForEach-Object { if ([System.IO.Path]::IsPathRooted($_)) { $_ } else { Join-Path $root $_ } }
$srcList = ($srcs | ForEach-Object { '"' + $_ + '"' }) -join " "
$libs = ($Lib | ForEach-Object { '"' + $_ + '"' }) -join " "
$incs = ($Inc | ForEach-Object { '/I"' + $_ + '"' }) -join " "
$sdlinc = Join-Path $Build "_deps\sdl3-src\include"
$imguiinc = Join-Path $Build "_deps\imgui-src"
$vk = Join-Path $Build "_deps\vulkan_headers-src\include"
$volk = Join-Path $Build "_deps\volk-src"
$sdllib = Join-Path $Build "_deps\sdl3-build\SDL3-static.lib"
$nxrtlib = Join-Path $Build "runtime\nxrt.lib"
$sys = "winmm.lib imm32.lib setupapi.lib version.lib user32.lib gdi32.lib shell32.lib ole32.lib oleaut32.lib advapi32.lib cfgmgr32.lib psapi.lib dbghelp.lib"
$env_cmd = Join-Path $PSScriptRoot "env.cmd"
$cl = "clang-cl /nologo /std:c++20 /O1 /EHsc /MD /W3 -Wno-unused-parameter -Wno-missing-field-initializers -Wno-cast-function-type-mismatch -Wno-cast-function-type-strict -Wno-language-extension-token " +
      "/DNOMINMAX /DWIN32_LEAN_AND_MEAN /D_CRT_SECURE_NO_WARNINGS /D_WIN32_WINNT=0x0A00 " +
      "/I`"$root\include`" /I`"$root\src\gfx`" /I`"$root\tests`" /I`"$sdlinc`" /I`"$imguiinc`" /I`"$vk`" /I`"$volk`" /I`"$recomp`" $incs " +
      "$srcList /Fe`"$out\unit.exe`" /Fo`"$out/`" /link `"$nxrtlib`" `"$sdllib`" $libs $sys /STACK:8388608 /HIGHENTROPYVA:NO"
cmd /c "call `"$env_cmd`" && $cl"
if ($LASTEXITCODE -ne 0) { Write-Host "BUILD FAILED"; exit 1 }
if ($NoRun) { exit 0 }
$g = if ($Group) { $Group } else { "--all" }
& "$out\unit.exe" $g
exit $LASTEXITCODE
