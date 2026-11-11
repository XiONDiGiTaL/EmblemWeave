# Capture launcher pages to docs/screenshots for the README.
#
# Sends no keystrokes or clicks: each page is opened with `--page <name>`, and
# the boot captures use `--boot-test`. Each capture is taken with PrintWindow,
# which renders only the launcher's own window, so nothing else on the screen
# can ever appear in a screenshot. The title strip and the invisible resize
# border are cropped off.
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\EmblemWeave.exe",
    [string]$OutDir = "$PSScriptRoot\..\docs\screenshots",
    [string[]]$Only = @(),
    [int]$Crop = 32,
    [int]$Border = 8,
    [int]$BootSeconds = 45,
    [switch]$SkipBoot
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct EWP_RECT { public int Left, Top, Right, Bottom; }
public class EWP_Win {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out EWP_RECT r);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
}
"@

New-Item -ItemType Directory -Force $OutDir | Out-Null

function Open-Launcher([string[]]$launchArgs) {
    $p = Start-Process -FilePath $Exe -ArgumentList $launchArgs -WorkingDirectory (Split-Path $Exe) -PassThru
    for ($i = 0; $i -lt 40 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 250; $p.Refresh() }
    # Size the window without raising it or taking focus (SWP_NOZORDER 0x4, SWP_NOACTIVATE 0x10).
    [EWP_Win]::SetWindowPos($p.MainWindowHandle, [IntPtr]::Zero, 40, 40, 1280, 900, 0x14) | Out-Null
    Start-Sleep -Seconds 2
    return $p
}

function Save-Shot($p, [string]$name) {
    $h = $p.MainWindowHandle
    # Raise it above other windows for a moment (no input is sent): a window
    # that is fully covered does not draw, and would capture as black.
    [EWP_Win]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x3) | Out-Null
    Start-Sleep -Milliseconds 900
    $r = New-Object EWP_RECT
    [EWP_Win]::GetWindowRect($h, [ref]$r) | Out-Null
    $fw = $r.Right - $r.Left; $fh = $r.Bottom - $r.Top
    $full = New-Object System.Drawing.Bitmap $fw, $fh
    $g = [System.Drawing.Graphics]::FromImage($full)
    $hdc = $g.GetHdc()
    # PW_RENDERFULLCONTENT (2): includes GPU-composited content.
    $ok = [EWP_Win]::PrintWindow($h, $hdc, 2)
    $g.ReleaseHdc($hdc); $g.Dispose()
    if (-not $ok) { $full.Dispose(); Write-Error "PrintWindow failed for $name"; return }
    $rect = New-Object System.Drawing.Rectangle $Border, $Crop, ($fw - 2 * $Border), ($fh - $Crop - $Border)
    $shot = $full.Clone($rect, $full.PixelFormat)
    $path = Join-Path $OutDir "$name.png"
    $shot.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $shot.Dispose(); $full.Dispose()
    [EWP_Win]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x3) | Out-Null
    Write-Output "$path $($rect.Width)x$($rect.Height)"
}

$pages = [ordered]@{
    "overview" = "Overview"; "setup" = "Game Setup"; "recompilation" = "Recompilation"
    "graphics" = "Graphics"; "audio" = "Audio"; "controls" = "Controls"
    "appearance" = "Appearance"; "diagnostics" = "Diagnostics"; "about" = "About"
}
foreach ($file in $pages.Keys) {
    if ($Only.Count -gt 0 -and $Only -notcontains $file) { continue }
    $p = Open-Launcher @("--page", "`"$($pages[$file])`"")
    Save-Shot $p $file
    Stop-Process -Id $p.Id
}

if (-not $SkipBoot) {
    # A boot test: running, finished, then its diagnostics.
    $p = Open-Launcher @("--boot-test")
    Start-Sleep -Seconds 3
    Save-Shot $p "boot-running"
    Start-Sleep -Seconds $BootSeconds
    Save-Shot $p "boot-done"
    Stop-Process -Id $p.Id
    $p = Open-Launcher @("--boot-test", "--page", "Diagnostics")
    Start-Sleep -Seconds ($BootSeconds + 3)
    Save-Shot $p "diagnostics-run"
    Stop-Process -Id $p.Id
}
