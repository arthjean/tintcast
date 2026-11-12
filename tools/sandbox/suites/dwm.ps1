. C:\tintcast\lib.ps1

Add-Type -Namespace TintcastDiag -Name Dwm -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)]
public struct Params {
    public uint Color;
    public uint Afterglow;
    public uint ColorBalance;
    public uint AfterglowBalance;
    public uint BlurBalance;
    public uint GlassReflection;
    public uint OpaqueBlend;
}
[DllImport("dwmapi.dll", EntryPoint = "#127")]
public static extern int GetColorizationParameters(out Params value);
[DllImport("dwmapi.dll")]
public static extern int DwmGetColorizationColor(out uint color, out bool opaque);
'@

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32;

namespace TintcastDiag {
    public static class Watch {
        [DllImport("advapi32.dll", CharSet = CharSet.Unicode)]
        static extern int RegOpenKeyExW(UIntPtr root, string subKey, int options, int sam, out IntPtr result);
        [DllImport("advapi32.dll")]
        static extern int RegNotifyChangeKeyValue(IntPtr key, bool subtree, int filter, IntPtr signal, bool async);
        [DllImport("kernel32.dll")]
        static extern void GetSystemTimePreciseAsFileTime(out long time);

        static readonly UIntPtr CurrentUser = new UIntPtr(0x80000001u);
        static readonly List<string> lines = new List<string>();
        static readonly List<Thread> threads = new List<Thread>();
        static volatile bool stopping;

        public static string Now() {
            long time;
            GetSystemTimePreciseAsFileTime(out time);
            return DateTime.FromFileTime(time).ToString("HH:mm:ss.ffff");
        }

        public static void Mark(string text) {
            lock (lines) { lines.Add(Now() + " " + text); }
        }

        static string Format(object value) {
            if (value == null) return "absent";
            if (value is int) return "0x" + ((int)value).ToString("x8");
            byte[] bytes = value as byte[];
            if (bytes != null && bytes.Length >= 16) return "#" + bytes[12].ToString("x2") + bytes[13].ToString("x2") + bytes[14].ToString("x2");
            if (bytes != null) return BitConverter.ToString(bytes).Replace("-", "");
            return value.ToString();
        }

        static string Read(string path, string[] names) {
            StringBuilder text = new StringBuilder();
            using (RegistryKey key = Registry.CurrentUser.OpenSubKey(path)) {
                foreach (string name in names) {
                    text.Append(' ').Append(name).Append('=').Append(Format(key == null ? null : key.GetValue(name)));
                }
            }
            return text.ToString();
        }

        public static void Start(string label, string path, string[] names) {
            Thread thread = new Thread(() => {
                IntPtr key;
                if (RegOpenKeyExW(CurrentUser, path, 0, 0x20019, out key) != 0) { Mark("value " + label + " cannot open"); return; }
                using (AutoResetEvent signal = new AutoResetEvent(false)) {
                    IntPtr handle = signal.SafeWaitHandle.DangerousGetHandle();
                    RegNotifyChangeKeyValue(key, false, 4, handle, true);
                    string last = Read(path, names);
                    Mark("value " + label + last + " (initial)");
                    while (!stopping) {
                        if (!signal.WaitOne(200)) continue;
                        string at = Now();
                        RegNotifyChangeKeyValue(key, false, 4, handle, true);
                        string now = Read(path, names);
                        if (now != last) {
                            lock (lines) { lines.Add(at + " value " + label + now); }
                            last = now;
                        }
                    }
                }
            });
            thread.IsBackground = true;
            thread.Start();
            threads.Add(thread);
        }

        public static string[] Lines() {
            lock (lines) { return lines.ToArray(); }
        }

        public static string[] Stop() {
            stopping = true;
            foreach (Thread thread in threads) thread.Join(1000);
            lock (lines) { return lines.ToArray(); }
        }
    }
}
'@
$null = [Windows.UI.ViewManagement.UISettings, Windows.UI.ViewManagement, ContentType = WindowsRuntime]

$variant = Get-Content -Raw (Join-Path $root 'variant.json') | ConvertFrom-Json
$themes = if ($variant.themes) { @($variant.themes) } else { @('daylight', 'ironwood', 'inkwell') }
$gap = if ($variant.gap) { [int]$variant.gap } else { 0 }
$cycles = if ($variant.cycles) { [int]$variant.cycles } else { 1 }
$beforeRestore = if ($null -ne $variant.beforeRestore) { [int]$variant.beforeRestore } else { 3000 }

$snapshots = Join-Path $out 'snapshots.jsonl'
$themesKey = 'Software\Microsoft\Windows\CurrentVersion\Themes'
$dwmKey = 'Software\Microsoft\Windows\DWM'
$accentKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\Accent'
$personalizeKey = 'Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$registryValues = @(
    @($dwmKey, 'AccentColor'),
    @($dwmKey, 'ColorizationColor'),
    @($dwmKey, 'ColorizationAfterglow'),
    @($accentKey, 'AccentColorMenu'),
    @($accentKey, 'StartColorMenu'),
    @($personalizeKey, 'SystemUsesLightTheme'),
    @($personalizeKey, 'AppsUseLightTheme')
)

function Reg-Text([string]$path, [string]$name) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($path)
    if (-not $key) { return 'no key' }
    $value = $key.GetValue($name)
    $key.Close()
    if ($null -eq $value) { 'absent' } elseif ($value -is [int]) { '0x{0:x8}' -f $value } elseif ($value -is [byte[]]) { ($value | ForEach-Object { $_.ToString('x2') }) -join '' } else { "$value" }
}

function Snapshot([string]$step) {
    $state = [ordered]@{ step = $step; at = [TintcastDiag.Watch]::Now() }
    foreach ($pair in $registryValues) { $state[$pair[1]] = Reg-Text $pair[0] $pair[1] }
    $params = New-Object TintcastDiag.Dwm+Params
    $hr = [TintcastDiag.Dwm]::GetColorizationParameters([ref]$params)
    $state['dwm-live'] = if ($hr -eq 0) { 'color 0x{0:x8} afterglow 0x{1:x8} balance {2}' -f $params.Color, $params.Afterglow, $params.ColorBalance } else { 'hr 0x{0:x8}' -f $hr }
    $ui = (New-Object Windows.UI.ViewManagement.UISettings).GetColorValue([Windows.UI.ViewManagement.UIColorType]::Accent)
    $state['ui-accent'] = '#{0:x2}{1:x2}{2:x2}' -f $ui.R, $ui.G, $ui.B
    $themePath = Reg-Text $themesKey 'CurrentTheme'
    if (Test-Path $themePath) {
        $state['theme-lines'] = ((Get-Content $themePath) | Where-Object { $_ -match '^(ColorizationColor|SystemMode|AppMode)=' }) -join '; '
    }
    Add-Content -Path $snapshots -Value (ConvertTo-Json -InputObject $state -Compress) -Encoding UTF8
    [TintcastDiag.Watch]::Mark(("snapshot {0}: DWM {1} colorization {2}, live {3}, menu {4}, ui {5}" -f $step, $state['AccentColor'], $state['ColorizationColor'], $state['dwm-live'], $state['AccentColorMenu'], $state['ui-accent']))
    Log ("snapshot {0}: DWM {1}, live {2}, menu {3}, ui {4}, theme {5}" -f $step, $state['AccentColor'], $state['dwm-live'], $state['AccentColorMenu'], $state['ui-accent'], $state['theme-lines'])
    $state
}

function Write-Processes([string]$name) {
    $services = @{}
    Get-CimInstance Win32_Service | Where-Object ProcessId | ForEach-Object { $services[[int]$_.ProcessId] += @($_.Name) }
    Get-CimInstance Win32_Process | Sort-Object ProcessId | ForEach-Object {
        $command = "$($_.CommandLine)"
        if ($command.Length -gt 140) { $command = $command.Substring(0, 140) }
        '{0} {1} [{2}] {3}' -f $_.ProcessId, $_.Name, (($services[[int]$_.ProcessId]) -join ','), $command
    } | Set-Content -Path (Join-Path $out "processes-$name.txt") -Encoding UTF8
}

function Decode-Etw {
    $etl = Join-Path $out 'registry.etl'
    $names = @{}
    foreach ($file in Get-ChildItem $out -Filter 'processes-*.txt') {
        foreach ($line in Get-Content $file.FullName) {
            $parts = $line -split ' ', 3
            if ($parts[0] -notmatch '^\d+$') { continue }
            if (-not $names.ContainsKey([int]$parts[0])) { $names[[int]$parts[0]] = $parts[1] }
        }
    }
    foreach ($id in $script:tintcastIds) { $names[[int]$id] = 'tintcast.exe' }
    $lines = foreach ($event in Get-WinEvent -Path $etl -Oldest -ErrorAction SilentlyContinue) {
        if ($event.Id -ne 5) { continue }
        $value = "$($event.Properties[5].Value)"
        if ($value -match 'AccentColor|Colorization|UseLightTheme|AccentPalette|StartColorMenu|ColorHistory|CurrentTheme') {
            '{0:HH:mm:ss.ffff} write {1} by {2} (pid {3} tid {4} key {5:x})' -f $event.TimeCreated, $value, $names[[int]$event.ProcessId], $event.ProcessId, $event.ThreadId, [uint64]$event.Properties[0].Value
        }
    }
    $lines | Set-Content -Path (Join-Path $out 'registry-writes.txt') -Encoding UTF8
    Log ("etw: {0} matching SetValueKey events" -f @($lines).Count)
    @($lines)
}

$script:tintcastIds = New-Object System.Collections.ArrayList
function Step([string]$label) {
    [TintcastDiag.Watch]::Mark("tintcast $($args -join ' ') starts")
    $result = Tintcast @args
    [TintcastDiag.Watch]::Mark("tintcast $($args -join ' ') ended (pid $script:lastTintcastId)")
    [void]$script:tintcastIds.Add($script:lastTintcastId)
    Snapshot $label | Out-Null
    $result
}

function Same([object]$state, [object]$baseline) {
    $state['AccentColor'] -eq $baseline['AccentColor'] -and $state['ColorizationColor'] -eq $baseline['ColorizationColor'] -and $state['ColorizationAfterglow'] -eq $baseline['ColorizationAfterglow']
}

try {
    Log ("start, variant {0}" -f ($variant | ConvertTo-Json -Compress))
    Start-Sleep -Seconds 10
    if ($variant.terminal) { Install-Terminal }
    Tintcast list | Out-Null
    if ($variant.import) {
        $import = Tintcast import 'C:\tintcast\fixtures'
        Check 'import' ($import.Code -eq 0) "exit $($import.Code)"
    }
    Write-Processes 'start'
    [TintcastDiag.Watch]::Start('dwm', $dwmKey, [string[]]@('AccentColor', 'ColorizationColor', 'ColorizationAfterglow', 'ColorizationColorBalance'))
    [TintcastDiag.Watch]::Start('accent', $accentKey, [string[]]@('AccentColorMenu', 'StartColorMenu', 'AccentPalette'))
    [TintcastDiag.Watch]::Start('mode', $personalizeKey, [string[]]@('SystemUsesLightTheme', 'AppsUseLightTheme'))
    Start-Sleep -Milliseconds 500
    $baseline = Snapshot 'baseline'
    Report '00-initial' | Out-Null

    $trace = logman start tintreg -p Microsoft-Windows-Kernel-Registry 0x100 0xff -ets -o (Join-Path $out 'registry.etl') 2>&1
    Log ("logman start: {0}" -f ($trace -join ' '))

    $red = 0
    for ($cycle = 1; $cycle -le $cycles; $cycle++) {
        foreach ($theme in $themes) {
            Step "$cycle-$theme" apply $theme | Out-Null
            $ready = (Get-Date).AddMilliseconds($gap)
            if ($variant.report) { Report "accent-$cycle-$theme" | Out-Null }
            $rest = ($ready - (Get-Date)).TotalMilliseconds
            if ($rest -gt 0) { Start-Sleep -Milliseconds $rest }
        }
        if ($variant.status) { Tintcast status | Out-Null }
        Start-Sleep -Milliseconds $beforeRestore
        Step "$cycle-restore" restore | Out-Null
        Start-Sleep -Seconds 3
        $after = Snapshot "$cycle-restore+3"
        $green = Same $after $baseline
        if (-not $green) { $red++ }
        Check "dwm-restored-$cycle" $green ("AccentColor {0}, ColorizationColor {1}; baseline {2}, {3}" -f $after['AccentColor'], $after['ColorizationColor'], $baseline['AccentColor'], $baseline['ColorizationColor'])
        $seen = [TintcastDiag.Watch]::Lines()
        $from = 0
        for ($i = 0; $i -lt $seen.Count; $i++) { if ($seen[$i] -match 'tintcast restore starts') { $from = $i } }
        $states = @($seen[$from..($seen.Count - 1)] | Where-Object { $_ -match 'value dwm AccentColor=(\S+) ColorizationColor=(\S+)' } | ForEach-Object {
                $null = $_ -match 'value dwm AccentColor=(\S+) ColorizationColor=(\S+)'
                '{0}/{1}' -f $Matches[1], $Matches[2]
            })
        $expected = '{0}/{1}' -f $baseline['AccentColor'], $baseline['ColorizationColor']
        $single = $states.Count -eq 1 -and $states[0] -eq $expected
        Check "restore-one-transition-$cycle" $single ("DWM went through {0} state(s): {1}" -f $states.Count, ($states -join ' -> '))
        if (-not $green -and $cycle -lt $cycles) {
            [TintcastDiag.Watch]::Mark('suite repairs DWM values')
            $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($dwmKey, $true)
            foreach ($name in 'AccentColor', 'ColorizationColor', 'ColorizationAfterglow') {
                $number = [BitConverter]::ToInt32([BitConverter]::GetBytes([uint32]$baseline[$name]), 0)
                $key.SetValue($name, $number, [Microsoft.Win32.RegistryValueKind]::DWord)
            }
            $key.Close()
            Start-Sleep -Seconds 3
            $repaired = Snapshot "$cycle-repair+3"
            Log ("repair held: {0}" -f (Same $repaired $baseline))
        }
    }

    Write-Processes 'end'
    $stop = logman stop tintreg -ets 2>&1
    Log ("logman stop: {0}" -f ($stop -join ' '))
    $writes = Decode-Etw
    $values = [TintcastDiag.Watch]::Stop()
    @($values) + @($writes) | Sort-Object | Set-Content -Path (Join-Path $out 'timeline.txt') -Encoding UTF8
    Log ("VERDICT {0}: {1} of {2} cycles left DWM values different from the baseline" -f $(if ($red) { 'RED' } else { 'GREEN' }), $red, $cycles)
    Report '99-final' | Out-Null
}
catch {
    Log ('suite error: {0}' -f $_)
    Check 'suite' $false "$_"
}
finally {
    Finish
}
