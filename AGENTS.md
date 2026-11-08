# Tintcast agent guidance

Tintcast gives the Windows desktop, terminals, and code editors one theme in one step: wallpaper, light or dark, accent color, Windows Terminal, VS Code, Neovim, and more. It shows every change first and can put the original setup back. It is written in Rust, with Zed's GPUI for the interface and `windows-rs` for the system.

The repository is in phase 0: `spike/` holds throwaway probes that answer whether each system call works. Product code starts in `crates/` in phase 1.

## Gates

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml`. `CARGO_TARGET_DIR` points at a shared `C:\dev\.cargo-target`, so binaries land there, not in `target/`.

## Rules

- Never change the user's live setup from the host: wallpaper, registry values, the Windows theme, or any app's settings. System changes run in Windows Sandbox through `spike/sandbox/launch.ps1`, which opens a Sandbox window for a few minutes. `tintcast-probe report` only reads, so it is safe on the host.
- Do not comment code.
- Commit and push only when the user asks.

## GPUI

GPUI comes from Zed's git repository at the commit Paneflow uses (`37faef8`), with the `[patch.crates-io]` entries Paneflow carries. Bump the commit and the patches together, from Paneflow's `Cargo.toml`. Read the API in the Cargo git checkout of that commit; `C:\dev\zed` is a newer checkout of the same repository.

## Undocumented Windows calls

- `IThemeManager2` (CLSID `9324da94-50ec-4a14-a770-e90ca03e7c8f`) applies a `.theme` file, which carries the accent color and the light or dark mode. Its layout comes from namazso's SecureUxTheme (GPL-3.0-or-later) and Auto Dark Mode. It must run on an STA thread.
- `uxtheme.dll` exports `GetUserColorPreference` by name on build 26300, but not `SetUserColorPreference`.
- Restore through recorded registry values and wallpaper paths, never through `IThemeManager2`: its "Unsaved theme" entry follows the current state. See `spike/FINDINGS.md`.
- Win + Ctrl + Shift + Space, Omarchy's theme key, is reserved by Windows.

## Scene

Tintcast ports logic from InsaneArts/Scene, a macOS app under the MIT License. `C:\dev\Scene` is reference material only: never commit there. When Scene code is ported, add its MIT notice to `NOTICE`. Tintcast needs its own wallpapers; Scene's images keep their own licenses.

## License

GPL-3.0-or-later. Outside contributions need a contributor agreement first, so the author can still license Tintcast in other ways.
