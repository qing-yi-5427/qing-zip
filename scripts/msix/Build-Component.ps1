[CmdletBinding()]
param(
    [ValidateSet('x64','ARM64')][string]$Architecture='x64',
    [string]$BuildDirectory,
    [string]$CmakePath='cmake.exe'
)
$ErrorActionPreference='Stop'
$repo=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
if (-not $BuildDirectory) { $BuildDirectory=Join-Path $repo 'target\qing-shell' }
# Run from a matching MSVC developer environment. No tool downloads, registry
# registration, Explorer restarts, certificate generation or installation.
& $CmakePath -S (Join-Path $repo 'qing-shell') -B $BuildDirectory -A $Architecture
if ($LASTEXITCODE -ne 0) { throw 'Shell component configuration failed' }
& $CmakePath --build $BuildDirectory --config Release
if ($LASTEXITCODE -ne 0) { throw 'Shell component build failed' }
& (Join-Path $BuildDirectory 'Release\qing-shell-args-tests.exe') (Join-Path $BuildDirectory 'Release\qing-shell-argv-fixture.exe')
if ($LASTEXITCODE -ne 0) { throw 'Shell argument quoting test failed' }
& (Join-Path $BuildDirectory 'Release\qing-shell-com-tests.exe') (Join-Path $BuildDirectory 'Release\qing-shell.dll')
if ($LASTEXITCODE -ne 0) { throw 'Isolated COM contract test failed' }
& (Join-Path $BuildDirectory 'Release\qing-shell-selection-tests.exe')
if ($LASTEXITCODE -ne 0) { throw 'Filesystem selection test failed' }
Write-Output (Join-Path $BuildDirectory 'Release')
