$ErrorActionPreference = 'Stop'

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$tauri = Join-Path $projectRoot 'frontend/node_modules/.bin/tauri.cmd'
$generated = Join-Path $PSScriptRoot 'generated'

if (-not (Test-Path -LiteralPath $tauri)) {
    throw "Tauri CLI was not found at $tauri"
}

& $tauri icon (Join-Path $PSScriptRoot 'icon.svg') --output $generated
if ($LASTEXITCODE -ne 0) {
    throw 'Tauri icon generation failed'
}

Copy-Item -LiteralPath (Join-Path $generated 'icon.png') -Destination (Join-Path $PSScriptRoot 'huitrace-yin-wave.png') -Force
& (Join-Path $PSScriptRoot 'export.ps1')

$frontend = Join-Path $projectRoot 'frontend'
Copy-Item (Join-Path $generated '*') (Join-Path $frontend 'src-tauri/icons') -Recurse -Force

# Use the richer ten-frame ICO for Windows and the web favicon.
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'huitrace-yin-wave.ico') -Destination (Join-Path $frontend 'src-tauri/icons/icon.ico') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'huitrace-yin-wave.ico') -Destination (Join-Path $frontend 'src/app/favicon.ico') -Force

# Public UI assets and compatibility entry points.
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'icon.svg') -Destination (Join-Path $frontend 'public/huitrace-icon-yin-wave.svg') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'huitrace-yin-wave.png') -Destination (Join-Path $frontend 'public/huitrace-icon-yin-wave.png') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'huitrace-yin-wave.png') -Destination (Join-Path $frontend 'public/huitrace-icon.png') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'icon-128.png') -Destination (Join-Path $frontend 'public/icon_128x128.png') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'icon-64.png') -Destination (Join-Path $frontend 'public/icon_32x32@2x.png') -Force

Write-Output 'Applied the yin-wave icon to Tauri, favicon, and public UI assets.'
