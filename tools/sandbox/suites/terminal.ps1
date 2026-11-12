. C:\tintcast\lib.ps1

$x = 300
$y = 450
$fragment = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json'
$powershell = '{61c54bbd-c2c6-5271-96e7-009a87ff44bf}'
$cmd = '{0caa0dad-35be-5f56-a8ff-afceeeaa6101}'
$oneHalfDark = '#282c34'

function Touch-Settings {
    (Get-Item $script:settings).LastWriteTime = Get-Date
}

function Check-Repaint([string]$label, [string]$expected) {
    $seen = WaitForPixel $x $y $expected 5
    Check "repaint-$label" ($seen -eq $expected) "expected $expected within 5 s, saw $seen"
}

try {
    Log 'start'
    Start-Sleep -Seconds 20
    Install-Terminal
    $original = [System.IO.File]::ReadAllBytes($script:settings)
    $originalText = [System.IO.File]::ReadAllText($script:settings)
    $originalHash = Hash $script:settings
    Copy-Item $script:settings (Join-Path $out 'settings-original.json')
    $campbell = Pixel $x $y
    Log "terminal background before: $campbell"
    Shot '00-initial' 0
    Report '00-initial' | Out-Null

    $plan = Tintcast plan ironwood
    Check 'plan-fragment-only' ($plan.Out -match 'Write .*Fragments\\Tintcast\\tintcast\.json' -and $plan.Out -notmatch 'Set /profiles/defaults/colorScheme') 'default settings need the fragment alone'
    $apply = Tintcast apply ironwood
    Check-Applied $apply 'ironwood'
    Check-Repaint 'ironwood' (Scheme-Background 'Ironwood')
    Check 'settings-untouched' ((Hash $script:settings) -eq $originalHash) 'settings.json byte for byte unchanged'
    $json = Get-Content -Raw $fragment | ConvertFrom-Json
    $updates = @($json.profiles | ForEach-Object { $_.updates })
    Check 'fragment-updates' (($updates -contains $powershell) -and ($updates -contains $cmd) -and @($json.profiles | Where-Object colorScheme -NE 'Tintcast Ironwood').Count -eq 0) ($updates -join ', ')
    $scheme = $json.schemes[0]
    Check 'fragment-scheme' ($scheme.name -eq 'Tintcast Ironwood' -and $scheme.brightWhite -and $scheme.cursorColor -and $scheme.selectionBackground) "scheme $($scheme.name)"
    Shot '01-ironwood' 0

    $apply = Tintcast apply linen
    Check-Applied $apply 'linen'
    Check-Repaint 'linen' (Scheme-Background 'Linen')
    Shot '02-linen' 0

    $restore = Tintcast restore
    Check 'restore' ($restore.Code -eq 0) "exit $($restore.Code)"
    Check-Repaint 'restored' $campbell
    Check 'settings-restored' ((Hash $script:settings) -eq $originalHash) 'settings.json equals the original'
    Check 'fragment-removed' (-not (Test-Path $fragment)) 'the fragment is gone'
    Shot '03-restored' 0

    $pattern = '"defaults"\s*:\s*\{\s*\}'
    if ($originalText -notmatch $pattern) { throw 'profiles.defaults is not an empty object' }
    $userText = "// my Terminal settings`r`n" + [regex]::Replace($originalText, $pattern, '"defaults": { "colorScheme": "One Half Dark" /* mine */ }', 1)
    Replace-ByRename $script:settings $userText
    Check-Repaint 'after-rename' $oneHalfDark
    $userBytes = [System.IO.File]::ReadAllBytes($script:settings)
    $userHash = Hash $script:settings
    Shot '04-user-default' 0

    New-Item -ItemType Directory -Force -Path (Split-Path $fragment) | Out-Null
    $probe = [ordered]@{
        profiles = @([ordered]@{ updates = $powershell; colorScheme = 'Suite Probe' }, [ordered]@{ updates = $cmd; colorScheme = 'Suite Probe' })
        schemes  = @([ordered]@{ name = 'Suite Probe'; background = '#123456'; foreground = '#ffffff' })
    }
    [System.IO.File]::WriteAllText($fragment, ($probe | ConvertTo-Json -Depth 6), $utf8)
    Touch-Settings
    Start-Sleep -Seconds 5
    $seen = Pixel $x $y
    $defaultsWin = $seen -eq $oneHalfDark
    $verdict = if ($defaultsWin) { 'profiles.defaults.colorScheme in settings.json overrides a fragment updates entry' } elseif ($seen -eq '#123456') { 'a fragment updates entry overrides profiles.defaults.colorScheme' } else { "unclear: saw $seen" }
    Set-Content -Path (Join-Path $out 'override.txt') -Value $verdict -Encoding UTF8
    Check 'override-recorded' ($defaultsWin -or $seen -eq '#123456') $verdict
    Shot '05-override-probe' 0
    Remove-Item -Recurse -Force (Split-Path $fragment)
    Touch-Settings
    Check-Repaint 'probe-removed' $oneHalfDark

    $plan = Tintcast plan ironwood
    Check 'plan-edits-settings' ($plan.Out -match 'Set /profiles/defaults/colorScheme in .*settings\.json') 'a user default scheme needs the settings.json edit'
    $apply = Tintcast apply ironwood
    Check-Applied $apply 'ironwood-edit'
    Check-Repaint 'edited' (Scheme-Background 'Ironwood')
    $edited = [System.IO.File]::ReadAllText($script:settings)
    Check 'edit-keeps-comments' ($edited -eq $userText.Replace('"One Half Dark"', '"Tintcast Ironwood"')) 'only the scheme name changed; comments kept'
    $leftovers = @(Get-ChildItem (Split-Path $script:settings) -Force -Filter '*.tmp')
    Check 'edit-by-rename' ($leftovers.Count -eq 0) 'no temporary file left beside settings.json'
    Shot '06-edited' 0
    $restore = Tintcast restore
    Check 'restore-edit' ($restore.Code -eq 0) "exit $($restore.Code)"
    Check 'edit-restored' ((Hash $script:settings) -eq $userHash) 'settings.json equals the user version byte for byte'
    Check-Repaint 'edit-restored' $oneHalfDark

    [System.IO.File]::WriteAllBytes($script:settings, $original)
    Check-Repaint 'back-to-original' $campbell
    Shot '99-final' 0
    Report '99-final' | Out-Null
}
catch {
    Log ('suite error: {0}' -f $_)
    Check 'suite' $false "$_"
}
finally {
    Finish
}
