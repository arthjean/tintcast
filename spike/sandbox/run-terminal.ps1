$ErrorActionPreference = 'Continue'
$root = 'C:\tintcast'
$assets = 'C:\assets'
$out = Join-Path $root 'results'
$probe = Join-Path $root 'tintcast-probe.exe'
$log = Join-Path $out 'log.txt'
$colors = Join-Path $root 'colors.ps1'
$fragmentDir = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows Terminal\Fragments\Tintcast'
$fragment = Join-Path $fragmentDir 'tintcast.json'
$utf8 = New-Object System.Text.UTF8Encoding $false
New-Item -ItemType Directory -Force -Path $out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

function Log([string]$line) {
    Add-Content -Path $log -Value ('{0:HH:mm:ss} {1}' -f (Get-Date), $line) -Encoding UTF8
}

function Hash([string]$path) {
    if (Test-Path $path) { (Get-FileHash -Algorithm SHA256 $path).Hash.Substring(0, 12) } else { 'missing' }
}

function Shot([string]$name) {
    Start-Sleep -Seconds 5
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
    $pixel = $bitmap.GetPixel([int]($bounds.Width * 0.5), [int]($bounds.Height * 0.7))
    $bitmap.Save((Join-Path $out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    Log ('step {0}: background #{1:x2}{2:x2}{3:x2}, settings.json {4}' -f $name, $pixel.R, $pixel.G, $pixel.B, (Hash $script:settings))
}

function Scheme([string]$name, [string]$background, [string]$foreground, [string[]]$ansi, [string]$cursor, [string]$selection) {
    $keys = 'black', 'red', 'green', 'yellow', 'blue', 'purple', 'cyan', 'white',
        'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue', 'brightPurple', 'brightCyan', 'brightWhite'
    $scheme = [ordered]@{ name = $name; background = $background; foreground = $foreground; cursorColor = $cursor; selectionBackground = $selection }
    for ($i = 0; $i -lt 16; $i++) { $scheme[$keys[$i]] = $ansi[$i] }
    $scheme
}

function Fragment([string]$daylightBackground, [object[]]$profiles) {
    $moondust = Scheme 'Tintcast Moondust' '#17181a' '#d9d7d2' @(
        '#17181a', '#da9c94', '#a3cea3', '#dcca92', '#8eb1dd', '#caa1cc', '#85d1d0', '#d9d7d2',
        '#454b51', '#f3aca3', '#b2e4b2', '#f2dd9d', '#9cc4f8', '#e1b2e3', '#8ee6e5', '#f0eeea') '#e1cc99' '#454b51'
    $daylight = Scheme 'Tintcast Daylight' $daylightBackground '#2b2d31' @(
        '#2b2d31', '#b5483d', '#4f7f3a', '#9a7417', '#3b6aa8', '#8a4f94', '#2f7f7d', '#d8d4cc',
        '#6b6f76', '#c95b4f', '#5f9446', '#b08a22', '#4a7fc4', '#a061aa', '#3a9794', '#f4f1ea') '#9a7417' '#d8d4cc'
    $body = [ordered]@{ schemes = @($moondust, $daylight) }
    if ($profiles) { $body['profiles'] = $profiles }
    New-Item -ItemType Directory -Force -Path $fragmentDir | Out-Null
    [System.IO.File]::WriteAllText($fragment, ($body | ConvertTo-Json -Depth 8), $utf8)
    Log ('fragment written ({0}, {1} profile updates)' -f $daylightBackground, $(if ($profiles) { $profiles.Count } else { 0 }))
}

function Touch {
    (Get-Item $script:settings).LastWriteTime = Get-Date
    Log 'settings.json touched'
}

function SetText([string]$text) {
    [System.IO.File]::WriteAllText($script:settings, $text, $utf8)
}

try {
    Log 'start'
    Start-Sleep -Seconds 20
    Set-Content -Path $colors -Encoding UTF8 -Value @'
$e = [char]27
Write-Host 'Tintcast color check'
foreach ($base in 40, 100) { Write-Host ((0..7 | ForEach-Object { "$e[$($base + $_)m      $e[0m" }) -join '') }
Write-Host ((30..37 | ForEach-Object { "$e[${_}mcolor$e[0m " }) -join '')
Write-Host ((90..97 | ForEach-Object { "$e[${_}mbright$e[0m " }) -join '')
'@

    $mode = 'packaged'
    try {
        $kit = Join-Path $assets 'kit'
        $bundle = Get-ChildItem $kit -Filter '*.msixbundle' | Select-Object -First 1
        $xaml = Get-ChildItem $kit -Filter 'Microsoft.UI.Xaml*.appx' | Select-Object -First 1
        Add-AppxPackage -Path $bundle.FullName -DependencyPath $xaml.FullName -ErrorAction Stop
        Log ('installed: {0}' -f ((Get-AppxPackage *WindowsTerminal*).PackageFullName -join ', '))
    }
    catch {
        Log "install failed, using the unpackaged build: $_"
        $mode = 'unpackaged'
    }

    $arguments = @('-M', 'powershell', '-NoExit', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $colors)
    if ($mode -eq 'packaged') {
        $script:settings = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json'
        $wt = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\wt.exe'
        for ($i = 0; $i -lt 10 -and -not (Test-Path $wt); $i++) { Start-Sleep -Seconds 1 }
    }
    else {
        $script:settings = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows Terminal\settings.json'
        $wt = (Get-ChildItem (Join-Path $assets 'portable') -Recurse -Filter 'wt.exe' | Select-Object -First 1).FullName
    }
    Log "mode $mode, wt $wt (exists: $(Test-Path $wt)), settings $script:settings"
    Start-Process -FilePath $wt -ArgumentList $arguments
    for ($i = 0; $i -lt 40 -and -not (Test-Path $script:settings); $i++) { Start-Sleep -Seconds 1 }
    Start-Sleep -Seconds 6
    if (-not (Test-Path $script:settings)) { throw 'Windows Terminal did not create settings.json' }

    $original = [System.IO.File]::ReadAllBytes($script:settings)
    Copy-Item $script:settings (Join-Path $out 'settings-original.json')
    Shot '00-initial'

    Fragment '#f4f1ea' $null
    Shot '01-fragment-only'

    $text = [System.IO.File]::ReadAllText($script:settings)
    $pattern = '"defaults"\s*:\s*\{\s*\}'
    if ($text -notmatch $pattern) { throw 'profiles.defaults is not an empty object' }
    SetText ([regex]::Replace($text, $pattern, '"defaults": { "colorScheme": "Tintcast Moondust" }', 1))
    Log 'settings.json: defaults.colorScheme = Tintcast Moondust'
    Shot '02-select-moondust'
    Copy-Item $script:settings (Join-Path $out 'settings-moondust.json')

    SetText ([System.IO.File]::ReadAllText($script:settings).Replace('"Tintcast Moondust"', '"Tintcast Daylight"'))
    Log 'settings.json: defaults.colorScheme = Tintcast Daylight'
    Shot '03-select-daylight'

    Fragment '#e3eedb' $null
    Shot '04-fragment-edited'
    Touch
    Shot '05-touched'

    Remove-Item -Recurse -Force $fragmentDir
    [System.IO.File]::WriteAllBytes($script:settings, $original)
    Log 'settings.json restored from its original bytes, fragment removed'
    Shot '06-restored'

    $powershell = '{61c54bbd-c2c6-5271-96e7-009a87ff44bf}'
    $cmd = '{0caa0dad-35be-5f56-a8ff-afceeeaa6101}'
    Fragment '#f4f1ea' @(
        [ordered]@{ updates = $powershell; colorScheme = 'Tintcast Moondust' },
        [ordered]@{ updates = $cmd; colorScheme = 'Tintcast Moondust' })
    Touch
    Shot '07-profile-updates'
    Copy-Item $script:settings (Join-Path $out 'settings-updates.json')

    Remove-Item -Recurse -Force $fragmentDir
    Touch
    Shot '08-updates-removed'
    Log ('settings.json equals the original: {0}' -f ((Hash $script:settings) -eq (Hash (Join-Path $out 'settings-original.json'))))

    & $probe hotkeys 2>&1 | Out-File -FilePath (Join-Path $out 'hotkeys.txt') -Encoding utf8
}
catch {
    Log ('runner error: {0}' -f $_)
}
finally {
    Log 'done'
    Set-Content -Path (Join-Path $out 'done.txt') -Value 'done'
    Stop-Computer -Force
}
