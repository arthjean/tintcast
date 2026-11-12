. C:\tintcast\lib.ps1

$x = 300
$y = 450

$deskX = 1600
$deskY = 300

function Apply-And-Show([string]$theme, [string]$name, [string]$shot) {
    $desk = Pixel $deskX $deskY
    $result = Tintcast apply $theme
    Check-Applied $result $theme
    $expected = Scheme-Background $name
    $seen = WaitForPixel $x $y $expected 5
    Check "repaint-$theme" ($seen -eq $expected) "Terminal background expected $expected, saw $seen"
    $shown = WaitForChange $deskX $deskY $desk 20
    Check "wallpaper-shown-$theme" ($shown.Ms -ge 0) ("desktop at {0},{1} went from {2} to {3} after {4} ms" -f $deskX, $deskY, $desk, $shown.Color, $shown.Ms)
    Start-Sleep -Milliseconds 1500
    Shot $shot 0
    Check-Accent $theme
}

try {
    Log 'start'
    Start-Sleep -Seconds 20
    Install-Terminal
    $campbell = Pixel $x $y
    Shot '00-initial' 0
    $initial = Report '00-initial'
    Log "build $($initial.build), background $($initial.info.'background-kind')"

    $import = Tintcast import 'C:\tintcast\fixtures'
    Check 'import' ($import.Code -eq 0 -and $import.Out -match 'Imported Tokyo Night as tokyo-night \(\d+ wallpapers?\)') "exit $($import.Code)"
    $list = Tintcast list
    Check 'list' ($list.Out -match 'Tokyo Night  \(tokyo-night, dark') 'the imported theme is listed'

    Apply-And-Show 'daylight' 'Daylight' '01-daylight'
    Apply-And-Show 'ironwood' 'Ironwood' '02-ironwood'
    Apply-And-Show 'tokyo-night' 'Tokyo Night' '03-tokyo-night'
    Tintcast status | Out-Null

    Start-RegistryWatch 'restore'
    $restore = Tintcast restore
    Check 'restore' ($restore.Code -eq 0 -and $restore.Out -match '^Restored ') "exit $($restore.Code)"
    $seen = WaitForPixel $x $y $campbell 5
    Check 'repaint-restored' ($seen -eq $campbell) "Terminal background expected $campbell, saw $seen"
    Shot '99-final'
    Report '99-final' | Out-Null
    Start-Sleep -Seconds 12
    Report '99-final-late' | Out-Null
}
catch {
    Log ('suite error: {0}' -f $_)
    Check 'suite' $false "$_"
}
finally {
    Finish
}
