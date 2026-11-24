#Requires -Version 5.1
<#
.SYNOPSIS
    Fails when a cmd parse error replaces the expected answer of build_game.cmd or nx_env.cmd.

.DESCRIPTION
    cmd parses a parenthesized if (...) block whole, even when its condition is false, so one
    unbalanced ) or an unescaped line continuation inside such a block breaks every call to the
    script, not just that branch. Neither script needs a build to show it: build_game.cmd answers
    an unknown game with "no game folder", and nx_env.cmd answers the removed --structured-events
    flag with its removal message. This runs those two calls and fails on a cmd parse error, a
    missing message, or a wrong exit code.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File engine\fefw-recomp\recomp\scripts\check_cmd_scripts.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$scripts = Split-Path -Parent $MyInvocation.MyCommand.Path
$parseErrors = @('was unexpected at this time', 'syntax of the command is incorrect')
$failed = 0

function Test-CmdScript {
    param(
        [string]$Script,
        [string]$Arguments,
        [string]$Expected,
        [int]$ExpectedExit = 2
    )

    Push-Location $scripts
    try {
        # 2>&1 goes to cmd, not to PowerShell: a redirected native stderr line becomes an
        # ErrorRecord, and with $ErrorActionPreference = 'Stop' that ends the run before the
        # failure can be reported.
        $output = (& cmd.exe /c ".\$Script $Arguments 2>&1" | Out-String)
        $code = $LASTEXITCODE
    } finally {
        Pop-Location
    }

    $label = "$Script $Arguments"
    $text = $output.Trim()
    foreach ($pattern in $parseErrors) {
        if ($text -like "*$pattern*") {
            Write-Host "FAIL $label"
            Write-Host "     cmd parse error: $text"
            return $false
        }
    }
    if ($text -notlike "*$Expected*") {
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

if ($failed -gt 0) {
    Write-Host "$failed cmd script check(s) failed"
    exit 1
}
Write-Host 'both scripts parsed and answered as expected'
exit 0
