# Phase 0 findings

Checked on 2026-11-08. The host runs Windows 11 26H2 (build 26300.9818). Windows Sandbox on that host runs build 26100.9818 (24H2), the same servicing level. Every change ran in the Sandbox (`sandbox/launch.ps1`); the host only ran read-only reports.

| Question | Answer | Evidence |
|---|---|---|
| Wallpaper per display, and back | Works. `IDesktopWallpaper::SetWallpaper` in 21 ms, restore in 15 ms, read back by `GetWallpaper`. | Sandbox steps 01 and 02 |
| Light or dark | Works. Two `Personalize` values and the `ImmersiveColorSet` broadcast repaint Settings and the taskbar live, in 134 ms. | Sandbox steps 03 and 04 |
| Accent, through the registry | Works, with the exact color. `UISettings` reads back `#d94f70`; Settings and the taskbar repaint. Tintcast must build the 8-shade `AccentPalette` itself. | Sandbox steps 05 and 06 |
| Accent, through a `.theme` file and `IThemeManager2::AddAndSelectTheme` | Works in 54 ms and sets the mode too, but Windows replaces the color with its own accessible shade (`#3fa34d` became `#1e8935`). | Sandbox step 07 |
| Restore through `IThemeManager2` | Fails. Applying the original "Unsaved theme" again changes nothing, because that entry follows the current state. | Sandbox step 08 |
| Restore through recorded registry values | Works. Every value, `UISettings`, and DWM's color return to the original. | Sandbox step 09 |
| `SetUserColorPreference` in `uxtheme.dll` | Not exported by name. `GetUserColorPreference` is. | Host report |
| Windows Terminal in the Sandbox | Not installed by default. `sandbox/fetch-terminal.ps1` downloads the release the host runs (1.24.12741.0), and `run-terminal.ps1` installs it as a package with `Add-AppxPackage`. | Terminal log |
| Color schemes from a fragment (`%LOCALAPPDATA%\Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json`) | Loaded, but a new or edited fragment changes nothing until `settings.json` changes: Terminal watches only that file. | Terminal steps 01 and 04 |
| Picking a scheme with `profiles.defaults.colorScheme` in `settings.json` | Repaints the open tab live, within five seconds. Switching the name switches the scheme. | Terminal steps 02 and 03 |
| Reloading after a fragment edit | Setting the last write time of `settings.json`, without changing a byte, reloads the fragment. | Terminal step 05 |
| A fragment's `updates` entries (`colorScheme` on the Windows PowerShell and Command Prompt profiles) | Repaints the tab with `settings.json` byte for byte unchanged. Terminal does not write the fragment's changes into `settings.json`. | Terminal steps 07 and 08 |
| Restore | Writing back the original bytes and removing the fragment brings Campbell back; the final `settings.json` matches the original. | Terminal steps 06 and 08 |
| GPUI from Zed `37faef8`, outside Zed | Builds and renders on Windows: window, Segoe UI text, colors. Built in one minute with Paneflow's cache. | Host, `tintcast-shell` |
| Tray icon (`tray-icon` 0.26) inside GPUI's message loop | Created; a left click reaches the GPUI entity and brings the window to the front. | Host, `tintcast-shell` |
| Global shortcut (`global-hotkey` 0.9) inside GPUI's message loop | Registers; a real press of Win + Ctrl + Alt + Space reaches the GPUI entity and brings the window to the front. | Host, `tintcast-shell` |
| Win + Ctrl + Shift + Space (Omarchy's key) | Reserved by Windows: taken in a clean Sandbox. | `sandbox/results-hotkeys/hotkeys.txt` |

Shortcuts free in a clean Sandbox: Win + Ctrl + Alt + Space, Win + Alt + Space, Ctrl + Alt + Space, Ctrl + Shift + Alt + Space, Win + Ctrl + Shift + T. Taken: Win + Ctrl + Shift + Space, Win + Shift + Space, Win + Alt + T.

On the host (`sandbox/results-hotkeys/hotkeys-host.txt`), Win + Ctrl + Alt + B and Win + Ctrl + Alt + N are free; Win + Ctrl + Space, Win + Ctrl + Alt + Shift + Space, and Win + Alt + B are taken. The host carries Windows' own shortcuts plus its apps', so a key free there is not reserved by Windows.

## What this decides

- Apply follows Scene's ledger: record each registry value and wallpaper before the first change, and restore those values. Never restore through `IThemeManager2`.
- The accent goes through the registry, with a palette Tintcast builds, so a theme's exact accent survives. The `.theme` route stays a fallback.
- The Sandbox has no activated Windows, so Settings locks its own personalization page there, but these calls still work.
- Windows Terminal gets its schemes from a Tintcast fragment, which never touches the user's file. The scheme is picked through the fragment's `updates` entries when the user has not set one; otherwise Tintcast writes `profiles.defaults.colorScheme` into `settings.json` as a recorded edit. Each apply ends by setting the last write time of `settings.json`, so an open Terminal reloads.
- `updates` reaches only profiles that Terminal or another app generates, by GUID. A profile the user wrote by hand needs the `settings.json` edit.
- The tab row stays dark under a light scheme. A Terminal `theme` for the window is phase 1 work.
- Default shortcuts: Win + Ctrl + Alt + Space opens the switcher, Win + Ctrl + Alt + B moves to the next background.
