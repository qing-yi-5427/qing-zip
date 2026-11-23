[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Portable,
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')] [string]$Target = 'x86_64-pc-windows-msvc',
    [Parameter(Mandatory)] [string]$TargetDirectory
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$portablePath = (Resolve-Path -LiteralPath $Portable).Path
$targetPath = (Resolve-Path -LiteralPath $TargetDirectory).Path
$config = Join-Path $targetPath 'hardening-config.toml'
$worker = Join-Path $targetPath "$Target/release/obz-worker.exe"
$workerHash = (Get-FileHash -LiteralPath $worker -Algorithm SHA256).Hash
$testName = 'cancel_selected_zip_verification_rolls_back_the_selected_output'
$saved = $env:QING_TEST_SELECTED_ZIP_PORTABLE
try {
    $env:QING_TEST_SELECTED_ZIP_PORTABLE = $portablePath
    $result = (cargo --config $config test --release --locked --target $Target --target-dir $targetPath `
        --manifest-path (Join-Path $root 'obz-rs/Cargo.toml') --test direct_extraction $testName -- --exact --nocapture `
        2>&1 | Out-String)
    $code = $LASTEXITCODE
} finally {
    $env:QING_TEST_SELECTED_ZIP_PORTABLE = $saved
}
Write-Output $result
if ($code -or $result -notmatch 'test result: ok\. 1 passed; 0 failed;') {
    throw 'Expected exactly one passing packaged cancellation test'
}
$witness = [regex]::Match($result, 'QING_SELECTED_ZIP_CANCEL_PASS worker=([0-9a-f]{64}) reads=(\d+)')
$reads = [int]$witness.Groups[2].Value
if (!$witness.Success -or $witness.Groups[1].Value -ine $workerHash -or
    $reads -le 0 -or $reads -ge 3000) {
    throw 'The selected ZIP test did not witness the packaged hardened worker using bounded reads'
}
Write-Output "PASS $Target packaged worker $workerHash, read operations $reads"
