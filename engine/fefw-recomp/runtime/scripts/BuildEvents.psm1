<#
.SYNOPSIS
    Structured Build Event Protocol for EmblemWeave
.DESCRIPTION
    Provides JSON Lines formatted build events for structured logging and monitoring.
    Events are written to stdout (or a specified stream) in JSON Lines format.
#>

#requires -Version 5.1

param(
    [Parameter(Mandatory=$true)]
    [string]$BuildId,

    [string]$OutputPath = "stdout"
)

$script:buildId = $BuildId
$script:startTime = [DateTime]::UtcNow
$script:stageStartTimes = @{}
$script:eventWriter = $null
$script:isStdout = $OutputPath -eq "stdout"

function Initialize-EventWriter {
    if ($script:isStdout) {
        $script:eventWriter = $null  # Use Write-Host directly
    } else {
        $script:eventWriter = [System.IO.StreamWriter]::new($OutputPath, $true, [System.Text.Encoding]::UTF8)
        $script:eventWriter.AutoFlush = $true
    }
}

function Write-Event {
    param([object]$Event)
    $json = $Event | ConvertTo-Json -Compress -Depth 10
    if ($script:isStdout) {
        Write-Host $json
    } elseif ($script:eventWriter) {
        $script:eventWriter.WriteLine($json)
    }
}

function Get-Timestamp {
    return [DateTime]::UtcNow.ToString("yyyy-MM-ddTHH:mm:ss.fffZ")
}

function Get-DurationMs {
    param([DateTime]$Start)
    return [math]::Floor(([DateTime]::UtcNow - $Start).TotalMilliseconds)
}

# Public functions for emitting events

function Start-BuildEvent {
    param(
        [string]$ToolchainFingerprint,
        [hashtable]$Config
    )
    $event = @{
        event = "build_started"
        timestamp = Get-Timestamp
        build_id = $script:buildId
        toolchain_fingerprint = $ToolchainFingerprint
        config = $Config
    }
    Write-Event $event
}

function Start-StageEvent {
    param(
        [string]$StageId,
        [string]$StageName,
        [int]$StageIndex,
        [int]$TotalStages
    )
    $script:stageStartTimes[$StageId] = [DateTime]::UtcNow
    $event = @{
        event = "stage_started"
        timestamp = Get-Timestamp
        stage_id = $StageId
        stage_name = $StageName
        stage_index = $StageIndex
        total_stages = $TotalStages
    }
    Write-Event $event
}

function Progress-StageEvent {
    param(
        [string]$StageId,
        [float]$Progress,
        [string]$Message
    )
    $event = @{
        event = "stage_progress"
        timestamp = Get-Timestamp
        stage_id = $StageId
        progress = $Progress
        message = $Message
    }
    Write-Event $event
}

function Complete-StageEvent {
    param(
        [string]$StageId,
        [bool]$Success,
        [string]$Output = $null
    )
    $duration = 0
    if ($script:stageStartTimes.ContainsKey($StageId)) {
        $duration = Get-DurationMs $script:stageStartTimes[$StageId]
        $script:stageStartTimes.Remove($StageId)
    }
    $event = @{
        event = "stage_completed"
        timestamp = Get-Timestamp
        stage_id = $StageId
        duration_ms = $duration
        success = $Success
        output = $Output
    }
    Write-Event $event
}

function Fail-StageEvent {
    param(
        [string]$StageId,
        [string]$Error
    )
    $duration = 0
    if ($script:stageStartTimes.ContainsKey($StageId)) {
        $duration = Get-DurationMs $script:stageStartTimes[$StageId]
        $script:stageStartTimes.Remove($StageId)
    }
    $event = @{
        event = "stage_failed"
        timestamp = Get-Timestamp
        stage_id = $StageId
        duration_ms = $duration
        error = $Error
    }
    Write-Event $event
}

function Warning-Event {
    param(
        [string]$Message,
        [string]$StageId = $null
    )
    $event = @{
        event = "warning"
        timestamp = Get-Timestamp
        stage_id = $StageId
        message = $Message
    }
    Write-Event $event
}

function Error-Event {
    param(
        [string]$Error,
        [bool]$Recoverable = $true,
        [string]$StageId = $null
    )
    $event = @{
        event = "error"
        timestamp = Get-Timestamp
        stage_id = $StageId
        error = $Error
        recoverable = $Recoverable
    }
    Write-Event $event
}

function Complete-BuildEvent {
    param(
        [bool]$Success,
        [hashtable]$Summary
    )
    $duration = [math]::Floor(([DateTime]::UtcNow - $script:startTime).TotalMilliseconds)
    $event = @{
        event = "build_completed"
        timestamp = Get-Timestamp
        build_id = $script:buildId
        duration_ms = $duration
        success = $Success
        summary = $Summary
    }
    Write-Event $event
}

function Cancel-BuildEvent {
    param([string]$Reason)
    $event = @{
        event = "build_cancelled"
        timestamp = Get-Timestamp
        build_id = $script:buildId
        reason = $Reason
    }
    Write-Event $event
}

function Close-EventWriter {
    if ($script:eventWriter -and -not $script:isStdout) {
        $script:eventWriter.Close()
        $script:eventWriter = $null
    }
}

Export-ModuleMember -Function @(
    'Start-BuildEvent',
    'Start-StageEvent',
    'Progress-StageEvent',
    'Complete-StageEvent',
    'Fail-StageEvent',
    'Warning-Event',
    'Error-Event',
    'Complete-BuildEvent',
    'Cancel-BuildEvent',
    'Initialize-EventWriter',
    'Close-EventWriter'
)