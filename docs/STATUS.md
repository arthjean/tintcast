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
