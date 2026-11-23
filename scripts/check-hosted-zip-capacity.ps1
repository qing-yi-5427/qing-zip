param(
    [Parameter(Mandatory)]
    [ValidateSet('baseline', 'combined')]
    [string]$Trial,
    [ValidatePattern('^[0-9]{1,20}$')]
    [string]$BuildRun
)
$ErrorActionPreference = 'Stop'
if (!$IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or $env:RUNNER_ARCH -cne 'X64' -or $env:GITHUB_REPOSITORY -cne 'qing-yi-5427/qing-zip') {
    throw 'Requires a disposable GitHub-hosted Windows x64 job'
}

$repo = 'qing-yi-5427/qing-zip'
$runs = @{
    baseline = @{ id = '45100520858'; source = '51a68cc32bafcd535eac6cef8eee5169a54adcbe' }
    combined = @{ id = '45116763549'; source = 'e9eeecd414fa7379fc959967fb1645ced24eeb62' }
}
$trialRun = $runs[$Trial]
if ($BuildRun) { $trialRun.id = $BuildRun }
$tempRoot = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).Path
if ($tempRoot -notmatch '^[A-Za-z]:\\[^"\r\n]+$') { throw 'Expected local runner temp path' }
$marker = [guid]::NewGuid().ToString('N')
$label = 'QZ-' + $marker.Substring(0, 16)
$job = (New-Item -ItemType Directory (Join-Path $tempRoot "qz-capacity-$marker")).FullName
$evidence = (New-Item -ItemType Directory (Join-Path $job 'evidence')).FullName
$artifactName = 'qing-zip-windows-x64-trial'
$image = Join-Path $job 'volume.vhd'
$capacity = 96MB
$mounted = $false
$context = @{
    status = 'blocked'; trial = $Trial; run = $trialRun.id; product_source = $trialRun.source
    harness = (git -C $PSScriptRoot rev-parse HEAD).Trim()
    runner = $env:RUNNER_OS; arch = $env:RUNNER_ARCH; system = [Environment]::OSVersion.Version.ToString()
    stage = 'artifact'; detached = $false; backing_removed = $false
}

function Write-Receipt {
    $context | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $evidence 'isolated-vhd.json') -Encoding utf8
}

function Read-Run {
    $json = gh api "repos/$repo/actions/runs/$($trialRun.id)"
    if ($LASTEXITCODE) { throw 'Could not read pinned artifact run' }
    $json | ConvertFrom-Json
}

function Get-OwnedDisk {
    $attachedImage = Get-DiskImage -ImagePath $image
    if (!$attachedImage.Attached -or $attachedImage.ImagePath -ne $image -or $attachedImage.Size -ne $capacity) {
        throw 'Mounted image is not the exact bounded backing file'
    }
    $matches = @($attachedImage | Get-Disk)
    if ($matches.Count -ne 1 -or $matches[0].Number -in $preexistingDiskNumbers -or
        $matches[0].IsBoot -or $matches[0].IsSystem -or $matches[0].Size -ne $capacity) {
        throw 'Backing image did not resolve to one new non-system disk of the expected capacity'
    }
    $matches[0]
}

Write-Receipt
try {
    $run = Read-Run
    for ($wait = 0; $run.status -ne 'completed' -and $wait -lt 16; $wait++) {
        Start-Sleep -Seconds 30
        $run = Read-Run
    }
    if ($run.head_sha -ne $trialRun.source -or $run.conclusion -ne 'success' -or $run.name -ne 'Windows regression') {
        throw 'Pinned Windows regression build is unavailable or source does not match'
    }
    $artifactJson = gh api "repos/$repo/actions/runs/$($trialRun.id)/artifacts"
    if ($LASTEXITCODE) { throw 'Could not read pinned artifact metadata' }
    $artifacts = @(($artifactJson | ConvertFrom-Json).artifacts | Where-Object { $_.name -ceq $artifactName -and !$_.expired })
    if ($artifacts.Count -ne 1 -or $artifacts[0].digest -notmatch '^sha256:[0-9a-f]{64}$') {
        throw 'Pinned hardened artifact missing or ambiguous'
    }
    $context.artifact = @{ id = $artifacts[0].id; digest = $artifacts[0].digest }
    $download = Join-Path $job 'download'
    gh run download $trialRun.id --repo $repo --name $artifactName --dir $download
    if ($LASTEXITCODE) { throw 'Hardened artifact download failed' }
    $packages = @(Get-ChildItem -LiteralPath $download -Recurse -File -Filter '*-portable.zip')
    $builds = @(Get-ChildItem -LiteralPath $download -Recurse -File -Filter 'hardened-build.json')
    if ($packages.Count -ne 1 -or $builds.Count -ne 1) { throw 'Hardened artifact missing or ambiguous files' }
    $context.portable_sha256 = (Get-FileHash -LiteralPath $packages[0].FullName).Hash.ToLowerInvariant()
    $stage = Join-Path $job 'stage'
    Expand-Archive -LiteralPath $packages[0].FullName -DestinationPath $stage
    $build = Get-Content -LiteralPath $builds[0].FullName -Raw | ConvertFrom-Json
    if ($build.target -ne 'x86_64-pc-windows-msvc') { throw 'Unexpected artifact target' }
    $context.binaries = [ordered]@{}
    foreach ($name in @('obz.exe', 'obz-worker.exe', 'obz-gui.exe')) {
        $entry = @($build.binaries | Where-Object name -ceq $name)
        $packagedName = if ($name -eq 'obz-gui.exe') { 'qing-zip.exe' } else { $name }
        $digest = (Get-FileHash -LiteralPath (Join-Path $stage $packagedName)).Hash.ToLowerInvariant()
        if ($entry.Count -ne 1 -or $entry[0].sha256 -ne $digest) { throw "Hardened binary hash mismatch for $name" }
        $context.binaries[$packagedName] = $digest
    }

    $context.stage = 'mount'
    Write-Receipt
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Runner is not an administrator' }
    $tempDrive = [System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot($tempRoot))
    if ($tempDrive.AvailableFreeSpace -lt 2GB) { throw 'Insufficient runner backing volume headroom' }
    if (Test-Path -LiteralPath $image) { throw 'Backing image unexpectedly exists' }
    $createScript = Join-Path $job 'create-vhd.txt'
    ('create vdisk file="{0}" type=fixed maximum=96' -f $image) | Set-Content -LiteralPath $createScript -Encoding ascii
    & (Join-Path $env:SystemRoot 'System32/diskpart.exe') /s $createScript | Out-File -LiteralPath (Join-Path $job 'diskpart.log')
    $length = (Get-Item -LiteralPath $image).Length
    if ($LASTEXITCODE -or $length -ne ($capacity + 512)) { throw 'DiskPart did not create the expected fixed VHD' }
    $context.vhd_backing_bytes = $length

    $preexistingDiskNumbers = @((Get-Disk).Number)
    $preexistingDriveLetters = @((Get-Volume).DriveLetter)
    Mount-DiskImage -ImagePath $image -NoDriveLetter
    $mounted = $true
    $disk = Get-OwnedDisk
    if ($disk.PartitionStyle -ne 'RAW') { throw 'Refusing to initialize any non-RAW disk' }
    $partition = $disk | Initialize-Disk -PartitionStyle MBR -PassThru | New-Partition -UseMaximumSize -AssignDriveLetter
    $letter = [string]$partition.DriveLetter
    if ($letter -notmatch '^[D-Z]$' -or $letter -in $preexistingDriveLetters -or
        "$letter`:" -in @($env:SystemDrive, $tempDrive.Name.TrimEnd('\')) -or
        (Get-OwnedDisk).Number -ne $partition.DiskNumber -or (Get-Partition -DriveLetter $letter).DiskNumber -ne $partition.DiskNumber) {
        throw 'Assigned drive is not the new verified VHD partition'
    }
    $volume = Format-Volume -Partition $partition -FileSystem NTFS -NewFileSystemLabel $label -AllocationUnitSize 4096 -Confirm:$false
    if ($volume.FileSystem -ne 'NTFS' -or $volume.FileSystemLabel -ne $label -or $volume.Size -lt 80MB -or
        $volume.Size -gt $capacity -or (Get-Partition -DriveLetter $letter).DiskNumber -ne (Get-OwnedDisk).Number) {
        throw 'Formatted partition does not match verified private VHD'
    }
    $context.volume_bytes = $volume.Size
    Set-Content -LiteralPath (Join-Path ('{0}:\' -f $letter) 'qz-volume-owner') -Value $marker -Encoding ascii -NoNewline
    $context.stage = 'product'
    Write-Receipt
    python (Join-Path $PSScriptRoot 'check-hosted-zip-capacity.py') $job $letter $label $marker $trialRun.source
    $productExit = $LASTEXITCODE
    $productReceipt = Join-Path $evidence 'zip-space-receipt.json'
    if (Test-Path -LiteralPath $productReceipt) {
        $context.status = (Get-Content -LiteralPath $productReceipt -Raw | ConvertFrom-Json).status
    }
    if ($productExit -ne 0 -or $context.status -ne 'passed') { throw 'ZIP disk capacity assertions did not all pass; inspect per-case receipt' }
}
catch {
    $context.error_line = $_.InvocationInfo.ScriptLineNumber
    $context.error = $_.Exception.Message.Replace($job, '<qa-root>')
    throw
}
finally {
    try {
        if (Test-Path -LiteralPath $image) {
            if ($mounted -or (Get-DiskImage -ImagePath $image).Attached) {
                Dismount-DiskImage -ImagePath $image
            }
            if ((Get-DiskImage -ImagePath $image).Attached) { throw 'Backing file is still attached' }
            $context.detached = $true
            Remove-Item -LiteralPath $image
            $context.backing_removed = !(Test-Path -LiteralPath $image)
        }
    }
    catch {
        $context.cleanup_error = $_.Exception.Message.Replace($job, '<qa-root>')
        if ($context.status -eq 'passed') { $context.status = 'blocked' }
        throw
    }
    finally { Write-Receipt }
}
