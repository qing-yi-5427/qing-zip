[CmdletBinding(SupportsShouldProcess)]
param([Parameter(Mandatory)][string]$Publisher)
$ErrorActionPreference='Stop'
foreach ($package in @(Get-AppxPackage -Name 'QingYi.QingZip.Shell')) {
    if ($package.Publisher -cne $Publisher) { throw 'Refusing to remove a package from a different publisher' }
    if ($PSCmdlet.ShouldProcess($package.PackageFullName,'Unregister qing-zip Windows 11 context menu for the current user')) {
        Remove-AppxPackage -Package $package.PackageFullName
    }
}
