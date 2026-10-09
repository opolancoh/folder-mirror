# Builds a release from Windows into dist\<version>\:
#   mirror-<version>-windows-x64.exe
#   mirror-<version>-linux-x64        (only if zig and cargo-zigbuild are installed)
#   SHA256SUMS.txt                    (covers every file in the folder)
# macOS binaries are built on a Mac with scripts/release.sh into the same folder.
#
# Run:  powershell -ExecutionPolicy Bypass -File scripts\release.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$out = Join-Path $root "dist\$version"
Write-Host "Release $version -> $out"

function Invoke-Cargo([string[]]$CargoArgs) {
    & cargo @CargoArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo $($CargoArgs -join ' ') failed" }
}

# Quality gate: nothing is built unless the code is clean and every test passes.
Write-Host "`n== Checks (clippy + tests)"
& cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { Write-Host "`nRelease stopped: clippy found problems. Nothing was built." -ForegroundColor Red; exit 1 }
& cargo test
if ($LASTEXITCODE -ne 0) { Write-Host "`nRelease stopped: some tests failed. Nothing was built." -ForegroundColor Red; exit 1 }
Write-Host 'All checks passed.' -ForegroundColor Green
New-Item -ItemType Directory -Force $out | Out-Null

# Windows x64 (C runtime linked in, see .cargo/config.toml)
Write-Host "`n== Windows x64"
Invoke-Cargo @('build', '--release', '--target', 'x86_64-pc-windows-msvc')
Copy-Item 'target\x86_64-pc-windows-msvc\release\mirror.exe' (Join-Path $out "mirror-$version-windows-x64.exe") -Force

# Linux x64, static (musl), cross-compiled with zig
Write-Host "`n== Linux x64"
if ((Get-Command zig -ErrorAction SilentlyContinue) -and (Get-Command cargo-zigbuild -ErrorAction SilentlyContinue)) {
    & rustup target add x86_64-unknown-linux-musl | Out-Null
    Invoke-Cargo @('zigbuild', '--release', '--target', 'x86_64-unknown-linux-musl')
    Copy-Item 'target\x86_64-unknown-linux-musl\release\mirror' (Join-Path $out "mirror-$version-linux-x64") -Force
} else {
    Write-Warning 'Skipped: install zig and cargo-zigbuild to build Linux from Windows:'
    Write-Warning '  winget install zig.zig'
    Write-Warning '  cargo install --locked cargo-zigbuild'
}

# Checksums for everything in the folder (including macOS files added from a Mac)
$sums = Get-ChildItem $out -File | Where-Object { $_.Name -ne 'SHA256SUMS.txt' } | Sort-Object Name | ForEach-Object {
    "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)"
}
[System.IO.File]::WriteAllLines((Join-Path $out 'SHA256SUMS.txt'), [string[]]$sums)

Write-Host "`nFiles in $out"
Get-ChildItem $out -File | ForEach-Object { Write-Host ("  {0,-40} {1,8:N1} MB" -f $_.Name, ($_.Length / 1MB)) }
if (-not (Get-ChildItem $out -Filter "mirror-$version-macos-*")) {
    Write-Host "`nmacOS: run 'sh scripts/release.sh' on a Mac and copy its files into dist/$version/."
}
