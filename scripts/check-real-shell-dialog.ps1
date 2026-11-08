param([int]$ApplicationPid,[string]$Exe,[string]$Probe,[string]$First,[string]$Second,[string]$Screenshot)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted'){throw 'Hosted fixture required'}
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Drawing,WindowsBase
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class WindowFixture {
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int n);
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr i);
}
"@
[void][WindowFixture]::SetProcessDPIAware()
$automation=[System.Windows.Automation.AutomationElement]
$scope=[System.Windows.Automation.TreeScope]::Descendants
function Wait-Value($name,[scriptblock]$test){
  $end=[DateTime]::UtcNow.AddSeconds(30)
  do { $v=& $test; if($v){return $v};Start-Sleep -Milliseconds 180 }while([DateTime]::UtcNow -lt $end)
  throw "Timed out waiting for $name"
}
function Find-Elem($value,[string]$property='Name',[object]$parent=$script:root) {
  $key=if($property -eq 'ID'){$automation::AutomationIdProperty}else{$automation::NameProperty}
  $condition=[System.Windows.Automation.PropertyCondition]::new($key,$value)
  return $parent.FindFirst($scope,$condition)
}
function Click-Item($element) {
  $scroll=$null
  if($element.TryGetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern,[ref]$scroll)){ $scroll.ScrollIntoView() }
  $click=$null
  if($element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$click)){ $click.Invoke();return }
  if($element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern,[ref]$click)){ $click.Toggle();return }
  $element.SetFocus()
  $b=$element.Current.BoundingRectangle
  if($element.Current.IsOffscreen -or $b.Width -le 0 -or $b.Height -le 0){throw 'No on-screen click location'}
  [void][WindowFixture]::SetCursorPos([int]($b.X+$b.Width/2),[int]($b.Y+$b.Height/2))
  [WindowFixture]::mouse_event(2,0,0,0,[UIntPtr]::Zero);[WindowFixture]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
try{
  $app=Get-Process -Id $ApplicationPid
  $hwnd=Wait-Value 'main window' {$app.Refresh();if($app.HasExited){throw 'GUI exited'};if($app.MainWindowHandle -ne [IntPtr]::Zero){$app.MainWindowHandle}}
  [void][WindowFixture]::ShowWindow($hwnd,3)
  [void][WindowFixture]::SetForegroundWindow($hwnd)
  $script:root=$automation::FromHandle($hwnd)
  Click-Item (Wait-Value 'settings' {Find-Elem '设置'})
  Click-Item (Wait-Value 'file compression switch' {Find-Elem 'oa-file-menu' 'ID'})
  $verb='Registry::HKEY_CURRENT_USER\Software\Classes\*\shell\qing-zip-compress-files\command'
  [void](Wait-Value 'file compression verb' {Test-Path -LiteralPath $verb})
  Click-Item (Wait-Value 'files' {Find-Elem '文件'})
  $evidence= & $Probe $Exe $First $Second $ApplicationPid | ConvertFrom-Json
  if($LASTEXITCODE -ne 0 -or $evidence.real_primary_pid -ne $ApplicationPid -or $evidence.invoked_for_files -ne 2){throw 'Shell invocation failed'}
  $dialog=Wait-Value 'creation dialog' {Find-Elem '新建压缩文件'}
  $dialogName=$dialog.Current.Name
  $dialogCondition=[System.Windows.Automation.PropertyCondition]::new($automation::AriaRoleProperty,'dialog')
  $dialogs=$script:root.FindAll($scope,$dialogCondition)
  if($dialogs.Count -ne 1){throw "Expected one live modal dialog; got $($dialogs.Count)"}
  $modal=$dialogs.Item(0)
  $count=Wait-Value '2 sources' { Find-Elem '2 个来源' }
  $rows=@(foreach($path in @($First,$Second)){
    $item=Wait-Value "source $path" {Find-Elem $path 'Name' $modal}
    if($item.Current.IsOffscreen){throw "Source offscreen: $path"}
    $item.Current.Name
  })
  $target=Wait-Value 'output' {Find-Elem 'qa-save' 'ID'}
  $output=$target.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
  if($output -ne (Join-Path (Split-Path $First) 'inputs.zip')){throw "Unexpected output: $output"}
  $checkbox=Wait-Value 'verify after creation' {Find-Elem 'oa-test-after' 'ID'}
  if($checkbox.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne 'On'){throw 'Verification is disabled'}
  $bounds=$script:root.Current.BoundingRectangle
  $bitmap=[Drawing.Bitmap]::new([int]$bounds.Width,[int]$bounds.Height)
  $g=[Drawing.Graphics]::FromImage($bitmap)
  try{$g.CopyFromScreen([int]$bounds.X,[int]$bounds.Y,0,0,$bitmap.Size);$bitmap.Save($Screenshot,[Drawing.Imaging.ImageFormat]::Png)}finally{$g.Dispose();$bitmap.Dispose()}
  Click-Item (Wait-Value 'submit' {Find-Elem '创建压缩文件'})
  [void](Wait-Value 'post-create integrity result' {Find-Elem '完整性检查通过'})
  $close=Wait-Value 'close result' {Find-Elem '关闭'}
  Click-Item $close
  Start-Sleep -Milliseconds 900
  if(Find-Elem 'oa-source-count' 'ID'){throw 'Another creation dialog opened'}
  [ordered]@{primary_pid=$ApplicationPid;dialog_count=$dialogs.Count;title=$dialogName;sources=$rows;output=$output;format='zip';created_with_integrity_check=$true} | ConvertTo-Json -Compress
}catch{
  if($script:root){
    $list=$script:root.FindAll($scope,[System.Windows.Automation.Condition]::TrueCondition)
    $state=foreach($e in $list){if($e.Current.Name -or $e.Current.AutomationId){$e.Current.ControlType.ProgrammaticName+' '+$e.Current.AutomationId+' '+$e.Current.Name}}
    [Console]::Error.WriteLine(($state|Select-Object -First 130) -join [Environment]::NewLine)
  }
  throw
}
