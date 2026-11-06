[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)][string]$PackagePath,
    [Parameter(Mandatory)][string]$ExternalLocation,
    [Parameter(Mandatory)][string]$Publisher,
    [string]$SignToolPath='signtool.exe'
)
$ErrorActionPreference='Stop'
$package=(Resolve-Path -LiteralPath $PackagePath).Path
$external=(Resolve-Path -LiteralPath $ExternalLocation).Path.TrimEnd('\')
if ((Split-Path $external -Leaf) -cne 'shell') { throw 'ExternalLocation must be the dedicated shell subdirectory, never the whole app directory' }
foreach ($file in @('qing-shell.dll','qing-shell-host.exe','Assets\AppIcon.png')) {
    if (-not (Test-Path -LiteralPath (Join-Path $external $file) -PathType Leaf)) { throw "Missing external component: $file" }
}
if (-not (Test-Path -LiteralPath (Join-Path (Split-Path $external -Parent) 'qing-zip.exe') -PathType Leaf)) { throw 'Parent qing-zip.exe is missing' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
& $SignToolPath verify /pa $package
if ($LASTEXITCODE -ne 0) { throw 'Package signature is not trusted; no certificate or trust setting was changed' }
$zip=[IO.Compression.ZipFile]::OpenRead($package)
try {
    $entry=$zip.GetEntry('AppxManifest.xml')
    if (-not $entry -or $entry.Length -gt 65536) { throw 'Invalid package manifest' }
    $settings=New-Object Xml.XmlReaderSettings
    $settings.DtdProcessing=[Xml.DtdProcessing]::Prohibit
    $settings.XmlResolver=$null
    $settings.MaxCharactersInDocument=65536
    $settings.CloseInput=$true
    $reader=[Xml.XmlReader]::Create($entry.Open(),$settings)
    try { $manifest=New-Object Xml.XmlDocument; $manifest.XmlResolver=$null; $manifest.Load($reader) } finally { $reader.Dispose() }
    if ($manifest.Package.Identity.Name -cne 'QingYi.QingZip.Shell' -or $manifest.Package.Identity.Publisher -cne $Publisher) { throw 'Package identity does not match this product and publisher' }
    $hashEntry=$zip.GetEntry('external-payload.json')
    if (-not $hashEntry -or $hashEntry.Length -gt 8192) { throw 'Missing or invalid signed external payload hashes' }
    $hashReader=New-Object IO.StreamReader($hashEntry.Open(),[Text.Encoding]::UTF8)
    try { $hashes=$hashReader.ReadToEnd() | ConvertFrom-Json } finally { $hashReader.Dispose() }
    $expectedFiles=@('qing-shell.dll','qing-shell-host.exe','Assets\AppIcon.png')
    if (@($hashes.PSObject.Properties).Count -ne $expectedFiles.Count) { throw 'Unexpected external payload manifest' }
    foreach ($file in $expectedFiles) {
        $expected=$hashes.PSObject.Properties[$file].Value
        if ($expected -notmatch '^[0-9A-Fa-f]{64}$' -or (Get-FileHash -LiteralPath (Join-Path $external $file) -Algorithm SHA256).Hash -ine $expected) { throw "External payload does not match the signed package: $file" }
    }
} finally { $zip.Dispose() }
if ($PSCmdlet.ShouldProcess($external,'Register qing-zip Windows 11 context menu for the current user')) {
    Add-AppxPackage -Path $package -ExternalLocation $external
    Get-AppxPackage -Name 'QingYi.QingZip.Shell' | Select-Object Name,PackageFullName,Publisher,Status
}
