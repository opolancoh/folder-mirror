# Builds the release binary for this Windows machine and copies it to dist\
# as a single file: dist\mirror-<version>-windows-<arch>.exe
#
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }

cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$output = Join-Path $dist "mirror-$version-windows-$arch.exe"
Copy-Item 'target\release\mirror.exe' $output -Force

Write-Host ''
Write-Host "Built: $output"
