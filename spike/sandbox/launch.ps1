param([switch]$NoBuild, [string]$Runner = 'run-tests.ps1', [string]$Assets = '', [int]$TimeoutMinutes = 12)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$manifest = Join-Path $repo 'Cargo.toml'
$run = Join-Path $PSScriptRoot 'run'

if (-not $NoBuild) {
    cargo build --release -p tintcast-probe --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'the probe did not build' }
}
$target = (cargo metadata --format-version 1 --no-deps --manifest-path $manifest | ConvertFrom-Json).target_directory

if (Test-Path $run) { Remove-Item -Recurse -Force $run }
New-Item -ItemType Directory -Force -Path (Join-Path $run 'results') | Out-Null
Copy-Item (Join-Path $target 'release\tintcast-probe.exe') $run
Copy-Item (Join-Path $PSScriptRoot $Runner) $run

$assetFolder = ''
if ($Assets) {
    $assetPath = (Resolve-Path $Assets).Path
    $assetFolder = @"
    <MappedFolder>
      <HostFolder>$assetPath</HostFolder>
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
    <Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\tintcast\$Runner</Command>
  </LogonCommand>
</Configuration>
"@
$wsb = Join-Path $run 'tintcast.wsb'
Set-Content -Path $wsb -Value $config -Encoding utf8
Start-Process -FilePath (Join-Path $env:windir 'System32\WindowsSandbox.exe') -ArgumentList "`"$wsb`""

$done = Join-Path $run 'results\done.txt'
$deadline = (Get-Date).AddMinutes($TimeoutMinutes)
while (-not (Test-Path $done) -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 5 }
if (-not (Test-Path $done)) { Write-Output 'timeout: the sandbox did not finish'; exit 1 }
Write-Output "done: $run\results"
