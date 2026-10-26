param([string]$NsisPath = "makensis.exe")
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    & cargo build --release --locked --manifest-path obz-gui/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'GUI build failed' }
    & cargo build --release --locked --manifest-path obz-rs/Cargo.toml --target-dir obz-gui/target
    if ($LASTEXITCODE -ne 0) { throw 'Core build failed' }
    & python scripts/package.py
    if ($LASTEXITCODE -ne 0) { throw 'Portable package failed' }
    & $NsisPath /INPUTCHARSET UTF8 scripts/installer.nsi
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
    Get-ChildItem dist -File | Where-Object Extension -in '.zip','.exe' | ForEach-Object {
        $checksum = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower()
        "$checksum  $($_.Name)" | Set-Content -LiteralPath ($_.FullName+'.sha256') -Encoding ascii
    }
} finally { Pop-Location }
