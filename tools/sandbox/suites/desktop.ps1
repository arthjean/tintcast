. C:\tintcast\lib.ps1

$dwm = 'Software\Microsoft\Windows\DWM'
$personalize = 'reg:HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'

function Same-Watched($a, $b) {
    ($a.watched | ConvertTo-Json -Compress -Depth 6) -eq ($b.watched | ConvertTo-Json -Compress -Depth 6)
}

function Value-Json($report, [string]$key) {
    Watched $report $key | ConvertTo-Json -Compress -Depth 4
}

function Dword($report, [string]$key) {
    $value = Watched $report $key
    if ($value -and $value.type -eq 'dword') { [int64]$value.data } else { $null }
}

$variant = Get-Content -Raw (Join-Path $root 'variant.json') | ConvertFrom-Json
$solid = '#1e3c5a'
Add-Type -Namespace Tintcast -Name Desk -MemberDefinition @'
[DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern bool SystemParametersInfo(int action, int param, string value, int flags);
[DllImport("user32.dll", SetLastError = true)]
public static extern bool SetSysColors(int count, int[] elements, int[] colors);
'@

function Set-SolidBackground {
    $wallpapers = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers'
    if (-not (Test-Path $wallpapers)) { New-Item -Path $wallpapers -Force | Out-Null }
    Set-ItemProperty -Path $wallpapers -Name BackgroundType -Value 1 -Type DWord
    Set-ItemProperty -Path 'HKCU:\Control Panel\Colors' -Name Background -Value '30 60 90'
    [void][Tintcast.Desk]::SetSysColors(1, [int[]]@(1), [int[]]@(30 -bor (60 -shl 8) -bor (90 -shl 16)))
    $cleared = [Tintcast.Desk]::SystemParametersInfo(0x14, 0, '', 3)
    Log "solid background 30 60 90, wallpaper cleared: $cleared"
}

function Background-Type {
    (Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers' -ErrorAction SilentlyContinue).BackgroundType
}

function Set-DwmDeny([bool]$deny) {
    $rights = [System.Security.AccessControl.RegistryRights]'ChangePermissions, ReadKey'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($dwm, [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree, $rights)
    $acl = $key.GetAccessControl()
    $user = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
    $rule = New-Object System.Security.AccessControl.RegistryAccessRule($user, 'SetValue', 'Deny')
    if ($deny) { $acl.AddAccessRule($rule) } else { [void]$acl.RemoveAccessRule($rule) }
    $key.SetAccessControl($acl)
    $key.Close()
    Log "DWM key SetValue deny for ${user}: $deny"
}

try {
    Log 'start'
    Start-Sleep -Seconds 20
    if ($variant.solid) {
        Set-SolidBackground
        Start-Sleep -Seconds 3
    }
    Start-Process 'ms-settings:colors'
    Start-Sleep -Seconds 8
    Shot '00-initial' 0
    $initial = Report '00-initial'
    $kind = $initial.info.'background-kind'
    Log "build $($initial.build), background $kind, monitors $($initial.info.monitors)"
    if ($variant.solid) {
        $walls = @($initial.watched.PSObject.Properties | Where-Object Name -Like 'wallpaper:*' | ForEach-Object { $_.Value })
        $seen = Pixel 1700 600
        Check 'solid-start' ($kind -eq 'solid-color' -and @($walls | Where-Object { $_ }).Count -eq 0 -and $seen -eq $solid) "background $kind, desktop $seen, wallpapers [$($walls -join ', ')], BackgroundType $(Background-Type)"
    }

    $list = Tintcast list
    Check 'list' ($list.Code -eq 0 -and $list.Out -match 'Ironwood  \(ironwood, dark, 3 wallpapers, bundled\)') "exit $($list.Code)"

    $plan = Tintcast plan ironwood
    $afterPlan = Report '01-after-plan'
    Check 'plan-writes-nothing' ((Same-Watched $initial $afterPlan) -and $plan.Code -eq 0) 'watched values unchanged by plan'
    Check 'plan-lists-changes' ($plan.Out -match '(?m)^Mode and accent$' -and $plan.Out -match 'Set AccentPalette to 32 bytes' -and $plan.Out -match '(?m)^Wallpaper$' -and $plan.Out -match 'Nothing was changed\.') 'preview names mode, accent, and wallpaper changes'
    Check 'plan-never-color-prevalence' ($plan.Out -notmatch 'ColorPrevalence') 'ColorPrevalence is not planned'
    if ($kind -eq 'spotlight' -or $kind -eq 'slideshow') {
        $word = if ($kind -eq 'spotlight') { 'Windows Spotlight background' } else { 'slideshow' }
        Check 'plan-background-note' ($plan.Out.Contains("Your $word stays off after Restore; Restore brings back the picture shown now.")) "background kind $kind"
    }
    if ($kind -eq 'solid-color') {
        Check 'plan-solid-note' ($plan.Out.Contains('Set Background to "30 60 90"') -and $plan.Out.Contains('Restore brings back your solid color.')) 'the preview records the solid color'
    }

    $ironwood = Tintcast apply ironwood
    Check-Applied $ironwood 'ironwood'
    Shot '02-ironwood'
    $report = Report '02-ironwood'
    Check-Accent 'ironwood'
    Check 'ironwood-dark' ((Dword $report "$personalize\AppsUseLightTheme") -eq 0 -and (Dword $report "$personalize\SystemUsesLightTheme") -eq 0) 'apps and Windows in dark mode'
    $wallpapers = @($report.watched.PSObject.Properties | Where-Object Name -Like 'wallpaper:*' | ForEach-Object { $_.Value })
    Check 'ironwood-wallpaper' (@($wallpapers | Where-Object { $_ -match '\\Tintcast\\wallpapers\\ironwood-[0-9a-f]{16}\.png$' }).Count -eq $wallpapers.Count -and $wallpapers.Count -ge 1) ($wallpapers -join ', ')
    Check 'color-prevalence-kept' ((Value-Json $report "$personalize\ColorPrevalence") -eq (Value-Json $initial "$personalize\ColorPrevalence")) 'ColorPrevalence unchanged'

    $daylight = Tintcast apply daylight --wallpaper 2
    Check-Applied $daylight 'daylight'
    Shot '03-daylight'
    $report = Report '03-daylight'
    Check-Accent 'daylight'
    Check 'daylight-light' ((Dword $report "$personalize\AppsUseLightTheme") -eq 1 -and (Dword $report "$personalize\SystemUsesLightTheme") -eq 1) 'apps and Windows in light mode'

    $undo = Tintcast undo
    Check 'undo-reapplies-ironwood' ($undo.Code -eq 0 -and $undo.Out -match '^Applied Ironwood in') "exit $($undo.Code)"
    $accentKey = 'reg:HKCU\Software\Microsoft\Windows\DWM\AccentColor'
    $afterUndo = Report '03b-after-undo'
    Start-Sleep -Seconds 3
    $afterUndoLater = Report '03c-after-undo-3s'
    Check 'undo-dwm-holds' ((Dword $afterUndo $accentKey) -eq (Dword $afterUndoLater $accentKey) -and (Dword $afterUndoLater $accentKey) -ne (Dword $initial $accentKey)) ('DWM AccentColor {0} right after Undo, {1} 3 s later, {2} at start' -f (Dword $afterUndo $accentKey), (Dword $afterUndoLater $accentKey), (Dword $initial $accentKey))
    $status = Tintcast status --json
    Check 'status-current' (($status.Out | ConvertFrom-Json).current.theme_id -eq 'ironwood') 'status shows Ironwood'

    $restore = Tintcast restore
    Check 'restore' ($restore.Code -eq 0 -and $restore.Out -match '^Restored ') "exit $($restore.Code)"
    Shot '04-restored'
    $restored = Report '04-restored'
    Check 'restore-exact' (Same-Watched $initial $restored) 'watched values equal the initial report'
    if ($variant.solid) {
        $seen = Pixel 1700 600
        Check 'solid-restored' ($seen -eq $solid) "desktop shows $seen after Restore, BackgroundType $(Background-Type)"
    }

    Set-DwmDeny $true
    $denied = Tintcast apply inkwell
    Set-DwmDeny $false
    Check 'denied-exit' ($denied.Code -eq 1) "exit $($denied.Code)"
    Check 'denied-message' ($denied.Out -match 'Mode and accent: Failed\. .*Windows blocked the change to (AccentColor|ColorizationColor|ColorizationAfterglow)\. Tintcast rolled back this part\.') 'names the blocked value and rolls back'
    Check 'denied-wallpaper-kept' ($denied.Out -match '(?m)^Wallpaper: Applied$') 'the wallpaper still applies'
    $afterDenied = Report '05-denied'
    $same = $true
    foreach ($property in $initial.watched.PSObject.Properties) {
        if ($property.Name -like 'reg:*') {
            if ((Value-Json $afterDenied $property.Name) -ne ($property.Value | ConvertTo-Json -Compress -Depth 4)) { $same = $false; Log "after denied, $($property.Name) differs" }
        }
    }
    Check 'denied-rolled-back' $same 'every registry value is back to its original'

    $final = Tintcast restore
    Check 'final-restore' ($final.Code -eq 0) "exit $($final.Code)"
    Shot '99-final'
    Report '99-final' | Out-Null
}
catch {
    Log ('suite error: {0}' -f $_)
    Check 'suite' $false "$_"
}
finally {
    Finish
}
