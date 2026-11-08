$out = 'C:\tintcast\results'
New-Item -ItemType Directory -Force -Path $out | Out-Null
Start-Sleep -Seconds 15
& 'C:\tintcast\tintcast-probe.exe' hotkeys 2>&1 | Out-File -FilePath (Join-Path $out 'hotkeys.txt') -Encoding utf8
Set-Content -Path (Join-Path $out 'done.txt') -Value 'done'
Stop-Computer -Force
