# Wrapper for tools/make_icon_from_png.py, so the icon can be built the same way from PowerShell.
#
#   pwsh -File tools/make_icon_from_png.ps1 -Source assets/branding/icon-source.png

param(
    [string]$Source = "assets/branding/icon-source.png",
    [string]$OutIco = "apps/emblemweave/assets/icon.ico",
    [string]$OutPng = "apps/emblemweave/assets/icon.png"
)

$ErrorActionPreference = 'Stop'
$py = (Get-Command python -ErrorAction SilentlyContinue).Source
if (-not $py) { throw 'python is needed to build the icon (Pillow: py -m pip install pillow)' }

& $py (Join-Path $PSScriptRoot 'make_icon_from_png.py') --source $Source --ico $OutIco --png $OutPng
if ($LASTEXITCODE -ne 0) { throw "icon build failed ($LASTEXITCODE)" }
