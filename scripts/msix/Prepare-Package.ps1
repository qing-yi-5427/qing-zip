[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ComponentDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Publisher,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+\.\d+$')][string]$Version,
    [ValidateSet('x64','arm64')][string]$Architecture='x64',
    [string]$MakeAppxPath='makeappx.exe',
    [string]$ManifestToolPath='mt.exe'
)
$ErrorActionPreference='Stop'
$repo=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$component=(Resolve-Path -LiteralPath $ComponentDirectory).Path
$output=[IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw 'OutputDirectory must be new; existing output is never overwritten' }
foreach ($file in @('qing-shell.dll','qing-shell-host.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $component $file) -PathType Leaf)) { throw "Missing component: $file" }
}
foreach ($part in $Version.Split('.')) { if ([uint64]$part -gt 65535) { throw 'Version component exceeds 65535' } }
if (-not $Publisher.StartsWith('CN=') -or $Publisher.Contains("`n") -or $Publisher.Contains("`r")) { throw 'Use the exact signing certificate Subject as Publisher' }
$shell=Join-Path $output 'shell'
$packageInput=Join-Path $output 'identity-package'
New-Item -ItemType Directory -Path (Join-Path $shell 'Assets'),$packageInput | Out-Null
foreach ($file in @('qing-shell.dll','qing-shell-host.exe')) { Copy-Item -LiteralPath (Join-Path $component $file) -Destination (Join-Path $shell $file) }
Copy-Item -LiteralPath (Join-Path $repo 'obz-gui\frontend\app-icon.png') -Destination (Join-Path $shell 'Assets\AppIcon.png')
$encoding=New-Object Text.UTF8Encoding($false)
$publisherXml=[Security.SecurityElement]::Escape($Publisher)
function Expand-Manifest([string]$name) {
    (Get-Content -LiteralPath (Join-Path $PSScriptRoot $name) -Raw -Encoding UTF8).Replace('@PUBLISHER@',$publisherXml).Replace('@VERSION@',$Version).Replace('@ARCHITECTURE@',$Architecture)
}
$manifest=Expand-Manifest 'AppxManifest.xml.in'
$hostManifest=Expand-Manifest 'host.manifest.in'
[void][xml]$manifest
[void][xml]$hostManifest
[IO.File]::WriteAllText((Join-Path $packageInput 'AppxManifest.xml'),$manifest,$encoding)
$hostManifestPath=Join-Path $output 'host.manifest'
[IO.File]::WriteAllText($hostManifestPath,$hostManifest,$encoding)
& $ManifestToolPath -nologo '-manifest' $hostManifestPath "-outputresource:$(Join-Path $shell 'qing-shell-host.exe');#1"
if ($LASTEXITCODE -ne 0) { throw 'Embedding the sparse package identity failed' }
$payload=@{}
foreach ($file in @('qing-shell.dll','qing-shell-host.exe','Assets\AppIcon.png')) {
    $payload[$file]=(Get-FileHash -LiteralPath (Join-Path $shell $file) -Algorithm SHA256).Hash
}
# These hashes are inside the signed package, binding its external code/assets
# to the reviewed payload. Registration checks them before Add-AppxPackage.
[IO.File]::WriteAllText((Join-Path $packageInput 'external-payload.json'),($payload | ConvertTo-Json),$encoding)
$packagePath=Join-Path $output "qing-zip-shell-$Version-$Architecture.msix"
# Microsoft requires /nv for an external-location package: DLL/EXE/assets are
# deliberately external. The XML and required external payload were checked above.
& $MakeAppxPath pack /d $packageInput /p $packagePath /nv
if ($LASTEXITCODE -ne 0) { throw 'Sparse package creation failed' }
@{Name='QingYi.QingZip.Shell';Publisher=$Publisher;Version=$Version;Architecture=$Architecture;Signed=$false;ExternalSubdirectory='shell'} |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'package-info.json') -Encoding UTF8
Write-Output "Unsigned review package: $packagePath"
Write-Output 'Copy shell/ beside the installed qing-zip.exe only during an approved integration step. Sign the MSIX with an existing trusted certificate before registration.'
