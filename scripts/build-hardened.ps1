[CmdletBinding()]
param(
    [ValidateSet('x86_64-pc-windows-msvc','aarch64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$TargetDirectory,
    [string]$CargoPath = 'cargo',
    [string]$PythonPath = 'python',
    [switch]$RunTests
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $projectRoot 'target\hardened' }
if (-not [IO.Path]::IsPathRooted($TargetDirectory)) {
    $TargetDirectory = Join-Path $projectRoot $TargetDirectory
}
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
$binaryDirectory = Join-Path $TargetDirectory "$Target\release"
$auditPath = Join-Path $binaryDirectory 'pe-security.json'

# These environment variables take precedence over target.rustflags even with
# --config. Reject ambiguous builds; do not change the host environment, process
# mitigation policy, certificate store or third-party runtime binaries.
foreach ($name in @('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS')) {
    if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($name))) {
        throw "$name overrides the hardened target configuration. Run this script from an environment without that override."
    }
}

$rustFlags = @(
    '-C', 'control-flow-guard=yes',
    '-C', 'link-arg=/CETCOMPAT',
    '-C', 'link-arg=/DYNAMICBASE',
    '-C', 'link-arg=/HIGHENTROPYVA',
    '-C', 'link-arg=/NXCOMPAT'
)
$peMachine = '0x8664'
if ($Target -eq 'aarch64-pc-windows-msvc') {
    $peMachine = '0xaa64'
    # CET shadow stacks are an x86/x64 PE capability.
    $rustFlags = @('-C','control-flow-guard=yes','-C','link-arg=/DYNAMICBASE','-C','link-arg=/HIGHENTROPYVA','-C','link-arg=/NXCOMPAT')
}
$flagArray = ConvertTo-Json -InputObject $rustFlags -Compress
$configPath = Join-Path $TargetDirectory 'hardening-config.toml'
# Explicit --target keeps flags off host build scripts and proc macros. The
# separate output tree prevents accidentally packaging an ordinary build.
$commonArguments = @('--config', $configPath)
$targetArguments = @('--locked', '--target', $Target, '--target-dir', $TargetDirectory)

function Invoke-HardenedNative {
    param([string]$Command, [string[]]$Arguments, [string]$Failure)
    Get-Command $Command -ErrorAction Stop | Out-Null
    $savedPreference = $ErrorActionPreference
    try {
        # Windows PowerShell 5.1 turns redirected native stderr (including
        # cargo's normal progress messages) into ErrorRecords. Collect those
        # messages without treating them as build failures; use the exit code.
        $ErrorActionPreference = 'Continue'
        & $Command @Arguments 2>&1 | ForEach-Object { Write-Host ([string]$_) }
        $nativeExitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $savedPreference }
    if ($nativeExitCode -ne 0) { throw "$Failure (exit $nativeExitCode)" }
}

Push-Location $projectRoot
try {
    # A real TOML file also avoids Windows PowerShell 5.1 stripping embedded
    # quotes when an inline array is passed to a native cargo.exe command.
    [IO.Directory]::CreateDirectory($TargetDirectory) | Out-Null
    [IO.File]::WriteAllText($configPath, "[target.$Target]`nrustflags=$flagArray`n", [Text.UTF8Encoding]::new($false))
    foreach ($manifest in @('obz-rs/Cargo.toml', 'obz-gui/Cargo.toml')) {
        Invoke-HardenedNative -Command $CargoPath -Arguments ($commonArguments + @('build', '--release') + $targetArguments + @('--manifest-path', $manifest)) -Failure "Hardened build failed: $manifest"
    }
    if ($RunTests) {
        foreach ($manifest in @('obz-rs/Cargo.toml', 'obz-gui/Cargo.toml')) {
            Invoke-HardenedNative -Command $CargoPath -Arguments ($commonArguments + @('test') + $targetArguments + @('--manifest-path', $manifest)) -Failure "Hardened tests failed: $manifest"
        }
    }
    $corePath = Join-Path $binaryDirectory 'obz.exe'
    $guiPath = Join-Path $binaryDirectory 'obz-gui.exe'
    Invoke-HardenedNative -Command $PythonPath -Arguments @((Join-Path $PSScriptRoot 'audit_pe_security.py'), '--require-hardened', '--expect-machine', $peMachine, '--output', $auditPath, $corePath, $guiPath) -Failure 'Authored executable PE mitigation gate failed'
    # Build evidence does not assert identical coverage for prebuilt dependencies
    # or active hardware enforcement on the current computer.
    $receiptPath = Join-Path $binaryDirectory 'hardened-build.json'
    $receipt = [ordered]@{
        target = $Target
        binary_directory = $binaryDirectory
        rustflags = $rustFlags
        config = $configPath
        audit = $auditPath
        tests_run = [bool]$RunTests
        binaries = @($corePath, $guiPath | ForEach-Object {
            [ordered]@{ name = [IO.Path]::GetFileName($_); sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant() }
        })
        coverage = 'Authored CLI and GUI PE metadata; third-party runtimes unchanged; hardware enforcement not asserted.'
    }
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $receiptPath -Encoding UTF8
    [pscustomobject]@{
        Target = $Target
        TargetDirectory = $TargetDirectory
        BinaryDirectory = $binaryDirectory
        AuditPath = $auditPath
        ReceiptPath = $receiptPath
    }
} finally { Pop-Location }
