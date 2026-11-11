# Generate the EmblemWeave application icon.
#
# The in-app mark is drawn live in egui and recolours per scheme. A window and
# taskbar icon has to be a bitmap, so this script renders the same crest once in
# the Emblem palette (gold monogram, steel shield, teal ribbon) and writes it to
# apps/eblemweave/assets/icon.png, which the launcher loads at startup.
#
# Run from the repository root:
#   powershell -ExecutionPolicy Bypass -File tools/make_icon.ps1

param([int]$Size = 256)

Add-Type -AssemblyName System.Drawing

$out = Join-Path $PSScriptRoot "..\apps\eblemweave\assets\icon.png"
New-Item -ItemType Directory -Force -Path (Split-Path $out) | Out-Null

$bmp = New-Object System.Drawing.Bitmap $Size, $Size
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.Clear([System.Drawing.Color]::Transparent)

function U([double]$x, [double]$y) {
    New-Object System.Drawing.PointF ([single]($x * $Size)), ([single]($y * $Size))
}

# Palette: the Emblem scheme, matching apps/eblemweave/src/theme.rs.
$metal      = [System.Drawing.Color]::FromArgb(0xff, 0xcd, 0xd2, 0xdb)
$metalDeep  = [System.Drawing.Color]::FromArgb(0xff, 0xb6, 0xbc, 0xc8)
$edge       = [System.Drawing.Color]::FromArgb(0xff, 0x63, 0x67, 0x70)
$gold       = [System.Drawing.Color]::FromArgb(0xff, 0xc9, 0xa2, 0x27)
$goldHi     = [System.Drawing.Color]::FromArgb(0xff, 0xe9, 0xcd, 0x6b)
$royal      = [System.Drawing.Color]::FromArgb(0xff, 0x1d, 0x48, 0xaf)
$weave      = [System.Drawing.Color]::FromArgb(0xff, 0x30, 0x9f, 0xad)
$weaveHi    = [System.Drawing.Color]::FromArgb(0xff, 0x8f, 0xdc, 0xdf)

# Shield outline: straight top edge, vertical shoulders, quarter-ellipse point.
$pts = New-Object System.Collections.Generic.List[System.Drawing.PointF]
$n = 40
for ($i = 0; $i -le $n; $i++) {
    $t = $i / $n
    $pts.Add((U (0.14 + 0.72 * $t) 0.075))
}
for ($i = 1; $i -le $n; $i++) {
    $t = $i / $n
    if ($t -le 0.46) { $hw = 1.0 } else { $u = ($t - 0.46) / 0.54; $hw = [math]::Sqrt([math]::Max(0.0, 1 - $u * $u)) }
    $pts.Add((U (0.5 + 0.36 * $hw) (0.075 + 0.875 * $t)))
}
for ($i = $n - 1; $i -ge 1; $i--) {
    $t = $i / $n
    if ($t -le 0.46) { $hw = 1.0 } else { $u = ($t - 0.46) / 0.54; $hw = [math]::Sqrt([math]::Max(0.0, 1 - $u * $u)) }
    $pts.Add((U (0.5 - 0.36 * $hw) (0.075 + 0.875 * $t)))
}
$outline = $pts.ToArray()

function Inset([System.Drawing.PointF[]]$poly, [double]$amount) {
    $cx = 0.5 * $Size; $cy = 0.57 * $Size
    $r = @()
    foreach ($p in $poly) {
        $r += New-Object System.Drawing.PointF ([single]($cx + ($p.X - $cx) * (1 - $amount))), ([single]($cy + ($p.Y - $cy) * (1 - $amount)))
    }
    return $r
}

# Drop shadow, dark keyline, light rim, engraved ring, pale field.
$shadow = Inset $outline -0.012
$shadowBrush = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(120, 0, 0, 0))
$g.FillPolygon($shadowBrush, $shadow)

$edgeBrush = New-Object System.Drawing.SolidBrush $edge
$g.FillPolygon($edgeBrush, $outline)

$rim = Inset $outline 0.028
$metalBrush = New-Object System.Drawing.SolidBrush $metal
$g.FillPolygon($metalBrush, $rim)

$ring = Inset $outline 0.062
$deepBrush = New-Object System.Drawing.SolidBrush $metalDeep
$g.FillPolygon($deepBrush, $ring)
$edgePen = New-Object System.Drawing.Pen $edge, ([single]([math]::Max(1.0, $Size * 0.006)))
$g.DrawPolygon($edgePen, $ring)

$field = Inset $outline 0.082
$g.FillPolygon($metalBrush, $field)
$pin = Inset $outline 0.048
$royalPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(140, $royal)), ([single]([math]::Max(1.0, $Size * 0.005)))
$g.DrawPolygon($royalPen, $pin)

# The ribbon, sampled through the same cubic as the live mark.
$p0 = U 0.175 0.630; $c1 = U 0.330 0.820; $c2 = U 0.690 0.815; $p1 = U 0.835 0.585
$band = @()
for ($i = 0; $i -le 60; $i++) {
    $t = $i / 60.0
    $u = 1 - $t
    $x = $p0.X * $u * $u * $u + $c1.X * 3 * $u * $u * $t + $c2.X * 3 * $u * $t * $t + $p1.X * $t * $t * $t
    $y = $p0.Y * $u * $u * $u + $c1.Y * 3 * $u * $u * $t + $c2.Y * 3 * $u * $t * $t + $p1.Y * $t * $t * $t
    $band += New-Object System.Drawing.PointF ([single]$x), ([single]$y)
}
$half = $Size * 0.040
$weavePen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(0xff, 0x05, 0x05, 0x06)), ([single]($half * 2 * 1.3))
$weavePen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
$g.DrawLines($weavePen, $band)
$fillPen = New-Object System.Drawing.Pen $weave, ([single]($half * 2))
$fillPen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
$g.DrawLines($fillPen, $band)
$hiPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(200, $weaveHi)), ([single]($half * 0.8))
$hiPen.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
$g.DrawLines($hiPen, $band)

# The monogram: gold face with a royal keyline, W in front of E.
function Glyph([string]$text, [double]$cx, [double]$cy, [double]$em, [double]$dx, [double]$dy) {
    $family = New-Object System.Drawing.FontFamily "Georgia"
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $fmt = [System.Drawing.StringFormat]::GenericTypographic
    $path.AddString($text, $family, [int][System.Drawing.FontStyle]::Bold, [single]($em * $Size), (U $cx $cy), $fmt)
    # Centre the glyph on its own bounds rather than on its advance.
    $b = $path.GetBounds()
    $m = New-Object System.Drawing.Drawing2D.Matrix
    $m.Translate([single](($cx * $Size) + $dx - $b.X - $b.Width / 2), [single](($cy * $Size) + $dy - $b.Y - $b.Height / 2))
    $path.Transform($m)
    return $path
}

$keyline = [single]([math]::Max(1.4, $Size * 0.010))
$ePath = Glyph "E" 0.355 0.455 0.455 0 0
$wPath = Glyph "W" 0.640 0.520 0.505 0 0

$shadowPen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(150, 0, 0, 0)), $keyline
foreach ($path in @($ePath, $wPath)) {
    $m = New-Object System.Drawing.Drawing2D.Matrix
    $m.Translate([single]($Size * 0.012), [single]($Size * 0.016))
    $copy = $path.Clone()
    $copy.Transform($m)
    $g.DrawPath($shadowPen, $copy)
}

$royalOutline = New-Object System.Drawing.Pen $royal, ($keyline * 1.5)
$goldBrush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    (U 0 0.25), (U 0 0.65), $goldHi, $gold)
foreach ($path in @($ePath, $wPath)) {
    $g.DrawPath($royalOutline, $path)
    $g.FillPath($goldBrush, $path)
}

$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "wrote $out (${Size}x${Size})"
