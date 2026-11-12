$ErrorActionPreference = 'Continue'
$root = 'C:\tintcast'
$assets = 'C:\assets'
$out = Join-Path $root 'results'
$exe = Join-Path $root 'tintcast.exe'
$log = Join-Path $out 'log.txt'
$utf8 = New-Object System.Text.UTF8Encoding $false
$script:checks = New-Object System.Collections.ArrayList
$script:applies = New-Object System.Collections.ArrayList
$script:settings = $null
New-Item -ItemType Directory -Force -Path $out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace Tintcast -Name Native -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern bool MoveFileEx(string existing, string replacement, int flags);
'@

function Log([string]$line) {
    Add-Content -Path $log -Value ('{0:HH:mm:ss.fff} {1}' -f (Get-Date), $line) -Encoding UTF8
}

function Check([string]$name, [bool]$pass, [string]$detail) {
    [void]$script:checks.Add([pscustomobject]@{ name = $name; pass = $pass; detail = $detail })
    $mark = if ($pass) { 'PASS' } else { 'FAIL' }
    Log "check $name $mark $detail"
}

function Tintcast {
    $stdout = Join-Path $out 'last-stdout.txt'
    $stderr = Join-Path $out 'last-stderr.txt'
    $started = Get-Date
    $process = Start-Process -FilePath $exe -ArgumentList $args -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    $script:lastTintcastId = $process.Id
    $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
    $text = [System.IO.File]::ReadAllText($stdout)
    $errors = [System.IO.File]::ReadAllText($stderr)
    Log ('tintcast {0} (pid {6}) -> exit {1} in {2} ms{3}{4}{5}' -f ($args -join ' '), $process.ExitCode, $elapsed, [Environment]::NewLine, $text.TrimEnd(), $(if ($errors) { [Environment]::NewLine + 'stderr: ' + $errors.TrimEnd() } else { '' }), $process.Id)
    $applied = [regex]::Match($text, '^Applied (.+?) in (\d+) ms')
    if ($applied.Success) {
        [void]$script:applies.Add([pscustomobject]@{ theme = $applied.Groups[1].Value; ms = [int]$applied.Groups[2].Value })
    }
    [pscustomobject]@{ Code = $process.ExitCode; Out = $text; Err = $errors }
}

function Report([string]$name) {
    $path = Join-Path $out "$name.json"
    Start-Process -FilePath $exe -ArgumentList 'report', '--json' -NoNewWindow -Wait `
        -RedirectStandardOutput $path -RedirectStandardError (Join-Path $out 'last-stderr.txt') | Out-Null
    $json = Get-Content -Raw $path | ConvertFrom-Json
    Log "report $name"
    $json
}

function Watched($report, [string]$key) {
    $report.watched.PSObject.Properties | Where-Object Name -EQ $key | Select-Object -ExpandProperty Value
}

function Pixel([int]$x, [int]$y) {
    $bitmap = New-Object System.Drawing.Bitmap 1, 1
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($x, $y, 0, 0, $bitmap.Size)
    $color = $bitmap.GetPixel(0, 0)
    $graphics.Dispose()
    $bitmap.Dispose()
    '#{0:x2}{1:x2}{2:x2}' -f $color.R, $color.G, $color.B
}

function Shot([string]$name, [int]$wait = 5) {
    Start-Sleep -Seconds $wait
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
    Log "screenshot $name"
}

function WaitForPixel([int]$x, [int]$y, [string]$expected, [int]$seconds = 5) {
    $deadline = (Get-Date).AddSeconds($seconds)
    do {
        $seen = Pixel $x $y
        if ($seen -eq $expected) { return $seen }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    $seen
}

function WaitForChange([int]$x, [int]$y, [string]$from, [int]$seconds = 15) {
    $started = Get-Date
    $deadline = $started.AddSeconds($seconds)
    do {
        $seen = Pixel $x $y
        if ($seen -ne $from) {
            return [pscustomobject]@{ Color = $seen; Ms = [int]((Get-Date) - $started).TotalMilliseconds }
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    [pscustomobject]@{ Color = $seen; Ms = -1 }
}

function Hash([string]$path) {
    if (Test-Path $path) { (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant() } else { 'missing' }
}

function Decode-Abgr($value) {
    $n = [uint32]$value.data
    '#{0:x2}{1:x2}{2:x2}' -f ($n -band 0xff), (($n -shr 8) -band 0xff), (($n -shr 16) -band 0xff)
}

function Replace-ByRename([string]$path, [string]$text) {
    $temp = Join-Path (Split-Path $path) ('.tintcast-suite-' + [guid]::NewGuid() + '.tmp')
    [System.IO.File]::WriteAllText($temp, $text, $utf8)
    if (-not [Tintcast.Native]::MoveFileEx($temp, $path, 1)) { throw "rename over $path failed" }
}

function Install-Terminal {
    $kit = Join-Path $assets 'kit'
    $bundle = Get-ChildItem $kit -Filter '*.msixbundle' | Select-Object -First 1
    $xaml = Get-ChildItem $kit -Filter 'Microsoft.UI.Xaml*.appx' | Select-Object -First 1
    Add-AppxPackage -Path $bundle.FullName -DependencyPath $xaml.FullName -ErrorAction Stop
    Log ('installed: {0}' -f ((Get-AppxPackage *WindowsTerminal*).PackageFullName -join ', '))
    $script:settings = Join-Path $env:LOCALAPPDATA 'Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json'
    $colors = Join-Path $root 'colors.ps1'
    Set-Content -Path $colors -Encoding UTF8 -Value @'
$e = [char]27
Write-Host 'Tintcast color check'
foreach ($base in 40, 100) { Write-Host ((0..7 | ForEach-Object { "$e[$($base + $_)m    $e[0m" }) -join '') }
'@
    $wt = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\wt.exe'
    for ($i = 0; $i -lt 10 -and -not (Test-Path $wt); $i++) { Start-Sleep -Seconds 1 }
    Start-Process -FilePath $wt -ArgumentList '--pos', '40,40', '--size', '90,28', 'powershell', '-NoExit', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $colors
    for ($i = 0; $i -lt 40 -and -not (Test-Path $script:settings); $i++) { Start-Sleep -Seconds 1 }
    Start-Sleep -Seconds 6
    if (-not (Test-Path $script:settings)) { throw 'Windows Terminal did not create settings.json' }
    Log "settings $script:settings"
}

function Scheme-Background([string]$theme) {
    $fragment = Join-Path $env:LOCALAPPDATA 'Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json'
    if (-not (Test-Path $fragment)) { return $null }
    $json = Get-Content -Raw $fragment | ConvertFrom-Json
    ($json.schemes | Where-Object name -EQ "Tintcast $theme").background
}

function Check-Accent([string]$label) {
    $report = Report "accent-$label"
    $registry = Decode-Abgr (Watched $report 'reg:HKCU\Software\Microsoft\Windows\DWM\AccentColor')
    $shown = Watched $report 'ui-accent'
    Check "accent-$label" ($registry -eq $shown) "UISettings shows $shown, the registry accent is $registry"
}

function Check-Applied($result, [string]$label) {
    Check "apply-$label" ($result.Code -eq 0 -and $result.Out -notmatch 'Applied, but') ("exit {0}" -f $result.Code)
}

function Start-RegistryWatch([string]$name) {
    $file = Join-Path $out "watch-$name.txt"
    Start-Job -ArgumentList $file -ScriptBlock {
        param($file)
        $values = @(
            @('Software\Microsoft\Windows\DWM', 'AccentColor'),
            @('Software\Microsoft\Windows\DWM', 'ColorizationColor'),
            @('Software\Microsoft\Windows\CurrentVersion\Explorer\Accent', 'AccentColorMenu'),
            @('Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers', 'BackgroundType'),
            @('Control Panel\Desktop', 'WallPaper')
        )
        $last = @{}
        $deadline = (Get-Date).AddSeconds(25)
        while ((Get-Date) -lt $deadline) {
            foreach ($pair in $values) {
                $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($pair[0])
                $value = if ($key) { $key.GetValue($pair[1]) } else { $null }
                if ($key) { $key.Close() }
                $text = if ($value -is [int]) { '0x{0:x8}' -f $value } else { "$value" }
                $id = $pair[1]
                if ($last[$id] -ne $text) {
                    Add-Content -Path $file -Value ('{0:HH:mm:ss.fff} {1} = {2}' -f (Get-Date), $id, $text)
                    $last[$id] = $text
                }
            }
            Start-Sleep -Milliseconds 25
        }
    } | Out-Null
    Start-Sleep -Seconds 2
    Log "registry watch $name started"
}

function Finish {
    if ($script:applies.Count -gt 0) {
        $slowest = ($script:applies | Measure-Object -Property ms -Maximum).Maximum
        $list = ($script:applies | ForEach-Object { '{0} {1} ms' -f $_.theme, $_.ms }) -join ', '
        Check 'apply-duration' ($slowest -le 1500) "slowest $slowest ms ($list)"
    }
    ConvertTo-Json -InputObject @($script:checks) -Depth 4 | Set-Content -Path (Join-Path $out 'checks.json') -Encoding UTF8
    Log 'done'
    Set-Content -Path (Join-Path $out 'done.txt') -Value 'done'
    Stop-Computer -Force
}
