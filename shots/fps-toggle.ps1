Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win2 {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$exe = 'C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp\game-build\fefw\build\release\fefw.exe'
$wd = Join-Path $env:APPDATA 'EmblemWeave\play'
$p = Start-Process -FilePath $exe -WorkingDirectory $wd -PassThru
Start-Sleep -Seconds 22
$p.Refresh()
$r = New-Object Win2+RECT
[Win2]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$x = [int](($r.Left + $r.Right) / 2); $y = [int](($r.Top + $r.Bottom) / 2)
[Win2]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 250
[Win2]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Win2]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Seconds 2
[Win2]::keybd_event(0x71, 0, 0, [UIntPtr]::Zero)   # VK_F2
Start-Sleep -Milliseconds 90
[Win2]::keybd_event(0x71, 0, 2, [UIntPtr]::Zero)
Start-Sleep -Seconds 3
$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
$bmp.Save((Join-Path $PSScriptRoot 'fps-hidden.png'), [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output 'captured'
if (-not $p.HasExited) { $p.Kill() }
