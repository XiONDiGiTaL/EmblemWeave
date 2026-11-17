Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$exe = 'C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\engine\fefw-recomp\game-build\fefw\build\release\fefw.exe'
$wd = Join-Path $env:APPDATA 'EmblemWeave\play'
$p = Start-Process -FilePath $exe -WorkingDirectory $wd -PassThru
Start-Sleep -Seconds 24
$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bmp.Size)
$out = Join-Path $PSScriptRoot 'fps-counter.png'
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved $out"
if (-not $p.HasExited) { $p.Kill() }
