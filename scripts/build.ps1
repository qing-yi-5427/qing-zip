param(
    [string]$NsisPath = "makensis.exe",
    [ValidateSet('x86_64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$TargetDirectory,
    [string]$OutputDirectory = 'dist',
    [switch]$RunTests
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $projectRoot $OutputDirectory
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
Push-Location $projectRoot
try {
    $releaseVersion = (Get-Content obz-gui/tauri.conf.json -Raw | ConvertFrom-Json).version
    $releaseName = "qing-zip-$releaseVersion"
    $stage = Join-Path $OutputDirectory "$releaseName-portable"
    $setup = Join-Path $OutputDirectory "$releaseName-setup.exe"
    if (Test-Path -LiteralPath $setup) { throw "Installer already exists: $setup" }
    & python scripts/verify-runtime.py
    if ($LASTEXITCODE -ne 0) { throw 'Runtime integrity check failed' }
    $build = & (Join-Path $PSScriptRoot 'build-hardened.ps1') -Target $Target -TargetDirectory $TargetDirectory -RunTests:$RunTests
    & python scripts/package.py --binary-directory $build.BinaryDirectory --output-directory $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Portable package failed' }
    & $NsisPath /INPUTCHARSET UTF8 "/DVERSION=$releaseVersion" "/DSTAGING=$stage" "/DOUTPUT=$setup" scripts/installer.nsi
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
    & python scripts/package-engine-sources.py --output-directory $OutputDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Engine source package failed' }
    foreach ($artifact in @("$releaseName-portable.zip", "$releaseName-setup.exe", "$releaseName-engine-sources.zip")) {
        $path = Join-Path $OutputDirectory $artifact
        $checksum = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        "$checksum  $artifact" | Set-Content -LiteralPath "$path.sha256" -Encoding ascii
    }
} finally { Pop-Location }
