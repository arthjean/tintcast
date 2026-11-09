param([string]$Version = '1.24.12741.0')

$ErrorActionPreference = 'Stop'
$cache = Join-Path $PSScriptRoot "cache\terminal-$Version"
$tag = "v$Version"
$patterns = '*_Windows10_PreinstallKit.zip', '*_x64.zip'

New-Item -ItemType Directory -Force -Path $cache | Out-Null
gh release download $tag -R microsoft/terminal -D $cache --clobber -p $patterns[0] -p $patterns[1]
if ($LASTEXITCODE -ne 0) { throw 'download failed' }

$digests = gh release view $tag -R microsoft/terminal --json assets --jq '.assets[] | "\(.name) \(.digest)"'
foreach ($line in $digests) {
    $name, $digest = $line -split ' '
    $file = Join-Path $cache $name
    if (-not (Test-Path $file)) { continue }
    $hash = 'sha256:' + (Get-FileHash -Algorithm SHA256 $file).Hash.ToLowerInvariant()
    if ($hash -ne $digest) { throw "$name does not match its release digest" }
}

$kit = Join-Path $cache 'kit'
Expand-Archive -Force -Path (Get-ChildItem $cache -Filter $patterns[0]).FullName -DestinationPath $kit
Get-ChildItem $kit -Filter 'Microsoft.UI.Xaml*.appx' | Where-Object Name -NotLike '*_x64__*' | Remove-Item
Expand-Archive -Force -Path (Get-ChildItem $cache -Filter $patterns[1]).FullName -DestinationPath (Join-Path $cache 'portable')
Write-Output "ready: $cache"
