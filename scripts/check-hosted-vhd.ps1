[CmdletBinding()]
param([Parameter(Mandatory)][ValidateSet("FAT32","exFAT")][string]$FileSystem)
$ErrorActionPreference="Stop"
if($env:GITHUB_ACTIONS -cne "true" -or $env:RUNNER_ENVIRONMENT -cne "github-hosted" -or $env:RUNNER_OS -cne "Windows"){throw "Disposable hosted Windows only"}
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$nonce=[Guid]::NewGuid().ToString("N")
$dir=Join-Path $temp ("qing-vhd-"+$nonce)
if(Test-Path -LiteralPath $dir){throw "Workspace already exists"}
New-Item -ItemType Directory -Path $dir | Out-Null
$imageFile=[IO.Path]::GetFullPath((Join-Path $dir ("test-"+$nonce+".vhd")))
if([IO.Path]::GetDirectoryName($imageFile) -cne $dir -or [IO.Path]::GetDirectoryName($dir) -cne $temp -or (Test-Path -LiteralPath $imageFile)){throw "Unexpected VHD path"}
$existing=@((Get-Disk).Number)
$receipt=[Collections.Generic.List[object]]::new()
function Verified-Disk([string]$Operation){
    $image=@(Get-DiskImage -ImagePath $imageFile)
    if($image.Count -ne 1 -or !$image[0].Attached -or [IO.Path]::GetFullPath($image[0].ImagePath) -ine $imageFile){throw "Image backing mismatch before $Operation"}
    $disk=@($image[0] | Get-Disk)
    if($disk.Count -ne 1){throw "Expected one disk for exact VHD before $Operation"}
    $disk=$disk[0]
    $reverse=@(Get-DiskImage -DevicePath $image[0].DevicePath)
    if($reverse.Count -ne 1 -or [IO.Path]::GetFullPath($reverse[0].ImagePath) -ine $imageFile -or !$reverse[0].Attached){throw "Reverse backing mismatch before $Operation"}
    if($disk.Number -in $existing -or $disk.IsBoot -or $disk.IsSystem -or $disk.IsClustered -or $disk.BusType.ToString() -ne "File Backed Virtual" -or ($disk.Size -lt 2139095040 -or $disk.Size -gt 2147483648) -or $image[0].DevicePath -ine ("\\.\PhysicalDrive"+$disk.Number)){throw "Unsafe target before $Operation"}
    $receipt.Add([pscustomobject]@{operation=$Operation;image=$imageFile;reverse=$reverse[0].ImagePath;disk=$disk.Number;device=$image[0].DevicePath;bus=$disk.BusType.ToString();size=$disk.Size;was_existing=$false;boot=$disk.IsBoot;system=$disk.IsSystem})
    return $disk
}
function Verified-Partition([string]$operation){
    $disk=Verified-Disk $operation
    $p=@(Get-Partition -DiskNumber $disk.Number | Where-Object PartitionNumber -eq $script:partNumber)
    if($p.Count -ne 1 -or $p[0].DiskNumber -ne $disk.Number -or $p[0].Offset -ne $script:partOffset -or $p[0].Size -ne $script:partLength){throw "Unexpected VHD partition before $operation"}
    return $p[0]
}
# DiskPart receives only a CREATE VDISK instruction naming a new file. It never
# receives a physical disk selector, partition, format, clean or remove command.
$create=Join-Path $dir 'create.txt'
[IO.File]::WriteAllText($create,("create vdisk file=`""+$imageFile+"`" maximum=2048 type=expandable"+[Environment]::NewLine+"exit"+[Environment]::NewLine),[Text.Encoding]::ASCII)
$mounted=$false
try {
    & diskpart.exe /s $create
    if($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $imageFile -PathType Leaf)){throw "Could not create scratch VHD"}
    Mount-DiskImage -ImagePath $imageFile -StorageType VHD -NoDriveLetter | Out-Null
    $mounted=$true
    $disk=Verified-Disk "initialize"
    if($disk.PartitionStyle -ne "RAW"){throw "New disk is not RAW"}
    Initialize-Disk -Number $disk.Number -PartitionStyle MBR | Out-Null
    $disk=Verified-Disk "partition"
    if($disk.PartitionStyle -ne "MBR" -or $disk.NumberOfPartitions -ne 0){throw "Disk already has a partition"}
    $occupied=@((Get-Volume).DriveLetter)+@((Get-PSDrive -PSProvider FileSystem).Name)
    $letter=@([char[]]"RSTUVWXYZ" | Where-Object { $_ -notin $occupied })[0]
    if(!$letter){throw "No unused drive letter on the isolated runner"}
    $part=New-Partition -DiskNumber $disk.Number -UseMaximumSize -DriveLetter $letter
    $script:partNumber=$part.PartitionNumber
    $script:partOffset=$part.Offset
    $script:partLength=$part.Size
    $part=Verified-Partition "format"
    $label="QZ"+$nonce.Substring(0,6).ToUpperInvariant()
    Format-Volume -Partition $part -FileSystem $FileSystem -NewFileSystemLabel $label -Confirm:$false | Out-Null
    $part=Verified-Partition "test"
    for($i=0;$i -lt 12;$i++){
        $vol=$part | Get-Volume
        if($part.DriveLetter -eq $letter -and $vol.FileSystem -ieq $FileSystem -and $vol.FileSystemLabel -ceq $label){break}
        Start-Sleep -Milliseconds 500
        $part=Verified-Partition "await volume metadata"
    }
    if($part.DriveLetter -ne $letter -or $vol.FileSystem -ine $FileSystem -or $vol.FileSystemLabel -cne $label){
        throw "Formatted scratch mismatch: drive=$($part.DriveLetter) expected=$letter fs=$($vol.FileSystem) label=$($vol.FileSystemLabel) expected=$label"
    }
    $again=@(Get-Partition -DriveLetter $part.DriveLetter)
    if($again.Count -ne 1 -or $again[0].DiskNumber -ne $part.DiskNumber -or $again[0].PartitionNumber -ne $part.PartitionNumber){throw "Drive letter does not address the verified image"}
    $path=Join-Path ($part.DriveLetter+":\") ("case-"+$nonce)
    New-Item -ItemType Directory -Path $path | Out-Null
    $env:QING_VHD_ROOT=$path
    $env:QING_VHD_FS=$FileSystem
    $env:QING_VHD_LABEL=$label
    python -X utf8 -u scripts/check-hosted-fat-archives.py
    if($LASTEXITCODE -ne 0){throw "Compatibility harness failed"}
    $tools=Get-Content -Raw fat-evidence/tools.json | ConvertFrom-Json
    if($tools.candidate.asset_id){
        python -X utf8 -u scripts/check-archive-edit-contention.py $tools.candidate.cli --root $path --kill --output (Join-Path "fat-evidence" ("contention-"+$FileSystem+".json"))
        if($LASTEXITCODE -ne 0){throw "Archive edit contention or forced termination failed"}
    }
} finally {
    if($mounted){Dismount-DiskImage -ImagePath $imageFile | Out-Null}
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8NoBOM (Join-Path 'fat-evidence' ("backing-"+$FileSystem+".json"))
}