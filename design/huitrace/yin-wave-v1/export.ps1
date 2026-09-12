$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$ink = [System.Drawing.Color]::FromArgb(255, 17, 19, 24)
$ivory = [System.Drawing.Color]::FromArgb(255, 248, 246, 240)
$transparent = [System.Drawing.Color]::FromArgb(0, 0, 0, 0)

function Test-BlackYinPoint {
    param([double]$X, [double]$Y, [double]$Center, [double]$Radius)

    # Undo the visual -35 degree rotation before testing the base geometry.
    $angle = 35.0 * [Math]::PI / 180.0
    $dx = $X - $Center
    $dy = $Y - $Center
    $unrotatedX = $Center + $dx * [Math]::Cos($angle) - $dy * [Math]::Sin($angle)
    $unrotatedY = $Center + $dx * [Math]::Sin($angle) + $dy * [Math]::Cos($angle)

    $lobeRadius = $Radius / 2.0
    $topCenterY = $Center - $lobeRadius
    $bottomCenterY = $Center + $lobeRadius
    $topInside = (($unrotatedX - $Center) * ($unrotatedX - $Center) + ($unrotatedY - $topCenterY) * ($unrotatedY - $topCenterY)) -le ($lobeRadius * $lobeRadius)
    $bottomInside = (($unrotatedX - $Center) * ($unrotatedX - $Center) + ($unrotatedY - $bottomCenterY) * ($unrotatedY - $bottomCenterY)) -le ($lobeRadius * $lobeRadius)

    return (($unrotatedX -le $Center) -or $topInside) -and (-not $bottomInside)
}

function Test-CapsulePoint {
    param([double]$X, [double]$Y, [double]$Left, [double]$Top, [double]$Width, [double]$Height)

    $radius = $Width / 2.0
    $centerX = $Left + $radius
    $topCenterY = $Top + $radius
    $bottomCenterY = $Top + $Height - $radius

    if ($X -ge $Left -and $X -lt ($Left + $Width) -and $Y -ge $topCenterY -and $Y -le $bottomCenterY) {
        return $true
    }

    $topDistance = (($X - $centerX) * ($X - $centerX)) + (($Y - $topCenterY) * ($Y - $topCenterY))
    $bottomDistance = (($X - $centerX) * ($X - $centerX)) + (($Y - $bottomCenterY) * ($Y - $bottomCenterY))
    return $topDistance -le ($radius * $radius) -or $bottomDistance -le ($radius * $radius)
}

function Get-SmallWaveSpec {
    param([int]$Size)

    switch ($Size) {
        16 { return @{ Width = 2; Gap = 2; Heights = @(6, 12, 6) } }
        20 { return @{ Width = 2; Gap = 1; Heights = @(5, 9, 14, 9, 5) } }
        24 { return @{ Width = 2; Gap = 2; Heights = @(6, 11, 17, 11, 6) } }
        32 { return @{ Width = 4; Gap = 2; Heights = @(8, 15, 24, 15, 8) } }
        40 { return @{ Width = 4; Gap = 3; Heights = @(10, 19, 30, 19, 10) } }
        48 { return @{ Width = 6; Gap = 3; Heights = @(12, 23, 36, 23, 12) } }
        64 { return @{ Width = 8; Gap = 4; Heights = @(16, 30, 48, 30, 16) } }
        default { throw "No pixel-fitted wave specification for $Size px." }
    }
}

function New-PixelFittedIcon {
    param([int]$Size)

    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear($transparent)
    $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
    $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias

    $margin = if ($Size -le 20) { 1.0 } elseif ($Size -le 32) { 1.5 } elseif ($Size -le 48) { 2.0 } else { 3.0 }
    $stroke = if ($Size -le 24) { 1.0 } elseif ($Size -le 40) { 2.0 } elseif ($Size -le 48) { 2.5 } else { 3.0 }
    $diameter = $Size - 2.0 * $margin
    $center = $Size / 2.0
    $radius = $diameter / 2.0
    $bounds = [System.Drawing.RectangleF]::new([float]$margin, [float]$margin, [float]$diameter, [float]$diameter)

    $ivoryBrush = [System.Drawing.SolidBrush]::new($ivory)
    $inkBrush = [System.Drawing.SolidBrush]::new($ink)
    $graphics.FillEllipse($ivoryBrush, $bounds)

    $circlePath = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $circlePath.AddEllipse($bounds)
    $state = $graphics.Save()
    $graphics.SetClip($circlePath)

    $matrix = [System.Drawing.Drawing2D.Matrix]::new()
    $matrix.RotateAt(-35.0, [System.Drawing.PointF]::new([float]$center, [float]$center))
    $graphics.Transform = $matrix
    $graphics.FillRectangle($inkBrush, [float]($center - $radius), [float]($center - $radius), [float]$radius, [float](2.0 * $radius))
    $graphics.FillEllipse($inkBrush, [float]($center - $radius / 2.0), [float]($center - $radius), [float]$radius, [float]$radius)
    $graphics.FillEllipse($ivoryBrush, [float]($center - $radius / 2.0), [float]$center, [float]$radius, [float]$radius)
    $graphics.Restore($state)
    $matrix.Dispose()
    $circlePath.Dispose()

    $borderPen = [System.Drawing.Pen]::new($ink, [float]$stroke)
    $borderPen.Alignment = [System.Drawing.Drawing2D.PenAlignment]::Inset
    $graphics.DrawEllipse($borderPen, $bounds)
    $borderPen.Dispose()
    $graphics.Dispose()
    $ivoryBrush.Dispose()
    $inkBrush.Dispose()

    # Draw the waveform directly on the target pixel grid. Every gap is at
    # least one physical pixel, so Windows scaling cannot merge adjacent bars.
    $spec = Get-SmallWaveSpec -Size $Size
    $barWidth = [int]$spec.Width
    $gap = [int]$spec.Gap
    $heights = @($spec.Heights)
    $totalWidth = $heights.Count * $barWidth + ($heights.Count - 1) * $gap
    $startX = [Math]::Floor(($Size - $totalWidth) / 2.0)

    for ($index = 0; $index -lt $heights.Count; $index++) {
        $height = [int]$heights[$index]
        $left = $startX + $index * ($barWidth + $gap)
        $top = [Math]::Floor(($Size - $height) / 2.0)

        for ($y = 0; $y -lt $Size; $y++) {
            for ($x = 0; $x -lt $Size; $x++) {
                $pixelX = $x + 0.5
                $pixelY = $y + 0.5
                if (Test-CapsulePoint -X $pixelX -Y $pixelY -Left $left -Top $top -Width $barWidth -Height $height) {
                    $isBlack = Test-BlackYinPoint -X $pixelX -Y $pixelY -Center $center -Radius $radius
                    $bitmap.SetPixel($x, $y, $(if ($isBlack) { $ivory } else { $ink }))
                }
            }
        }
    }

    return $bitmap
}

$sourcePath = Join-Path $PSScriptRoot 'huitrace-yin-wave.png'
$source = [System.Drawing.Bitmap]::new($sourcePath)
$sizes = @(16, 20, 24, 32, 40, 48, 64, 96, 128, 256)
$pixelFittedSizes = @(16, 20, 24, 32, 40, 48, 64)
$frames = @()

foreach ($size in $sizes) {
    if ($pixelFittedSizes -contains $size) {
        $bitmap = New-PixelFittedIcon -Size $size
    } else {
        $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
        $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $graphics.DrawImage($source, 0, 0, $size, $size)
        $graphics.Dispose()
    }

    $stream = [System.IO.MemoryStream]::new()
    $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $frames += ,$stream.ToArray()
    $bitmap.Save((Join-Path $PSScriptRoot "icon-$size.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $stream.Dispose()
    $bitmap.Dispose()
}

$icoPath = Join-Path $PSScriptRoot 'huitrace-yin-wave.ico'
$file = [System.IO.File]::Create($icoPath)
$writer = [System.IO.BinaryWriter]::new($file)
$writer.Write([uint16]0)
$writer.Write([uint16]1)
$writer.Write([uint16]$sizes.Count)

$offset = 6 + 16 * $sizes.Count
for ($index = 0; $index -lt $sizes.Count; $index++) {
    $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
    $writer.Write([byte]$dimension)
    $writer.Write([byte]$dimension)
    $writer.Write([byte]0)
    $writer.Write([byte]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]32)
    $writer.Write([uint32]$frames[$index].Length)
    $writer.Write([uint32]$offset)
    $offset += $frames[$index].Length
}

foreach ($frame in $frames) {
    $writer.Write([byte[]]$frame)
}

$writer.Dispose()
$source.Dispose()

Write-Output "Exported seven pixel-fitted taskbar sizes, three high-resolution sizes, and $icoPath"
