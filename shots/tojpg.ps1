# Downscale a PNG capture to a JPEG small enough to look at. Usage: tojpg.ps1 -In a.png -Out a.jpg [-Width 700]
param(
    [Parameter(Mandatory = $true)][string]$In,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$Width = 700
)
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Image]::FromFile((Resolve-Path $In))
$height = [int]($Width * $src.Height / $src.Width)
$bmp = New-Object System.Drawing.Bitmap $Width, $height
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.InterpolationMode = 'HighQualityBicubic'
$gfx.DrawImage($src, 0, 0, $Width, $height)
$gfx.Dispose()
$enc = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
$params = New-Object System.Drawing.Imaging.EncoderParameters 1
$params.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), 74
$bmp.Save($Out, $enc, $params)
$bmp.Dispose()
$src.Dispose()
Write-Output "wrote $Out"
