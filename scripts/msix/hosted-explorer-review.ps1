$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or $env:RUNNER_ARCH -cne 'ARM64' -or $env:GITHUB_REPOSITORY -cne 'qing-yi-5427/qing-zip' -or [Environment]::OSVersion.Version.Build -lt 22000) { throw 'Disposable hosted Windows 11 ARM64 required' }
New-Item -ItemType Directory -Force explorer-evidence | Out-Null
$receipt = @{ stage='start'; registration='not attempted'; unregistered=$false; certificates_removed=$false; source=(git rev-parse HEAD).Trim() }
function Save-Receipt { $receipt | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 explorer-evidence/setup.json }
$cert=$null; $thumb=$null; $publisher='CN=QingZip Disposable Explorer QA'; $job=Join-Path $env:RUNNER_TEMP ('qz-explorer-'+[guid]::NewGuid().ToString('N'))
Save-Receipt
try {
  if (@(Get-AppxPackage -Name 'QingYi.QingZip.Shell').Count) { throw 'Existing product identity; refusing to change it' }
  $run=gh api repos/qing-yi-5427/qing-zip/actions/runs/45119972417 | ConvertFrom-Json
  if ($LASTEXITCODE -or $run.head_sha -cne 'b2bf9f3f4eaddcbfefd3144ad0fdb8769fa1824b' -or $run.conclusion -cne 'success') { throw 'Unexpected product build' }
  gh run download 45119972417 --repo qing-yi-5427/qing-zip --name qing-zip-0.12.23-windows-arm64-trial --dir (Join-Path $job 'download')
  if ($LASTEXITCODE) { throw 'Package download failed' }
  $package=@(Get-ChildItem (Join-Path $job 'download') -Recurse -File -Filter '*-arm64-portable.zip')
  if ($package.Count -ne 1 -or (Get-FileHash $package[0].FullName).Hash.ToLowerInvariant() -cne '39b494dcbff1b3afa20b4c85bcb0ddb4de5da017fcee3c056593daab038fc10e') { throw 'Package hash mismatch' }
  $receipt.portable_sha256='39b494dcbff1b3afa20b4c85bcb0ddb4de5da017fcee3c056593daab038fc10e'; $receipt.product_source=$run.head_sha
  $stage=Join-Path $job 'stage'; Expand-Archive $package[0].FullName $stage
  $app=@(Get-ChildItem $stage -Recurse -File -Filter qing-zip.exe); if ($app.Count -ne 1) { throw 'Expected one app' }
  $build=Join-Path $job 'component'; ./scripts/msix/Build-Component.ps1 -Architecture ARM64 -BuildDirectory $build
  $kits=(Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Windows Kits/Installed Roots').KitsRoot10
  $sdk=Get-ChildItem (Join-Path $kits 'bin') -Directory | Where-Object Name -match '^10\.0\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'arm64'; Join-Path $_.FullName 'x64' } | Where-Object { (Test-Path "$_/makeappx.exe") -and (Test-Path "$_/mt.exe") -and (Test-Path "$_/signtool.exe") } | Select-Object -First 1
  if (!$sdk) { throw 'SDK packaging tools missing' }
  $review=Join-Path $job 'review'; ./scripts/msix/Prepare-Package.ps1 -ComponentDirectory "$build/Release" -OutputDirectory $review -Publisher $publisher -Version '0.12.23.0' -Architecture arm64 -MakeAppxPath "$sdk/makeappx.exe" -ManifestToolPath "$sdk/mt.exe"
  Copy-Item -LiteralPath (Join-Path $review 'shell') -Destination $app[0].DirectoryName -Recurse
  $cert=New-SelfSignedCertificate -Type CodeSigningCert -Subject $publisher -CertStoreLocation Cert:/CurrentUser/My -NotAfter (Get-Date).AddDays(1)
  $thumb=$cert.Thumbprint; $receipt.thumbprint=$thumb
  $certPath=Join-Path $job 'temporary.cer'; Export-Certificate -Cert $cert -FilePath $certPath | Out-Null
  Import-Certificate -FilePath $certPath -CertStoreLocation Cert:/CurrentUser/TrustedPeople | Out-Null
  Import-Certificate -FilePath $certPath -CertStoreLocation Cert:/LocalMachine/Root | Out-Null
  $msix=Join-Path $review 'qing-zip-shell-0.12.23.0-arm64.msix'
  & "$sdk/signtool.exe" sign /fd SHA256 /sha1 $thumb /s My $msix; if ($LASTEXITCODE) { throw 'Signing failed' }
  $receipt.msix_sha256=(Get-FileHash $msix).Hash.ToLowerInvariant(); $receipt.stage='register'; Save-Receipt
  ./scripts/msix/Register-Package.ps1 -PackagePath $msix -ExternalLocation (Join-Path $app[0].DirectoryName 'shell') -Publisher $publisher -SignToolPath "$sdk/signtool.exe"
  $pkg=@(Get-AppxPackage -Name 'QingYi.QingZip.Shell'); if ($pkg.Count -ne 1 -or $pkg[0].Publisher -cne $publisher) { throw 'Registration missing' }
  $receipt.registration='passed'; $receipt.package_full_name=$pkg[0].PackageFullName; $receipt.stage='explorer'; Save-Receipt
  python -m pip install pywinauto==0.6.9 pillow==11.3.0; if ($LASTEXITCODE) { throw 'UI dependencies unavailable' }
  python -u scripts/msix/hosted-explorer-review.py --root $job --app $app[0].FullName
  if ($LASTEXITCODE) { throw 'Explorer observation did not pass; inspect evidence' }
  $receipt.stage='completed'
} catch { $receipt.error=$_.Exception.Message.Replace($job,'<test-root>'); throw }
finally {
  try { ./scripts/msix/Unregister-Package.ps1 -Publisher $publisher; $receipt.unregistered=@(Get-AppxPackage -Name 'QingYi.QingZip.Shell').Count -eq 0 } catch { $receipt.unregister_error=$_.Exception.Message }
  try {
    if ($thumb) { foreach ($store in @('Cert:/CurrentUser/My','Cert:/CurrentUser/TrustedPeople','Cert:/LocalMachine/Root')) { $entry=Join-Path $store $thumb; if (Test-Path $entry) { Remove-Item -LiteralPath $entry -Force } }; $receipt.certificates_removed=!(Test-Path "Cert:/CurrentUser/My/$thumb") -and !(Test-Path "Cert:/CurrentUser/TrustedPeople/$thumb") -and !(Test-Path "Cert:/LocalMachine/Root/$thumb") } else { $receipt.certificates_removed=$true }
  } catch { $receipt.certificate_error=$_.Exception.Message }
  Save-Receipt
  if (!$receipt.unregistered -or !$receipt.certificates_removed) { throw 'Disposable runner cleanup not verified' }
}
