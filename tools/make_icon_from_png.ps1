# Turns a supplied PNG into the Windows program icon.
#
# The launcher, the built fefw.exe and the taskbar all read apps/emblemweave/assets/icon.ico.
# This script does not draw anything: it takes the artwork as given, trims the empty border,
# centres it on a transparent square, and writes every size Windows asks for into one .ico.
#
#   pwsh -File tools/make_icon_from_png.ps1 -Source assets/branding/icon-source.png
#
# The source can have a transparent or a white background. The crop keeps the artwork's
# proportions and pads it to a square with a small margin, which is what makes a wide
# winged crest legible at 16 px.

param(
    [string]$Source = "assets/branding/icon-source.png",
    [string]$OutIco = "apps/emblemweave/assets/icon.ico",
    [string]$OutPng = "apps/emblemweave/assets/icon.png"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$root = Split-Path -Parent $PSScriptRoot
$src = if ([System.IO.Path]::IsPathRooted($Source)) { $Source } else { Join-Path $root $Source }
if (-not (Test-Path $src)) { throw "no source image at $src" }

$image = [System.Drawing.Bitmap]::FromFile((Resolve-Path $src))
try {
    # Background: keep transparency when the file has it, otherwise treat near-white as empty.
    $hasAlpha = $false
    $minX = $image.Width; $minY = $image.Height; $maxX = -1; $maxY = -1
    for ($y = 0; $y -lt $image.Height; $y += 2) {
        for ($x = 0; $x -lt $image.Width; $x += 2) {
            $c = $image.GetPixel($x, $y)
            if ($c.A -lt 16) { $hasAlpha = $true; continue }
            if ($c.R -gt 244 -and $c.G -gt 244 -and $c.B -gt 244) { continue }
            if ($x -lt $minX) { $minX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
    if ($maxX -lt 0) { throw "the source image is empty or entirely white" }
    $boxW = $maxX - $minX + 1
    $boxH = $maxY - $minY + 1

    # Square source box that holds the artwork with a 6% margin, centred on it.
    $side = [int]([Math]::Max($boxW, $boxH) * 1.12)
    $cx = [int](($minX + $maxX) / 2)
    $cy = [int](($minY + $maxY) / 2)
    $sx = [Math]::Max(0, $cx - [int]($side / 2))
    $sy = [Math]::Max(0, $cy - [int]($side / 2))
    $side = [Math]::Min($side, [Math]::Min($image.Width - $sx, $image.Height - $sy))

    $canvas = New-Object System.Drawing.Bitmap 512, 512
    $g = [System.Drawing.Graphics]::FromImage($canvas)
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $dest = New-Object System.Drawing.Rectangle 0, 0, 512, 512
    $srcRect = New-Object System.Drawing.Rectangle $sx, $sy, $side, $side
    $g.DrawImage($image, $dest, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()

    $sizes = @(16, 20, 24, 32, 40, 48, 64, 128, 256)
    $images = @()
    foreach ($s in $sizes) {
        $bmp = New-Object System.Drawing.Bitmap $s, $s
        $gg = [System.Drawing.Graphics]::FromImage($bmp)
        $gg.Clear([System.Drawing.Color]::Transparent)
        $gg.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $gg.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $gg.DrawImage($canvas, (New-Object System.Drawing.Rectangle 0, 0, $s, $s))
        $gg.Dispose()
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $images += ,@($s, $ms.ToArray())
        $bmp.Dispose(); $ms.Dispose()
    }

    # ICO container: header, one directory entry per size, then the PNG payloads.
    $outPath = Join-Path $root $OutIco
    $dir = Split-Path -Parent $outPath
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $fs = [System.IO.File]::Create($outPath)
    $bw = New-Object System.IO.BinaryWriter $fs
    try {
        $bw.Write([UInt16]0); $bw.Write([UInt16]1); $bw.Write([UInt16]$images.Count)
        $offset = 6 + 16 * $images.Count
        foreach ($im in $images) {
            $s = $im[0]; $bytes = $im[1]
            $dim = if ($s -ge 256) { 0 } else { $s }
            $bw.Write([byte]$dim); $bw.Write([byte]$dim)
            $bw.Write([byte]0); $bw.Write([byte]0)
            $bw.Write([UInt16]1); $bw.Write([UInt16]32)
            $bw.Write([UInt32]$bytes.Length); $bw.Write([UInt32]$offset)
            $offset += $bytes.Length
        }
        foreach ($im in $images) { $bw.Write($im[1]) }
    } finally { $bw.Dispose(); $fs.Dispose() }

    # The 256 px still, for the window icon and the repository preview.
    $png = New-Object System.Drawing.Bitmap 256, 256
    $gp = [System.Drawing.Graphics]::FromImage($png)
    $gp.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $gp.DrawImage($canvas, (New-Object System.Drawing.Rectangle 0, 0, 256, 256))
    $gp.Dispose()
    $png.Save((Join-Path $root $OutPng), [System.Drawing.Imaging.ImageFormat]::Png)
    $png.Dispose(); $canvas.Dispose()

    Write-Host "wrote $outPath ($($images.Count) sizes) and $OutPng from $src"
} finally {
    $image.Dispose()
}
