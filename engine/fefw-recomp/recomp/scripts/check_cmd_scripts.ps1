#Requires -Version 5.1
<#
.SYNOPSIS
    Fails when a cmd parse error, or cmd's line limit, replaces the expected answer of
    build_game.cmd or nx_env.cmd.

.DESCRIPTION
    cmd parses a parenthesized if (...) block whole, even when its condition is false, so one
    unbalanced ) or an unescaped line continuation inside such a block breaks every call to the
    script, not just that branch. cmd also expands a line before running it and refuses one over
    8191 characters, which a long PATH reached inside vcvars64.bat. Neither script needs a build
    to show either: build_game.cmd answers an unknown game with "no game folder", and nx_env.cmd
    answers the removed --structured-events flag with its removal message. This runs those calls,
    and the same two scripts with a PATH near cmd's limit (through the launcher handshake and as
    a direct call), and fails on a cmd parse or line-limit error, a missing message, or a wrong
    exit code. The direct call runs vcvars64.bat, so it needs Visual Studio's C++ tools and
    reports SKIP without them.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File engine\fefw-recomp\recomp\scripts\check_cmd_scripts.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$scripts = Split-Path -Parent $MyInvocation.MyCommand.Path
$cmdErrors = @('was unexpected at this time', 'syntax of the command is incorrect', 'The input line is too long')
$cmdExe = Join-Path $env:SystemRoot 'System32\cmd.exe'
$failed = 0

function Test-CmdScript {
    param(
        [string]$Script,
        [string]$Arguments,
        [string]$Expected,
        [int]$ExpectedExit = 2,
        # When set, the call runs with this PATH instead of the inherited one. The launcher
        # runs the build with a PATH of its own, and build_game.cmd can be started from a
        # shell whose PATH is most of cmd's 8191-character line limit on its own.
        [string]$Path,
        # Set as engine.rs does for the build it starts: nx_env.cmd then keeps the environment
        # the launcher prepared and must not run vcvars64.bat again.
        [switch]$Launcher
    )

    $label = "$Script $Arguments"
    if ($Path) {
        $label = "$label (PATH $($Path.Length) chars, $(if ($Launcher) { 'NX_ENV_READY' } else { 'direct' }))"
    }

    $oldPath = $env:PATH
    $oldReady = $env:NX_ENV_READY
    Push-Location $scripts
    try {
        if ($Path) { $env:PATH = $Path }
        if ($Launcher) { $env:NX_ENV_READY = '1' } elseif ($Path) { Remove-Item Env:\NX_ENV_READY -ErrorAction SilentlyContinue }
        # 2>&1 goes to cmd, not to PowerShell: a redirected native stderr line becomes an
        # ErrorRecord, and with $ErrorActionPreference = 'Stop' that ends the run before the
        # failure can be reported.
        $output = (& $cmdExe /c ".\$Script $Arguments 2>&1" | Out-String)
        $code = $LASTEXITCODE
    } finally {
        $env:PATH = $oldPath
        if ($null -eq $oldReady) { Remove-Item Env:\NX_ENV_READY -ErrorAction SilentlyContinue } else { $env:NX_ENV_READY = $oldReady }
        Pop-Location
    }

    $text = $output.Trim()
    foreach ($pattern in $cmdErrors) {
        if ($text -like "*$pattern*") {
            Write-Host "FAIL $label"
            Write-Host "     cmd refused the line: $text"
            return $false
        }
    }
    if ($Expected -and $text -notlike "*$Expected*") {
        Write-Host "FAIL $label"
        Write-Host "     expected '$Expected', got: $text"
        return $false
    }
    if ($code -ne $ExpectedExit) {
        Write-Host "FAIL $label"
        Write-Host "     expected exit $ExpectedExit, got $code"
        return $false
    }
    Write-Host "PASS $label"
    return $true
}

if (-not (Test-CmdScript -Script 'build_game.cmd' -Arguments '__missing' -Expected 'no game folder')) { $failed++ }
if (-not (Test-CmdScript -Script 'nx_env.cmd' -Arguments '--structured-events' -Expected 'structured-events was removed')) { $failed++ }

# A PATH just under cmd's 8191-character limit. nx_env.cmd used to paste it into one line for
# vcvars64.bat and die with "The input line is too long." (issue #90); both of these calls have
# to answer instead. `cmd /c exit 0` needs nothing from the toolchain, so only the direct call,
# which runs vcvars64.bat, depends on Visual Studio being installed.
$longPath = $env:PATH
$n = 0
while ($longPath.Length -lt 7900 -and $n -lt 500) {
    $n++
    $longPath = "$longPath;C:\__emblemweave_cmd_check\$n"
}
if ($longPath.Length -lt 8191) {
    if (-not (Test-CmdScript -Script 'build_game.cmd' -Arguments '__missing' -Expected 'no game folder' -Path $longPath)) { $failed++ }
    if (-not (Test-CmdScript -Script 'nx_env.cmd' -Arguments 'cmd /c exit 0' -Expected '' -ExpectedExit 0 -Path $longPath -Launcher)) { $failed++ }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vsPath = ''
    if (Test-Path $vswhere) {
        $vsPath = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Out-String).Trim()
    }
    if (-not $vsPath) {
        Write-Host 'SKIP nx_env.cmd from a long PATH (direct): no Visual Studio C++ tools'
    } elseif (-not (Test-CmdScript -Script 'nx_env.cmd' -Arguments 'cmd /c exit 0' -Expected '' -ExpectedExit 0 -Path $longPath)) { $failed++ }
} else {
    Write-Host "SKIP the long-PATH cases: the inherited PATH is already $($longPath.Length) characters"
}

if ($failed -gt 0) {
    Write-Host "$failed cmd script check(s) failed"
    exit 1
}
Write-Host 'the scripts parsed and answered as expected'
exit 0
