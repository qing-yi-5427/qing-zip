[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackagePath,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$CertificateThumbprint,
    [Parameter(Mandatory)][string]$Publisher,
    [Parameter(Mandatory)][ValidateScript({
        $uri = $null
        [Uri]::TryCreate($_, [UriKind]::Absolute, [ref]$uri) -and
            $uri.Scheme -in @('http','https') -and $uri.Host -and
            -not $uri.UserInfo -and -not $uri.Fragment -and $_ -notmatch '[\x00-\x20]'
    })][string]$TimestampServer,
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
# /tw makes an absent timestamp a warning; SignTool returns 2 for warnings.
# Reject that status as well as errors, even if the signature itself is valid.
& $SignToolPath verify /pa /all /tw $package
if ($LASTEXITCODE -ne 0) { throw 'MSIX signature or timestamp verification failed' }
