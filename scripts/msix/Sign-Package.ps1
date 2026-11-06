[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackagePath,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$CertificateThumbprint,
    [Parameter(Mandatory)][string]$Publisher,
    [Parameter(Mandatory)][ValidatePattern('^https://')][string]$TimestampServer,
    [string]$SignToolPath='signtool.exe'
)
$ErrorActionPreference='Stop'
$package=(Resolve-Path -LiteralPath $PackagePath).Path
$certificate=Get-Item -LiteralPath "Cert:\CurrentUser\My\$CertificateThumbprint"
if (-not $certificate.HasPrivateKey -or $certificate.Subject -cne $Publisher) { throw 'Certificate private key or exact Publisher match is missing' }
if ($certificate.NotAfter -lt (Get-Date) -or $certificate.NotBefore -gt (Get-Date)) { throw 'Signing certificate is not currently valid' }
$chain=New-Object Security.Cryptography.X509Certificates.X509Chain
try { if (-not $chain.Build($certificate)) { throw 'Certificate is not trusted. This script will not add trust or import certificates.' } } finally { $chain.Dispose() }
& $SignToolPath sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampServer /td SHA256 $package
if ($LASTEXITCODE -ne 0) { throw 'MSIX signing failed' }
& $SignToolPath verify /pa $package
if ($LASTEXITCODE -ne 0) { throw 'MSIX signature verification failed' }
