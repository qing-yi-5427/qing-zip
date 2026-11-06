param(
    [string]$NsisPath = "makensis.exe",
    [ValidateSet('x86_64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$TargetDirectory,
    [switch]$RunTests
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    & python scripts/verify-runtime.py
    if ($LASTEXITCODE -ne 0) { throw 'Runtime integrity check failed' }
    $build = & (Join-Path $PSScriptRoot 'build-hardened.ps1') -Target $Target -TargetDirectory $TargetDirectory -RunTests:$RunTests
    & python scripts/package.py --binary-directory $build.BinaryDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Portable package failed' }
    $releaseVersion = (Get-Content obz-gui/tauri.conf.json -Raw | ConvertFrom-Json).version
    & $NsisPath /INPUTCHARSET UTF8 "/DVERSION=$releaseVersion" scripts/installer.nsi
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
    & python scripts/package-engine-sources.py
    if ($LASTEXITCODE -ne 0) { throw 'Engine source package failed' }
    Get-ChildItem dist -File | Where-Object Extension -in '.zip','.exe' | ForEach-Object {
        $checksum = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower()
        "$checksum  $($_.Name)" | Set-Content -LiteralPath ($_.FullName+'.sha256') -Encoding ascii
    }
} finally { Pop-Location }
