# Click a point inside the EmblemWeave launcher window, for scripted UI checks.
#
# Coordinates are relative to the window's top-left corner, the same frame as a
# screenshot from tools/capture_window.ps1.
param(
    [int]$X,
    [int]$Y,
    [string]$ProcessName = "EmblemWeave"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct EWC_RECT { public int Left, Top, Right, Bottom; }
public class EWC_Win {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out EWC_RECT r);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
}
"@

$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $proc) { Write-Error "no window for $ProcessName"; exit 1 }
$h = $proc.MainWindowHandle
[EWC_Win]::SetForegroundWindow($h) | Out-Null
$r = New-Object EWC_RECT
[EWC_Win]::GetWindowRect($h, [ref]$r) | Out-Null
[EWC_Win]::SetCursorPos($r.Left + $X, $r.Top + $Y) | Out-Null
Start-Sleep -Milliseconds 120
# LEFTDOWN 0x2, LEFTUP 0x4
[EWC_Win]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[EWC_Win]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
