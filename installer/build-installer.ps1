# Builds the Windows app and packs it into dist\NetworkTester-Setup-<version>.exe
# Needs: Rust (cargo) and Inno Setup (ISCC.exe). Get Inno Setup at https://jrsoftware.org/isdl.php
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Version comes from the client crate, so there is one place to change it.
$version = (Select-String -Path "crates\client\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value

# Find the Inno Setup compiler.
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $bases = @("$env:LOCALAPPDATA\Programs", "${env:ProgramFiles(x86)}", "$env:ProgramFiles") | Where-Object { $_ -and (Test-Path $_) }
    $iscc = $bases |
        ForEach-Object { Get-ChildItem $_ -Directory -Filter "Inno Setup*" -ErrorAction SilentlyContinue } |
        ForEach-Object { Join-Path $_.FullName "ISCC.exe" } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
}
if (-not $iscc) { throw "Inno Setup (ISCC.exe) not found. Install it from https://jrsoftware.org/isdl.php" }

Write-Host "==> Building the app (release)"
cargo build --release --locked -p network-tester
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

Write-Host "==> Building the installer (version $version)"
& $iscc "/DAppVersion=$version" "installer\NetworkTester.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

$out = "dist\NetworkTester-Setup-$version.exe"
Write-Host ""
Write-Host "Done: $out ($([math]::Round((Get-Item $out).Length / 1MB, 1)) MB)"
