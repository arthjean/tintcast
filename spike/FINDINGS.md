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
| Windows Terminal in the Sandbox | Not installed, so its fragment was not tested there. | Sandbox log |
| GPUI from Zed `37faef8`, outside Zed | Builds and renders on Windows: window, Segoe UI text, colors. Built in one minute with Paneflow's cache. | Host, `tintcast-shell` |
| Tray icon (`tray-icon` 0.26) inside GPUI's message loop | Created; a left click reaches the GPUI entity and brings the window to the front. | Host, `tintcast-shell` |
| Global shortcut (`global-hotkey` 0.9) inside GPUI's message loop | Registers; a real press of Win + Ctrl + Alt + Space reaches the GPUI entity and brings the window to the front. | Host, `tintcast-shell` |
| Win + Ctrl + Shift + Space (Omarchy's key) | Reserved by Windows: taken in a clean Sandbox. | `sandbox/results-hotkeys/hotkeys.txt` |

Shortcuts free in a clean Sandbox: Win + Ctrl + Alt + Space, Win + Alt + Space, Ctrl + Alt + Space, Ctrl + Shift + Alt + Space, Win + Ctrl + Shift + T. Taken: Win + Ctrl + Shift + Space, Win + Shift + Space, Win + Alt + T.

## What this decides

- Apply follows Scene's ledger: record each registry value and wallpaper before the first change, and restore those values. Never restore through `IThemeManager2`.
- The accent goes through the registry, with a palette Tintcast builds, so a theme's exact accent survives. The `.theme` route stays a fallback.
- The Sandbox has no activated Windows, so Settings locks its own personalization page there, but these calls still work.
