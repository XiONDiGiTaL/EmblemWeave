# Capture one launcher page as a window-only PNG: start it on that page at a fixed size, bring it to the front, grab the
# window rectangle (not the desktop), then close it. Usage: shot-page.ps1 -Page overview -Out shots\docs-new\overview.png
param(
    [Parameter(Mandatory = $true)][string]$Page,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Size = '1440x900',
    [int]$WaitMs = 4200
)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out RECT val, int size);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$exe = Join-Path $PSScriptRoot '..\target\release\EmblemWeave.exe'
$p = Start-Process -FilePath $exe -ArgumentList '--page', $Page, '--size', $Size -PassThru
Start-Sleep -Milliseconds $WaitMs
$p.Refresh()
$h = $p.MainWindowHandle
if ($h -eq 0) { throw "the launcher window did not appear" }
$flags = (0x0003 -bor 0x0040)   # SWP_NOSIZE + SWP_SHOWWINDOW, so moving the window does not resize the page
[void][Win]::SetWindowPos($h, [IntPtr](-1), 60, 60, 0, 0, $flags)
[void][Win]::SetWindowPos($h, [IntPtr](-2), 60, 60, 0, 0, $flags)
[void][Win]::BringWindowToTop($h)
[void][Win]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 1400
# The visible frame, dropping the resize border DWM adds around it.
$r = New-Object Win+RECT
[void][Win]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)
$w = $r.Right - $r.Left
$hh = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$gfx.Dispose()
$bmp.Dispose()
Write-Output "saved $Out ${w}x${hh}"
$p.CloseMainWindow() | Out-Null
Start-Sleep -Milliseconds 1200
if (-not $p.HasExited) { $p.Kill() }
