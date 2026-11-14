# Builds mx_batch.exe (the batch Maxwell -> GLSL -> SPIR-V driver) with clang-cl, into tools\shaders\build\smo\.
# Run from the project folder:  pwsh -File tools\shaders\smo\build.ps1
# Needs: LLVM (C:\Program Files\LLVM\bin\clang-cl.exe) and the VS Build Tools (vcvars64.bat, for the Windows SDK and CRT).
$ErrorActionPreference = 'Stop'
$here  = Split-Path -Parent $MyInvocation.MyCommand.Path
$out   = Join-Path $here '..\build\smo' | ForEach-Object { [System.IO.Path]::GetFullPath($_) }
$obj   = Join-Path $out 'obj'
$clang = 'C:\Program Files\LLVM\bin\clang-cl.exe'
$vcvars = Join-Path (& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath) 'VC\Auxiliary\Build\vcvars64.bat'

if (-not (Test-Path $clang))   { throw "clang-cl not found at $clang" }
if (-not (Test-Path $vcvars))  { throw "vcvars64.bat not found at $vcvars" }

# import the VS developer environment (INCLUDE, LIB, PATH) into this session
$envdump = cmd /c "`"$vcvars`" >nul && set"
foreach ($line in $envdump) {
    $i = $line.IndexOf('=')
    if ($i -gt 0) { Set-Item -Path ("Env:" + $line.Substring(0, $i)) -Value $line.Substring($i + 1) }
}

New-Item -ItemType Directory -Force -Path $obj | Out-Null
$src = @((Join-Path $here 'mx_batch.c'), (Join-Path $here 'mx_translate.c'))
$exe = Join-Path $out 'mx_batch.exe'

# _CRT_SECURE_NO_WARNINGS: mx_translate.c uses getenv/strcpy, which MSVC's CRT marks deprecated; the code is unchanged.
$fe = '/Fe' + $exe
$fo = '/Fo' + $obj + '\'
& $clang /nologo /std:c11 /O2 /W3 /D_CRT_SECURE_NO_WARNINGS "/I$here" @src $fe $fo 2>&1 | ForEach-Object { $_ }
if ($LASTEXITCODE -ne 0) { throw "build failed (exit $LASTEXITCODE)" }
Write-Output "built: $exe"
