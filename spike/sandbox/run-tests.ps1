$ErrorActionPreference = 'Continue'
$root = 'C:\tintcast'
$out = Join-Path $root 'results'
$probe = Join-Path $root 'tintcast-probe.exe'
$state = Join-Path $out 'state.json'
$log = Join-Path $out 'log.txt'
$image = Join-Path $env:USERPROFILE 'Pictures\tintcast-test.bmp'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Add-Type -AssemblyName System.Windows.Forms, System.Drawing

function Log([string]$line) {
    Add-Content -Path $log -Value ('{0:HH:mm:ss} {1}' -f (Get-Date), $line) -Encoding UTF8
}

function Probe {
    $output = & $probe @args 2>&1 | Out-String
    Log ('probe {0} -> exit {1}{2}{3}' -f ($args -join ' '), $LASTEXITCODE, [Environment]::NewLine, $output.Trim())
}

function Shot([string]$name) {
    Start-Sleep -Seconds 5
    $bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Left, $bounds.Top, 0, 0, $bitmap.Size)
    $bitmap.Save((Join-Path $out "$name.png"), [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()
}

function Step([string]$name) {
    Shot $name
    & $probe report (Join-Path $out "$name.json")
    Log "step $name"
}

try {
    Log 'start'
    Start-Sleep -Seconds 20
    Start-Process 'ms-settings:colors'
    Start-Sleep -Seconds 8
    & $probe report $state
    Step '00-initial'

    & $probe image $image '#e1cc99' '#17181a'
    Probe wallpaper-set $image
    Step '01-wallpaper-set'
    Probe wallpaper-restore $state
    Step '02-wallpaper-restored'

    Probe appearance-set dark
    Step '03-dark'
    Probe registry-restore $state
    Step '04-dark-restored'

    Probe accent-registry '#d94f70'
    Step '05-accent-registry'
    Probe registry-restore $state
    Step '06-accent-registry-restored'

    Probe theme-apply '#3fa34d' dark
    Step '07-theme'
    Probe theme-restore $state
    Step '08-theme-restored'
    Probe registry-restore $state
    Step '09-final'

    $wt = Get-Command wt.exe -ErrorAction SilentlyContinue
    Log ('wt.exe: {0}' -f $(if ($wt) { $wt.Source } else { 'not found' }))
    Log ('Windows Terminal package: {0}' -f ((Get-AppxPackage *WindowsTerminal* | Select-Object -ExpandProperty PackageFullName) -join ', '))
}
catch {
    Log ('runner error: {0}' -f $_)
}
finally {
    Log 'done'
    Set-Content -Path (Join-Path $out 'done.txt') -Value 'done'
    Stop-Computer -Force
}
