param([Parameter(Mandatory)][string]$Receipt)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') { throw 'Disposable hosted runner required' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$uia=[System.Windows.Automation.AutomationElement]; $scope=[System.Windows.Automation.TreeScope]
$report=[ordered]@{ status='started'; action='navigate only'; navigation_invoked=$false; custom_page_observed=$false; runner_processes=@(); scale_controls=@(); custom_page_controls=@() }
try {
 $report.current_process=(Get-CimInstance Win32_Process -Filter "ProcessId=$PID" | Select-Object ProcessId,ParentProcessId,SessionId,Name)
 $report.runner_processes=@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^Runner\.(Listener|Worker)' } | Select-Object ProcessId,ParentProcessId,SessionId,Name)
 $report.sessions=(@(quser 2>&1 | ForEach-Object {"$_"}) -join "`n")
 Start-Process 'ms-settings:display'
 $condition=New-Object System.Windows.Automation.PropertyCondition -ArgumentList $uia::AutomationIdProperty,'SystemSettings_Display_Scaling_ItemSizeOverride_EntityItem'
 $deadline=[DateTime]::UtcNow.AddSeconds(25)
 do {
  $window=@($uia::RootElement.FindAll($scope::Children,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object { !$_.Current.IsOffscreen -and $_.FindFirst($scope::Descendants,$condition) }) | Select-Object -First 1
  if($window){break};Start-Sleep -Milliseconds 250
 } while([DateTime]::UtcNow -lt $deadline)
 if(!$window){throw 'Visible display scale control absent'}
 $elements=@($window.FindAll($scope::Descendants,[System.Windows.Automation.Condition]::TrueCondition))
 $report.scale_controls=@($elements | Where-Object {$_.Current.Name -match 'scale|scal' -or $_.Current.AutomationId -match 'Scal'} | ForEach-Object { @{name=$_.Current.Name; id=$_.Current.AutomationId; type=$_.Current.ControlType.ProgrammaticName; enabled=$_.Current.IsEnabled; offscreen=$_.Current.IsOffscreen; patterns=@($_.GetSupportedPatterns() | ForEach-Object ProgrammaticName)} })
 $group=$window.FindFirst($scope::Descendants,$condition)
 $buttonCondition=New-Object System.Windows.Automation.PropertyCondition -ArgumentList $uia::AutomationIdProperty,'EntityItemButton'
 $target=$group.FindFirst($scope::Descendants,$buttonCondition)
 if(!$target -or !$target.Current.IsEnabled -or $target.Current.IsOffscreen) { throw 'Scale navigation button absent or disabled' }
 $invoke=$null
 if(!$target.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$invoke)){throw 'Scale navigation does not support Invoke'}
 $invoke.Invoke();$report.navigation_invoked=$true
 $customCondition=New-Object System.Windows.Automation.PropertyCondition -ArgumentList $uia::AutomationIdProperty,'SystemSettings_Display_Scaling_CustomOverride_ValueTextBox'
 $deadline=[DateTime]::UtcNow.AddSeconds(10)
 do { $custom=$window.FindFirst($scope::Descendants,$customCondition);if($custom -and !$custom.Current.IsOffscreen){break};Start-Sleep -Milliseconds 200 } while([DateTime]::UtcNow -lt $deadline)
 if(!$custom -or $custom.Current.IsOffscreen){throw 'Custom scaling page marker absent'}
 $report.custom_page_observed=$true
 $elements=@($window.FindAll($scope::Descendants,[System.Windows.Automation.Condition]::TrueCondition))
 $report.custom_page_controls=@($elements | Where-Object {$_.Current.Name -match 'scal|sign out|restart|custom' -or $_.Current.AutomationId -match 'Scal'} | ForEach-Object { @{name=$_.Current.Name; id=$_.Current.AutomationId; type=$_.Current.ControlType.ProgrammaticName; enabled=$_.Current.IsEnabled; offscreen=$_.Current.IsOffscreen} })
 $report.status='observed'
} catch { $report.status='blocked'; $report.error=$_.Exception.Message }
finally { $report | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 -LiteralPath $Receipt }
