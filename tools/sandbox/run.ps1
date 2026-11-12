param(
    [Parameter(Mandatory)][ValidateSet('desktop', 'terminal', 'acceptance', 'dwm')][string]$Suite,
    [switch]$NoBuild,
    [string]$TerminalVersion = '1.24.12741.0',
    [string]$OmarchyTheme = 'C:\dev\omarchy\themes\tokyo-night',
    [int]$TimeoutMinutes = 20,
    [string]$Variant = '{}'
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$manifest = Join-Path $repo 'Cargo.toml'
$suites = Join-Path $PSScriptRoot 'suites'
$options = $Variant | ConvertFrom-Json
$needsTerminal = $Suite -in 'terminal', 'acceptance' -or ($Suite -eq 'dwm' -and $options.terminal)
$needsOmarchy = $Suite -eq 'acceptance' -or ($Suite -eq 'dwm' -and $options.import)

if (-not $NoBuild) {
    cargo build --release -p tintcast-cli --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'tintcast did not build' }
}
$target = (cargo metadata --format-version 1 --no-deps --manifest-path $manifest | ConvertFrom-Json).target_directory
$exe = Join-Path $target 'release\tintcast.exe'
if (-not (Test-Path $exe)) { throw "missing $exe; run without -NoBuild" }

$terminalAssets = Join-Path $repo "spike\sandbox\cache\terminal-$TerminalVersion"
if ($needsTerminal -and -not (Test-Path (Join-Path $terminalAssets 'kit'))) {
    throw "Windows Terminal $TerminalVersion is not cached; run spike\sandbox\fetch-terminal.ps1 -Version $TerminalVersion"
}
if ($needsOmarchy -and -not (Test-Path (Join-Path $OmarchyTheme 'colors.toml'))) {
    throw "no Omarchy theme at $OmarchyTheme"
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$session = Join-Path $PSScriptRoot "out\$Suite-$stamp"
$run = Join-Path $session 'run'
$results = Join-Path $run 'results'
New-Item -ItemType Directory -Force -Path $results | Out-Null
Copy-Item $exe $run
Copy-Item (Join-Path $suites 'lib.ps1') $run
Copy-Item (Join-Path $suites "$Suite.ps1") (Join-Path $run 'suite.ps1')
Set-Content -Path (Join-Path $run 'variant.json') -Value $Variant -Encoding utf8
if ($needsOmarchy) {
    $fixture = Join-Path $run ('fixtures\' + (Split-Path $OmarchyTheme -Leaf))
    New-Item -ItemType Directory -Force -Path $fixture | Out-Null
    Copy-Item (Join-Path $OmarchyTheme 'colors.toml') $fixture
    $backgrounds = Join-Path $OmarchyTheme 'backgrounds'
    if (Test-Path $backgrounds) { Copy-Item -Recurse $backgrounds $fixture }
}

$assetFolder = ''
if ($needsTerminal) {
    $assetFolder = @"
    <MappedFolder>
      <HostFolder>$terminalAssets</HostFolder>
      <SandboxFolder>C:\assets</SandboxFolder>
      <ReadOnly>true</ReadOnly>
    </MappedFolder>
"@
}

$config = @"
<Configuration>
  <Networking>Disable</Networking>
  <MappedFolders>
    <MappedFolder>
      <HostFolder>$run</HostFolder>
      <SandboxFolder>C:\tintcast</SandboxFolder>
      <ReadOnly>false</ReadOnly>
    </MappedFolder>
$assetFolder
  </MappedFolders>
  <LogonCommand>
    <Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\tintcast\suite.ps1</Command>
  </LogonCommand>
</Configuration>
"@
$wsb = Join-Path $session 'tintcast.wsb'
Set-Content -Path $wsb -Value $config -Encoding utf8
Write-Output "suite ${Suite}: $session"
$idle = (Get-Date).AddMinutes(3)
while ((Get-Process -Name 'WindowsSandbox*' -ErrorAction SilentlyContinue) -and (Get-Date) -lt $idle) { Start-Sleep -Seconds 5 }
if (Get-Process -Name 'WindowsSandbox*' -ErrorAction SilentlyContinue) { throw 'another Windows Sandbox is still running' }
Start-Process -FilePath (Join-Path $env:windir 'System32\WindowsSandbox.exe') -ArgumentList "`"$wsb`""

$done = Join-Path $results 'done.txt'
$deadline = (Get-Date).AddMinutes($TimeoutMinutes)
while (-not (Test-Path $done) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 5 }
if (-not (Test-Path $done)) {
    Write-Output 'timeout: the sandbox did not finish'
    exit 1
}

$failures = 0
$initialPath = Join-Path $results '00-initial.json'
$finalPath = Join-Path $results '99-final.json'
if ((Test-Path $initialPath) -and (Test-Path $finalPath)) {
    $initial = (Get-Content -Raw $initialPath | ConvertFrom-Json).watched
    $final = (Get-Content -Raw $finalPath | ConvertFrom-Json).watched
    $keys = @($initial.PSObject.Properties.Name) + @($final.PSObject.Properties.Name) | Sort-Object -Unique
    foreach ($key in $keys) {
        $before = $initial.$key | ConvertTo-Json -Compress -Depth 6
        $after = $final.$key | ConvertTo-Json -Compress -Depth 6
        if ($before -ne $after) {
            Write-Output "differs: $key"
            Write-Output "  before: $before"
            Write-Output "  after:  $after"
            $failures++
        }
    }
    if ($failures -eq 0) { Write-Output "watched values: all $($keys.Count) equal before apply and after Restore" }
}
else {
    Write-Output 'missing 00-initial.json or 99-final.json'
    $failures++
}

$checksPath = Join-Path $results 'checks.json'
if (Test-Path $checksPath) {
    $checks = @(Get-Content -Raw $checksPath | ConvertFrom-Json)
    foreach ($check in $checks) {
        $mark = if ($check.pass) { 'PASS' } else { 'FAIL' }
        Write-Output ("{0} {1}: {2}" -f $mark, $check.name, $check.detail)
        if (-not $check.pass) { $failures++ }
    }
}
else {
    Write-Output 'missing checks.json'
    $failures++
}

Write-Output "results: $results"
if ($failures -gt 0) {
    Write-Output "$failures failure(s)"
    exit 1
}
Write-Output 'suite passed'
