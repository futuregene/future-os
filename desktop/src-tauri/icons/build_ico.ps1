# Build a comprehensive icon.ico from icon.png for Windows taskbar clarity.
#
# Steps:
#   1. Crop transparent padding from icon.png (the artwork has ~40px margins
#      that waste pixels when the taskbar renders it at ~40px).
#   2. Generate all sizes Windows needs: 16,20,24,30,32,36,40,48,64,72,96,128,256.
#      The taskbar, Alt+Tab, and title bar each pick the closest match — no
#      GDI upscaling, no blurriness.
#
# Usage:  powershell -ExecutionPolicy Bypass -File build_ico.ps1
#
# The output (icon.ico) is referenced by tauri.conf.json bundle.icon and
# embedded into the exe by tauri-build. Rebuild the exe after running this.

Add-Type -AssemblyName System.Drawing

$srcPath = Join-Path $PSScriptRoot "icon.png"
$outPath = Join-Path $PSScriptRoot "icon.ico"

# ── Step 1: Crop transparent padding ─────────────────────────────────────

$img = [System.Drawing.Image]::FromFile($srcPath)
$bmp = New-Object System.Drawing.Bitmap($img)
$w = $bmp.Width
$h = $bmp.Height

# Find content bounds (alpha > 10)
$minX = $w; $minY = $h; $maxX = 0; $maxY = 0
for ($y = 0; $y -lt $h; $y++) {
    for ($x = 0; $x -lt $w; $x++) {
        $px = $bmp.GetPixel($x, $y)
        if ($px.A -gt 10) {
            if ($x -lt $minX) { $minX = $x }
            if ($x -gt $maxX) { $maxX = $x }
            if ($y -lt $minY) { $minY = $y }
            if ($y -gt $maxY) { $maxY = $y }
        }
    }
}

$contentW = $maxX - $minX + 1
$contentH = $maxY - $minY + 1
$size = [Math]::Max($contentW, $contentH)
$offsetX = [Math]::Max(0, $minX - [Math]::Floor(($size - $contentW) / 2))
$offsetY = [Math]::Max(0, $minY - [Math]::Floor(($size - $contentH) / 2))
$size = [Math]::Min($size, $w - $offsetX)
$size = [Math]::Min($size, $h - $offsetY)

Write-Output "Source: ${w}x${h}, content: ${contentW}x${contentH} at ($minX,$minY)"
Write-Output "Cropping to: ${size}x${size} at ($offsetX,$offsetY)"

# Draw only the content region into a square canvas
$cropped = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($cropped)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$srcRect = New-Object System.Drawing.Rectangle($offsetX, $offsetY, $size, $size)
$g.DrawImage($img, 0, 0, $srcRect, [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose()
$bmp.Dispose()
$img.Dispose()

# ── Step 2: Generate all ICO sizes from the cropped source ────────────────

$sizes = @(16, 20, 24, 30, 32, 36, 40, 48, 64, 72, 96, 128, 256)

$entries = @()
foreach ($s in $sizes) {
    $bmp2 = New-Object System.Drawing.Bitmap($s, $s)
    $g2 = [System.Drawing.Graphics]::FromImage($bmp2)
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g2.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g2.DrawImage($cropped, 0, 0, $s, $s)
    $g2.Dispose()

    $ms = New-Object System.IO.MemoryStream
    $bmp2.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $pngBytes = $ms.ToArray()
    $ms.Dispose()
    $bmp2.Dispose()

    $entries += @{
        Width  = $s
        Height = $s
        Data   = $pngBytes
        Size   = $pngBytes.Length
    }
    Write-Output "  ${s}x${s} : $($pngBytes.Length) bytes"
}
$cropped.Dispose()

# ── Step 3: Assemble ICO file ────────────────────────────────────────────

$icoMs = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($icoMs)

# Header: reserved(2) + type=1(2) + count(2)
$bw.Write([uint16]0)
$bw.Write([uint16]1)
$bw.Write([uint16]$entries.Count)

# Entry headers
$dataOffset = 6 + $entries.Count * 16
for ($i = 0; $i -lt $entries.Count; $i++) {
    $e = $entries[$i]
    $wVal = if ($e.Width -eq 256) { 0 } else { $e.Width }
    $hVal = if ($e.Height -eq 256) { 0 } else { $e.Height }
    $bw.Write([byte]$wVal)
    $bw.Write([byte]$hVal)
    $bw.Write([byte]0)          # color palette
    $bw.Write([byte]0)          # reserved
    $bw.Write([uint16]1)        # planes
    $bw.Write([uint16]32)       # bpp
    $bw.Write([uint32]$e.Size)
    $bw.Write([uint32]$dataOffset)
    $dataOffset += $e.Size
}

# PNG data
foreach ($e in $entries) {
    $bw.Write($e.Data)
}

$bw.Flush()
$icoBytes = $icoMs.ToArray()
$bw.Dispose()
$icoMs.Dispose()

[System.IO.File]::WriteAllBytes($outPath, $icoBytes)
Write-Output ""
Write-Output "Done: $outPath ($($icoBytes.Length) bytes, $($entries.Count) sizes)"
