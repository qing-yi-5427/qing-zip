param([Parameter(Mandatory)][string]$PackageFullName)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
    throw 'Hosted Explorer QA only'
}
$manager = [Windows.Management.Deployment.PackageManager, Windows.Management.Deployment, ContentType=WindowsRuntime]::new()
try {
    $package = $manager.FindPackageForUser('', $PackageFullName)
} catch {
    $errorDetail = $_.Exception
    while ($errorDetail.InnerException) { $errorDetail = $errorDetail.InnerException }
    if (('{0:X8}' -f $errorDetail.HResult) -ne '80073CF1') { throw }
    $package = $null
}
if ($null -eq $package) {
    @{ present = $false } | ConvertTo-Json -Compress
} else {
    [ordered]@{
        present = $true
        full_name = $package.Id.FullName
        publisher = $package.Id.Publisher
        user_external_path = $package.UserExternalPath
        effective_external_path = $package.EffectiveExternalPath
    } | ConvertTo-Json -Compress
}
