param(
  [string]$Points = '',     # "x,y;x,y" window-relative clicks
  [string]$Shot = '',
  [switch]$Close,
  [switch]$NoStart,
  [string]$PageName = 'graphics',
  [int]$WaitMs = 4000
)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Ui {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$exe = 'C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\target\release\EmblemWeave.exe'
if ($NoStart) {
  $p = Get-Process -Name EmblemWeave -ErrorAction Stop | Select-Object -First 1
} else {
  $p = Start-Process -FilePath $exe -ArgumentList '--page',$PageName,'--size','1440x900' -PassThru
  Start-Sleep -Milliseconds $WaitMs
}
$p.Refresh()
$h = $p.MainWindowHandle
$r = New-Object Ui+RECT
[Ui]::GetWindowRect($h, [ref]$r) | Out-Null
Write-Output ("window " + $r.Left + "," + $r.Top + " " + ($r.Right - $r.Left) + "x" + ($r.Bottom - $r.Top))
# -1 is HWND_TOPMOST, then back to normal: the window comes above whatever the desktop had in front.
[Ui]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 0x0003) | Out-Null
[Ui]::BringWindowToTop($h) | Out-Null
[Ui]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 600
[Ui]::SetWindowPos($h, [IntPtr](-2), 0, 0, 0, 0, 0x0003) | Out-Null
$list = @()
foreach ($pair in ($Points -split ';')) {
  if (-not $pair.Trim()) { continue }
  $xy = $pair -split ','
  $list += ,@([int]$xy[0], [int]$xy[1])
}
foreach ($pt in $list) {
  $x = $r.Left + $pt[0]
  $y = $r.Top + $pt[1]
  [Ui]::SetCursorPos($x, $y) | Out-Null
  Start-Sleep -Milliseconds 200
  [Ui]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 70
  [Ui]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 900
  Write-Output ("clicked " + $pt[0] + "," + $pt[1])
}
if ($Shot) {
  Start-Sleep -Milliseconds 700
  $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
  $bmp.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Output "saved $Shot"
}
if ($Close) {
  $p.CloseMainWindow() | Out-Null
  Start-Sleep -Milliseconds 1500
  if (-not $p.HasExited) { $p.Kill() } else { Write-Output 'closed cleanly' }
}
