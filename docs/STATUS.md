# Tintcast status

## EP-002: Omarchy import run (2026-11-09)

The release `tintcast.exe` imported `C:\dev\omarchy\themes` (Omarchy commit `d9cc361`) into a scratch data folder: `LOCALAPPDATA` pointed at a temporary folder, so the host's own `%LOCALAPPDATA%\Tintcast` was never touched, and the Omarchy checkout stayed unchanged.

| Measure | Result |
|---|---|
| Themes imported | 23 of 23 (target: 22 or more) |
| Wallpapers stored | 98 of 98 source images (PNG, JPEG, and WebP), none rejected |
| Warnings | Unknown `colors.toml` keys ignored in 5 themes: `active_border_color`, `active_tab_background`, `hyprland_active_border`, `hyprland_inactive_border` |
| First use (`tintcast list` on an empty data folder, 12 bundled 3840 by 2160 wallpapers generated) | 3.9 s |
| Import duration | 21.9 s for 98 images |
| `tintcast list` afterwards (27 themes, every stored wallpaper reused) | 113 ms |
| Disk use | `wallpapers\` 796 MB (110 PNG files and 110 thumbnails), `themes\` 70 MB |

A 3840 by 2160 WebP converts to PNG plus thumbnail in 163 ms in a release build (`cargo test --release -p tintcast-theme --test wallpaper_timing -- --nocapture`), against an 800 ms budget.

The PNG store weighs about 8 MB per 4K wallpaper. The PRD's open engineering question on PNG versus JPEG for stored wallpapers stays open.

## EP-003: Windows integrations in Windows Sandbox (2026-11-12)

`pwsh tools/sandbox/run.ps1 -Suite <name>` ran the release `tintcast.exe` in Windows Sandbox (build 26100.9818, 24H2, networking off). The final runs used the final code; their logs, reports, and screenshots are in `tools/sandbox/out\` (not committed).

| Suite | Run | Result |
|---|---|---|
| `desktop` (mode, accent, wallpaper, plan, undo, a denied registry write) | `desktop-20261109-171448` | Passed: every check, and all 18 watched values equal before the first apply and after Restore |
| `desktop -Variant '{"solid":true}'` (the same, from a solid color background) | `desktop-20261109-171245` | Passed: every check, the solid color `#1e3c5a` back after Restore, and all 18 watched values equal |
| `terminal` (Windows Terminal 1.24.12741.0) | `terminal-20261109-171640` | Passed: every check, and all 18 watched values equal |
| `acceptance` (Daylight, Ironwood, imported Tokyo Night, then Restore) | `acceptance-20261109-171811` | Passed: every check, each wallpaper painted before its screenshot, and all 18 watched values equal right after Restore and 12 s later |
| `dwm` (the acceptance themes three times over, with Terminal open, ETW registry trace) | `dwm-20261112-134559` | Passed: each of the 3 Restores moved the DWM values straight to the originals in one step, and all 18 watched values equal |

| Measure | Result |
|---|---|
| Apply duration, wallpaper already stored | At most 832 ms per apply across the final runs (the first apply of each `desktop` run; the others stay under 400 ms), registry transactions included (target: p95 at most 1500 ms) |
| Accent read back through `UISettings` | Equal to the theme's accent after every apply (`#d08a5b`, `#9a7417`, `#7aa2f7`) |
| Terminal repaint | The open tab shows the scheme's background within 5 s after each apply and after Restore |
| `settings.json` | Byte for byte unchanged by fragment-only applies; edited in place with comments kept when the user sets `profiles.defaults.colorScheme`; equal to the user's bytes after Restore |
| A denied write (`SetValue` denied on `HKCU\Software\Microsoft\Windows\DWM`) | "Windows blocked the change to AccentColor."; mode and accent rolled back, the wallpaper kept |

Findings:

- **Terminal settings layering (US-012):** a user's `profiles.defaults.colorScheme` in `settings.json` overrides a fragment's `updates` entry. The probe scheme in the fragment never showed; the user's One Half Dark stayed.
- **Reload after a rename:** replacing `settings.json` through `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING` repaints the open tab within 5 s.
- **Fixed: DWM accent values after Restore.** The earlier acceptance runs ended with an earlier theme's accent in `HKCU\Software\Microsoft\Windows\DWM` (Tokyo Night's `#7aa2f7` or Daylight's `#9a7417`). The `dwm` suite's ETW trace and change-notification watcher found two causes.
  - The shell owns those values: within 0.1 ms of any write to `Explorer\Accent`, explorer.exe, ShellHost.exe, and StartMenuExperienceHost.exe each rewrite the whole DWM colorization block from what they read.
  - Cause one: the engine read each original just before writing it, after the accent write had already made the shell copy the theme's accent into DWM. The ledger kept the theme's value as the original (Inkwell's `0xffafb83f` for `DWM\AccentColor`), so every Restore wrote the first theme's accent back itself.
  - Cause two: one write at a time let the shell read half-restored pairs. DWM went through 3 to 7 states per Restore, and whichever shell process wrote last won.
  - The fix: apply reads every original before its first write; Restore and Roll back decide every entry before writing; and an integration's registry writes commit in one registry transaction (KTM), so listeners only see the before or the after state. A probe measured one DWM transition with a transaction against 5 to 7 with separate writes.
  - The regression tests `originals_come_from_before_the_first_write_even_when_windows_mirrors_one_value_into_another` (engine) and `the_shell_mirroring_the_accent_never_sees_half_a_theme_and_restore_keeps_the_user_dwm_values` (windows) fail on the old engine.
- **Restore with a denied value:** when Windows denies one value of a transaction, Restore reports that value as failed, keeps its ledger entry, and commits the others again without it.
- **Fixed in review: Restore from a solid color background (US-011).** Starting from a solid color (`desktop -Variant '{"solid":true}'`), Restore reported "Restored" but the desktop showed a Windows Spotlight picture and `BackgroundType` stayed 0. `IDesktopWallpaper::SetWallpaper(monitor, "")` brings back Windows' default wallpaper rather than removing the picture. Removing it now goes through `SystemParametersInfoW(SPI_SETDESKWALLPAPER)` with an empty string; the rerun showed `#1e3c5a` again, with `BackgroundType` back to 1 and all 18 watched values equal.
- **Fixed in review: acceptance screenshots.** Tokyo Night's 8192 by 4608 wallpaper takes about 524 ms to paint, so the screenshot taken right after apply still showed Ironwood's wallpaper. The suite now waits until the desktop pixel changes (`wallpaper-shown-<theme>`) before taking each screenshot.
