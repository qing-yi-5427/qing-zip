# Exercise a layout outside the checkout with only the native CLI and reviewed runtime.
param([Parameter(Mandatory)][string]$Cli,[Parameter(Mandatory)][string]$Desktop,[Parameter(Mandatory)][string]$Layout)
$ErrorActionPreference = 'Stop'
$Worker = Join-Path (Split-Path $Cli -Parent) 'obz-worker.exe'
$apps = @($Cli, $Desktop, $Worker)
$report = python scripts/audit_pe_security.py @apps | ConvertFrom-Json
if ($LASTEXITCODE -or $report.Count -ne 3 -or @($report | Where-Object machine -ne '0xaa64').Count) {
    throw 'incorrect CLI, worker or GUI PE machine'
}
$report | Select-Object path,machine,bits | Format-Table
Get-FileHash $apps | Format-Table
if (Test-Path -LiteralPath $Layout) { throw 'native probe requires a fresh directory' }
New-Item -ItemType Directory -Path (Join-Path $Layout 'runtime') | Out-Null
Copy-Item -LiteralPath $Cli, $Worker -Destination $Layout
Copy-Item -LiteralPath runtime/nanazip-arm64 -Destination (Join-Path $Layout 'runtime') -Recurse
$executable = Join-Path $Layout 'obz.exe'
$cap = & $executable capabilities | ConvertFrom-Json
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$want = (Resolve-Path (Join-Path $Layout 'runtime/nanazip-arm64/NanaZip.Universal.Console.exe')).Path
if ([IO.Path]::GetFullPath($cap.engine.engine) -ne $want -or $cap.engine.backend -ne 'NanaZip') {
    throw "incorrect runtime: $($cap.engine.engine)"
}
$inputDir = Join-Path $Layout 'input'
New-Item -ItemType Directory -Path $inputDir | Out-Null
$random = [Random]::new(99871)
foreach ($name in 'keep.bin','discard.bin') {
    $bytes = [byte[]]::new(65536)
    $random.NextBytes($bytes)
    [IO.File]::WriteAllBytes((Join-Path $inputDir $name), $bytes)
}
$archive = Join-Path $Layout 'split.7z'
Push-Location $inputDir
try {
    & $want a -t7z -mx=0 -ms=off -v32k $archive keep.bin discard.bin
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} finally { Pop-Location }
$env:OPEN_ARCHIVE_PROFILE = '1'
$dest = Join-Path $Layout 'out'
$trace = & $executable extract "$archive.001" $dest --include keep.bin 2>&1 | Out-String
$exitCode = $LASTEXITCODE
Write-Host $trace
if ($exitCode -or $trace -notmatch 'OA_SELECTED_WORKER status=ok' -or $trace -notmatch 'decoded=131072 selected=65536 discarded=65536') {
    throw 'isolated CLI did not use the native full-verification worker'
}
if (@(Get-ChildItem -LiteralPath $dest -Recurse -File).Count -ne 1 -or
    (Get-FileHash (Join-Path $dest 'keep.bin')).Hash -ne (Get-FileHash (Join-Path $inputDir 'keep.bin')).Hash) {
    throw 'isolated selected output differs from source'
}
Write-Host "Verified isolated native runtime: $want"

# A complete ordinary archive uses the separately shipped helper. Exercise it
# outside the checkout as well as the selected split mode above.
$single = Join-Path $Layout 'single.zip'
Push-Location $inputDir
try {
    & $want a -tzip -mx=0 $single keep.bin discard.bin
    if ($LASTEXITCODE) { exit $LASTEXITCODE }
} finally { Pop-Location }
$singleDest = Join-Path $Layout 'single-out'
$trace = & $executable extract $single $singleDest --threads 4 2>&1 | Out-String
if ($LASTEXITCODE -or $trace -notmatch 'OA_FULL_WORKER threads_applied=4' -or $trace -notmatch 'OA_FULL_WORKER decoded=131072 completed=2') {
    throw "shipped native helper did not complete the approved archive: $trace"
}
foreach ($name in 'keep.bin','discard.bin') {
    if ((Get-FileHash (Join-Path $singleDest $name)).Hash -ne (Get-FileHash (Join-Path $inputDir $name)).Hash) {
        throw 'native helper output differs from source'
    }
}
