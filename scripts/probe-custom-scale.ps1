param([Parameter(Mandatory)][string]$Receipt)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted') { throw 'Disposable hosted runner required' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$uia=[System.Windows.Automation.AutomationElement]; $scope=[System.Windows.Automation.TreeScope]
$report=[ordered]@{ status='started'; action='navigate only'; session_processes=@(); scale_controls=@(); custom_page_controls=@() }
try {
 $report.current_process=(Get-CimInstance Win32_Process -Filter "ProcessId=$PID" | Select-Object ProcessId,ParentProcessId,SessionId,Name)
 $report.runner_processes=@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^Runner\.(Listener|Worker)' } | Select-Object ProcessId,ParentProcessId,SessionId,Name)
 $report.sessions=(@(quser 2>&1 | ForEach-Object {"$_"}) -join "`n")
 Start-Process 'ms-settings:display'
 $condition=New-Object System.Windows.Automation.PropertyCondition -ArgumentList $uia::NameProperty,'Settings'
 $deadline=[DateTime]::UtcNow.AddSeconds(25)
 do {
  $window=@($uia::RootElement.FindAll($scope::Children,$condition) | Where-Object { !$_.Current.IsOffscreen }) | Select-Object -First 1
  if($window){break};Start-Sleep -Milliseconds 250
 } while([DateTime]::UtcNow -lt $deadline)
 if(!$window){throw 'Settings window absent'}
 $elements=@($window.FindAll($scope::Descendants,[System.Windows.Automation.Condition]::TrueCondition))
 $report.scale_controls=@($elements | Where-Object {$_.Current.Name -match 'scale|scal' -or $_.Current.AutomationId -match 'Scal'} | ForEach-Object { @{name=$_.Current.Name; id=$_.Current.AutomationId; type=$_.Current.ControlType.ProgrammaticName; enabled=$_.Current.IsEnabled; offscreen=$_.Current.IsOffscreen; patterns=@($_.GetSupportedPatterns() | ForEach-Object ProgrammaticName)} })
 $targets=@($elements | Where-Object { $_.Current.Name -eq 'Scale' -and $_.Current.IsEnabled -and !$_.Current.IsOffscreen })
 foreach($target in $targets){
  $invoke=$null
  if($target.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$invoke)){
   $invoke.Invoke();$report.navigation_invoked=$true;Start-Sleep -Seconds 2;break
  }
 }
 $elements=@($window.FindAll($scope::Descendants,[System.Windows.Automation.Condition]::TrueCondition))
 $report.custom_page_controls=@($elements | Where-Object {$_.Current.Name -match 'scal|sign out|restart|custom' -or $_.Current.AutomationId -match 'Scal'} | ForEach-Object { @{name=$_.Current.Name; id=$_.Current.AutomationId; type=$_.Current.ControlType.ProgrammaticName; enabled=$_.Current.IsEnabled; offscreen=$_.Current.IsOffscreen} })
 $report.status='observed'
} catch { $report.status='blocked'; $report.error=$_.Exception.Message }
finally { $report | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 -LiteralPath $Receipt }
