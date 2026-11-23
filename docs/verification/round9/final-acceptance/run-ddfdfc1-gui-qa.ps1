param(
    [Parameter(Mandatory)][string]$Repo,
    [Parameter(Mandatory)][string]$Stage,
    [Parameter(Mandatory)][string]$FixtureRoot,
    [Parameter(Mandatory)][string]$Out
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$env:PYTHONUTF8 = '1'

$head = 'ddfdfc11db6b8e7dd5caa2113642cf56ecb59284'
$run = 'https://github.com/qing-yi-5427/qing-zip/actions/runs/45273082510'
$repo = (Resolve-Path -LiteralPath $Repo).Path
$fixtureRoot = (Resolve-Path -LiteralPath $FixtureRoot).Path
$source = Get-Content -LiteralPath (Join-Path $Stage 'checked-source.json') -Raw | ConvertFrom-Json
if ($source.pr_head -ne $head -or $source.run -ne $run -or $head -notin $source.commit_parents) {
    throw 'The staged artifact receipt does not identify the requested source and run.'
}

$zip = (Resolve-Path -LiteralPath (Join-Path $Stage 'portable.zip')).Path
$metadata = (Resolve-Path -LiteralPath (Join-Path $Stage 'hardened-build.json')).Path
$exe = (Resolve-Path -LiteralPath (Join-Path $Stage 'staged/qing-zip-0.12.23-portable/qing-zip.exe')).Path
if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $source.portable_sha256) {
    throw 'The portable ZIP differs from the receipt.'
}

$checkScripts = @(
    'scripts/check-batch-crash-desktop.py',
    'scripts/check-batch-resume-desktop.py',
    'scripts/check-batch-extract-desktop.py',
    'scripts/check-nested-multi-layer.py'
)
git -C $repo diff --quiet $head -- @checkScripts
if ($LASTEXITCODE -ne 0 -or (git -C $repo rev-parse HEAD) -ne $head) {
    throw 'Use an unchanged checkout of the fixed source revision.'
}
if (Get-Process -Name qing-zip,obz-gui,obz,obz-worker -ErrorAction SilentlyContinue) {
    throw 'A qing-zip process is already running.'
}
if (Test-Path -LiteralPath $Out) {
    throw 'Use a new evidence directory.'
}

New-Item -ItemType Directory -Path $Out | Out-Null
$out = (Resolve-Path -LiteralPath $Out).Path
Copy-Item -LiteralPath (Join-Path $Stage 'checked-source.json') -Destination $out
$common = @('--exe', $exe, '--fixtures-in', $fixtureRoot, '--source', "$head / $run",
            '--package', $zip, '--hardening', $metadata)
$tests = [ordered]@{
    'crash-fresh' = @('--expect-recovery', '--scenario', 'fresh')
    'crash-resumed' = @('--expect-recovery', '--scenario', 'resumed')
    'crash-last' = @('--expect-recovery', '--scenario', 'last')
    'crash-partial' = @('--expect-recovery', '--scenario', 'partial')
    'normal-continue' = @()
    'normal-overwrite-review' = @('--overwrite-review')
    'normal-write-failure' = @('--write-failure')
    'normal-record-recovery' = @('--record-recovery')
    'normal-exit-while-saving' = @('--exit-while-saving')
}
$results = foreach ($case in $tests.GetEnumerator()) {
    $script = if ($case.Key.StartsWith('crash-')) { $checkScripts[0] } else { $checkScripts[1] }
    $receipt = Join-Path $out "$($case.Key)/receipt.json"
    $flags = @($case.Value)
    python (Join-Path $repo $script) @common @flags --out (Join-Path $out $case.Key) *> (Join-Path $out "$($case.Key).log")
    $exitCode = $LASTEXITCODE
    $passed = $exitCode -eq 0 -and (Test-Path -LiteralPath $receipt)
    Write-Host "$($case.Key): $passed (exit $exitCode)"
    [ordered]@{ case = $case.Key; passed = $passed; exit_code = $exitCode }
    if (Get-Process -Name qing-zip,obz-gui,obz,obz-worker -ErrorAction SilentlyContinue) {
        Write-Warning 'An application process remains. Stopping before the next case.'
        break
    }
}
[ordered]@{
    source = $source
    source_tree = (git -C $repo rev-parse "$head^{tree}")
    windows = [Environment]::OSVersion.VersionString
    architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    cases = @($results)
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $out 'summary.json') -Encoding utf8
if (@($results).Count -ne $tests.Count -or ($results | Where-Object { -not $_.passed })) {
    exit 1
}
