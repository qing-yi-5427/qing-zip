param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$OldApp,
    [Parameter(Mandatory)][string]$Msix,
    [Parameter(Mandatory)][string]$Publisher,
    [Parameter(Mandatory)][string]$PackageFullName,
    [Parameter(Mandatory)][string]$SignTool
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or $env:RUNNER_ARCH -cne 'ARM64') {
    throw 'Disposable hosted Windows ARM64 required'
}
$rootPath = (Resolve-Path -LiteralPath $Root).Path.TrimEnd('\')
$oldDirectory = Split-Path (Resolve-Path -LiteralPath $OldApp).Path -Parent
$newDirectory = Join-Path $rootPath 'relocated portable'
$probeDirectory = Join-Path $rootPath 'moved without registration'
$newApp = Join-Path $newDirectory 'qing-zip.exe'
$identity = 'QingYi.QingZip.Shell'
$settingsCommand = 'HKCU:\Software\Classes\*\shell\qing-zip\command'
$winps = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$report = [ordered]@{
    status = 'FAIL'; package_full_name = $PackageFullName; publisher = $Publisher
    signed_package_sha256 = (Get-FileHash -LiteralPath $Msix).Hash.ToLowerInvariant()
    phases = [ordered]@{}
}

function Test-ChildPath([string]$path) {
    [IO.Path]::GetFullPath($path).StartsWith($rootPath + '\', [StringComparison]::OrdinalIgnoreCase)
}

function Move-Portable([string]$from, [string]$to) {
    $resolved = (Resolve-Path -LiteralPath $from).Path
    if (!(Test-ChildPath $resolved) -or !(Test-ChildPath $to) -or (Test-Path -LiteralPath $to)) {
        throw 'Portable move must stay inside the fresh QA root and use an unused destination'
    }
    [IO.Directory]::Move($resolved, $to)
}

function Assert-Registration([string]$phase, [string]$expectedExternal) {
    $raw = & $winps -NoProfile -NonInteractive -File (Join-Path $PSScriptRoot 'read-external-registration.ps1') -PackageFullName $PackageFullName
    if ($LASTEXITCODE -ne 0) { throw "WinRT package query failed in $phase" }
    $registration = $raw | ConvertFrom-Json
    $catalog = @(Get-AppxPackage -Name $identity)
    $present = -not [string]::IsNullOrEmpty($expectedExternal)
    if ($present) {
        if (!$registration.present -or $catalog.Count -ne 1 -or
            $registration.full_name -cne $PackageFullName -or $registration.publisher -cne $Publisher -or
            $registration.user_external_path -ine $expectedExternal -or
            $registration.effective_external_path -ine $expectedExternal) {
            throw "WinRT registration location mismatch in $phase"
        }
    } elseif ($registration.present -or $catalog.Count) {
        throw "Sparse package remains registered in $phase"
    }
    $entry = [ordered]@{ winrt_present = $registration.present; current_user_catalog_count = $catalog.Count }
    if ($present) {
        $entry.user_external_path = '<test-root>' + $registration.user_external_path.Substring($rootPath.Length)
        $entry.effective_external_path = '<test-root>' + $registration.effective_external_path.Substring($rootPath.Length)
        $entry.external_directory_exists = Test-Path -LiteralPath $expectedExternal -PathType Container
    }
    $report.phases[$phase] = $entry
}

function Portable-Fingerprints([string]$base) {
    $items = Get-ChildItem -LiteralPath $base -File -Recurse | Sort-Object FullName
    $lines = @(foreach ($item in $items) {
        $relative = [IO.Path]::GetRelativePath($base, $item.FullName)
        (Get-FileHash -LiteralPath $item.FullName).Hash.ToLowerInvariant() + ' ' + $relative
    })
    return ,$lines
}

function Invoke-Ui([string]$phase, [string]$app) {
    python -u (Join-Path $PSScriptRoot 'hosted-explorer-review.py') --root $rootPath --app $app --lifecycle-phase $phase
    if ($LASTEXITCODE -ne 0) { throw "Explorer lifecycle observation failed in $phase" }
}

function Set-OpenMenu([string]$app, [bool]$enabled) {
    $cli = Join-Path (Split-Path $app -Parent) 'obz.exe'
    $verb = if ($enabled) { 'shell-enable' } else { 'shell-disable' }
    & $cli $verb $app
    if ($LASTEXITCODE) { throw "Product $verb failed" }
    $command = Get-Item -LiteralPath $settingsCommand -ErrorAction SilentlyContinue
    if ($enabled -and (!$command -or $command.GetValue('') -ine ('"' + $app + '" "%1"'))) {
        throw 'Open-menu setting does not belong to this portable executable'
    }
    if (!$enabled -and $command) { throw 'Product open-menu setting remains enabled' }
}

try {
    $oldShell = Join-Path $oldDirectory 'shell'
    Assert-Registration 'old_registered' $oldShell
    if (!$report.phases.old_registered.external_directory_exists) { throw 'Old shell directory missing' }
    $sentinel = 'ordinary portable user note'
    Set-Content -LiteralPath (Join-Path $oldDirectory 'portable-keep.txt') -Encoding utf8NoBOM -Value $sentinel
    $oldFiles = Portable-Fingerprints $oldDirectory
    $report.portable_file_count = $oldFiles.Count
    $report.portable_file_hashes_before = $oldFiles
    $report.without_reregister = [ordered]@{ move_succeeded = $false }
    try {
        try { Move-Portable $oldDirectory $probeDirectory; $report.without_reregister.move_succeeded = $true }
        catch { $report.without_reregister.move_error = $_.Exception.Message.Replace($rootPath, '<test-root>') }
        if ($report.without_reregister.move_succeeded) {
            Assert-Registration 'unsupported_move_kept_old_binding' $oldShell
            if ($report.phases.unsupported_move_kept_old_binding.external_directory_exists -or
                !(Test-Path -LiteralPath (Join-Path $probeDirectory 'qing-zip.exe') -PathType Leaf)) {
                throw 'Unsupported move did not produce a missing old target and intact moved application'
            }
            Invoke-Ui 'stale' (Join-Path $probeDirectory 'qing-zip.exe')
        }
    } finally {
        if ($report.without_reregister.move_succeeded) { Move-Portable $probeDirectory $oldDirectory }
    }
    Assert-Registration 'back_at_original_location' $oldShell

    & (Join-Path $PSScriptRoot 'Unregister-Package.ps1') -Publisher $Publisher
    Assert-Registration 'unregistered_before_supported_move' ''
    Set-OpenMenu $OldApp $false
    Move-Portable $oldDirectory $newDirectory
    if (Test-Path -LiteralPath $oldDirectory) { throw 'Old portable directory remained after move' }
    $newFiles = Portable-Fingerprints $newDirectory
    if (($oldFiles -join "`n") -cne ($newFiles -join "`n")) { throw 'Portable file bytes differ after move' }
    $report.portable_file_hashes_after_move_match = $true
    Assert-Registration 'moved_while_unregistered' ''

    $newShell = Join-Path $newDirectory 'shell'
    Set-OpenMenu $newApp $true
    & (Join-Path $PSScriptRoot 'Register-Package.ps1') -PackagePath $Msix -ExternalLocation $newShell -Publisher $Publisher -SignToolPath $SignTool
    Assert-Registration 'relocated_registered' $newShell
    if (!$report.phases.relocated_registered.external_directory_exists) { throw 'Relocated shell directory missing' }
    Invoke-Ui 'relocated' $newApp

    & (Join-Path $PSScriptRoot 'Unregister-Package.ps1') -Publisher $Publisher
    Assert-Registration 'finally_unregistered' ''
    $report.final_menu_checked_with_open_setting_still_enabled = (Get-Item -LiteralPath $settingsCommand).GetValue('') -ieq ('"' + $newApp + '" "%1"')
    if (!$report.final_menu_checked_with_open_setting_still_enabled) { throw 'Modern-menu absence needs an enabled open setting' }
    Invoke-Ui 'unregistered' $newApp
    Set-OpenMenu $newApp $false
    $finalFiles = Portable-Fingerprints $newDirectory
    if (($oldFiles -join "`n") -cne ($finalFiles -join "`n")) { throw 'Unregistration changed the portable payload' }
    if ((Get-FileHash -LiteralPath $Msix).Hash.ToLowerInvariant() -cne $report.signed_package_sha256) { throw 'Signed MSIX changed' }
    $report.portable_file_hashes_after_cleanup_match = $true
    $report.status = 'PASS'
} catch {
    $report.error = $_.Exception.Message.Replace($rootPath, '<test-root>')
    throw
} finally {
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath explorer-evidence/portable-lifecycle.json -Encoding utf8
}
