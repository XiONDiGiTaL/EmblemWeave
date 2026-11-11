# Capture the EmblemWeave launcher window to a PNG.
#
# The launcher is a GPU-composited window, so the only dependable way to read it
# is to raise it above every other window and copy its rectangle off the screen.
param(
    [string]$Out = "C:\ewtarget\shot.png",
    [string]$ProcessName = "eblemweave"
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct EW_RECT { public int Left, Top, Right, Bottom; }
public class EW_Win {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out EW_RECT r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
}
"@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Error "no window for $ProcessName"; exit 1 }

$h = $proc.MainWindowHandle
[EW_Win]::ShowWindow($h, 9) | Out-Null
# SWP_NOSIZE 0x1, SWP_SHOWWINDOW 0x40, HWND_TOPMOST -1, then move to the corner.
[EW_Win]::SetWindowPos($h, [IntPtr](-1), 40, 40, 0, 0, 0x1 -bor 0x40) | Out-Null
[EW_Win]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 900

$r = New-Object EW_RECT
[EW_Win]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left
$hgt = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
# Drop topmost again so the window behaves normally afterwards.
[EW_Win]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x1 -bor 0x2) | Out-Null
Write-Output "$Out ${w}x${hgt}"
