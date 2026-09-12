$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$frontend = Join-Path $projectRoot 'frontend'
$encoded = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $PSScriptRoot 'artwork.png')))
# The artwork is full bleed; this native SVG wrapper defines the production silhouette.
$svg = '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1024" height="1024" viewBox="0 0 1024 1024"><defs><clipPath id="tile"><rect x="16" y="16" width="992" height="992" rx="208"/></clipPath></defs><image x="16" y="16" width="992" height="992" clip-path="url(#tile)" xlink:href="data:image/png;base64,' + $encoded + '"/></svg>'
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'icon.svg'),$svg)
$generated = Join-Path $PSScriptRoot 'generated'
& (Join-Path $frontend 'node_modules/.bin/tauri.cmd') icon (Join-Path $PSScriptRoot 'icon.svg') --output $generated
if ($LASTEXITCODE -ne 0) { throw 'Tauri icon generation failed' }
Copy-Item (Join-Path $generated '*') (Join-Path $frontend 'src-tauri/icons') -Recurse -Force
Copy-Item (Join-Path $generated 'icon.png') (Join-Path $PSScriptRoot 'huitrace-ink.png')
& (Join-Path $PSScriptRoot 'export.ps1')
Copy-Item (Join-Path $PSScriptRoot 'huitrace-ink.ico') (Join-Path $frontend 'src-tauri/icons/icon.ico')
Copy-Item (Join-Path $PSScriptRoot 'huitrace-ink.ico') (Join-Path $frontend 'src/app/favicon.ico')
Copy-Item (Join-Path $PSScriptRoot 'huitrace-ink.png') (Join-Path $frontend 'public/huitrace-icon-ink.png')
Copy-Item (Join-Path $PSScriptRoot 'huitrace-ink.png') (Join-Path $frontend 'public/huitrace-icon.png')
Copy-Item (Join-Path $PSScriptRoot 'icon-128.png') (Join-Path $frontend 'public/icon_128x128.png')
Copy-Item (Join-Path $PSScriptRoot 'icon-64.png') (Join-Path $frontend 'public/icon_32x32@2x.png')
