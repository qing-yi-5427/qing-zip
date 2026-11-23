param(
    [Parameter(Mandatory=$true)][string]$Receipt,
    [Parameter(Mandatory=$true)][string]$Screenshot,
    [ValidateRange(0,500)][int]$SelectPercent = 0
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'Display settings test requires a disposable GitHub-hosted runner'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
$uia = [System.Windows.Automation.AutomationElement]
$scope = [System.Windows.Automation.TreeScope]
$control = [System.Windows.Automation.ControlType]
$selectionPattern = [System.Windows.Automation.SelectionPattern]::Pattern
$expandPattern = [System.Windows.Automation.ExpandCollapsePattern]::Pattern
$selectItem = [System.Windows.Automation.SelectionItemPattern]::Pattern
$info = [ordered]@{ requested = $SelectPercent; action = 'probe'; settings_visible = $false; combos = @() }
$expanded = $null

function Condition($property, $value) {
    New-Object System.Windows.Automation.PropertyCondition -ArgumentList $property, $value
}

function Percent([string]$name) {
    if ($name -match '^([0-9]{2,3})\s*%(?:$|\s)') { return [int]$Matches[1] }
    return 0
}

function SettingsScreenshot($window) {
    $rect = $window.Current.BoundingRectangle
    if ($rect.IsEmpty -or $rect.Width -lt 100 -or $rect.Height -lt 100) { return }
    $bitmap = New-Object System.Drawing.Bitmap ([int]$rect.Width), ([int]$rect.Height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$rect.X, [int]$rect.Y, 0, 0, $bitmap.Size)
        $bitmap.Save($Screenshot, [System.Drawing.Imaging.ImageFormat]::Png)
    } catch {
        $info.screenshot_error = $_.Exception.Message
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

try {
    Start-Process 'ms-settings:display'
    $root = $uia::RootElement
    $byName = Condition $uia::NameProperty 'Settings'
    $byCombo = Condition $uia::ControlTypeProperty $control::ComboBox
    $byScale = Condition $uia::NameProperty 'Scale'
    $byLayout = Condition $uia::NameProperty 'Scale & layout'
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        $windows = @($root.FindAll($scope::Children, $byName))
        foreach ($candidate in $windows) {
            if (!$candidate.Current.IsOffscreen -and ($candidate.FindFirst($scope::Descendants, $byScale) -or
                $candidate.FindFirst($scope::Descendants, $byLayout))) {
                $window = $candidate
                break
            }
        }
        if ($window) { break }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)
    $info.settings_window_count = $windows.Count
    if (!$window) { throw 'The visible System > Display page did not appear in UI Automation' }
    $info.settings_visible = $true
    $combos = @($window.FindAll($scope::Descendants, $byCombo))
    $scale = @()
    foreach ($combo in $combos) {
        $selection = $null
        $label = ''
        if ($combo.TryGetCurrentPattern($selectionPattern, [ref]$selection)) {
            $label = ($selection.Current.GetSelection() | ForEach-Object { $_.Current.Name }) -join ' '
        }
        $name = $combo.Current.Name
        $id = $combo.Current.AutomationId
        $info.combos += @{ name = $name; id = $id; selection = $label; enabled = $combo.Current.IsEnabled }
        if ($name -match '^Scale(?:\W|$)' -or $id -match 'Scale' -or (Percent $label)) {
            $scale += @{ element = $combo; initial = Percent $label }
        }
    }
    if ($scale.Count -ne 1 -or !$scale[0].element.Current.IsEnabled) {
        SettingsScreenshot $window
        throw 'The standard Scale combo is missing, disabled, or ambiguous'
    }
    $combo = $scale[0].element
    $info.current = $scale[0].initial
    $expanded = $combo.GetCurrentPattern($expandPattern)
    $expanded.Expand()
    $byListItem = Condition $uia::ControlTypeProperty $control::ListItem
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $items = @($combo.FindAll($scope::Descendants, $byListItem) | Where-Object { Percent $_.Current.Name })
        if (!$items) {
            $items = @($window.FindAll($scope::Descendants, $byListItem) | Where-Object { Percent $_.Current.Name })
        }
        if ($items) { break }
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    $info.options = @($items | ForEach-Object {
        @{ label = $_.Current.Name; percent = Percent $_.Current.Name; enabled = $_.Current.IsEnabled }
    })
    if (!$info.current) {
        foreach ($item in $items) {
            $selected = $null
            if ($item.TryGetCurrentPattern($selectItem, [ref]$selected) -and $selected.Current.IsSelected) {
                $info.current = Percent $item.Current.Name
            }
        }
    }
    SettingsScreenshot $window
    if (!$info.current -or !$items) { throw 'Scale selection or its options were not observable' }
    if ($SelectPercent) {
        $matches = @($items | Where-Object { $_.Current.IsEnabled -and (Percent $_.Current.Name) -eq $SelectPercent })
        if ($matches.Count -ne 1) { throw 'Requested enabled Scale option was not unique' }
        $info.action = 'select'
        $matches[0].GetCurrentPattern($selectItem).Select()
        $info.selection_invoked = $true
    }
} catch {
    $info.error = $_.Exception.Message
} finally {
    if ($expanded) {
        try { $expanded.Collapse() } catch { $info.collapse_error = $_.Exception.Message }
    }
    $info | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Receipt -Encoding UTF8
}
