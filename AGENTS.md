# Tintcast agent guidance

Tintcast gives the Windows desktop, terminals, and code editors one theme in one step: wallpaper, light or dark, accent color, Windows Terminal, VS Code, Neovim, and more. It shows every change first and can put the original setup back. It is written in Rust, with Zed's GPUI for the interface and `windows-rs` for the system.

Phase 1 product code lives in `crates/`, and dependencies point one way: `tintcast-theme` ← `tintcast-engine` ← `tintcast-windows` ← `tintcast-cli` (binary `tintcast`) and `tintcast-app` (binary `tintcast-app`). `spike/` keeps the phase 0 probes and their evidence; leave it unchanged.

## Engine

- Every system change is an `Operation` that an `Integration` plans and the engine runs. Calling `SystemServices` from elsewhere breaks the ledger, Undo, and the three-way Restore.
- Engine logic is tested against `tintcast_engine::fake::FakeServices`, with `DataDir::at` on a temporary folder and a unique `EngineLock` name per test. Each file directly under `crates/*/tests/` starts with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, because `clippy.toml` does not exempt test helpers.
- The data folder files (`ledger.json`, `history.json`, `journal\<run-id>.json`) carry `"version": 1` and are replaced only through `write_atomic`. File backups are stored once, by SHA-256, in `backups\`.
- Themes reach the engine only through `tintcast_engine::Library`: it embeds the four bundled themes, loads `themes\<slug>\`, imports Omarchy folders, and stores every wallpaper as `wallpapers\<theme-id>-<content-hash>.png` with a thumbnail in `wallpapers\thumbs\`. `Library::open` deletes stored wallpapers that the ledger, the history, journals, and the library no longer reference, so tests pass `LibraryOptions` with a unique lock and a small `bundled_size`.
- Theme folders are data: nothing in them is executed, links and junctions are skipped, and images decode under the limits in `tintcast_theme::limits`.
- Windows calls live in `tintcast-windows`: `WindowsServices` implements `SystemServices` and refuses writes outside FR-11's allowlist (`Allowlist`), and the `Desktop` trait holds the read-only checks the integrations need (contrast theme, wallpaper policy, background kind, `UISettings` accent). `tintcast_windows::integrations` is the one list of integrations, used through `System::engine`.
- Before each write, the ledger entry's `pending` records the value about to be written, so Restore, Roll back, and Keep still recognize Tintcast's write when a crash lands between the write and the ledger update.
- Windows copies values into each other: within a millisecond of an `Explorer\Accent` write, explorer, ShellHost, and StartMenuExperienceHost rewrite the whole `HKCU\...\DWM` colorization block. So an apply reads every original before its first write (the run's snapshot), Restore and Roll back decide every entry before writing any, and an integration's registry writes go through `SystemServices::commit_registry`, one registry transaction (KTM) per run of consecutive registry operations, so the shell never reads half a theme.

## Gates

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The toolchain is pinned in `rust-toolchain.toml`. `CARGO_TARGET_DIR` points at a shared `C:\dev\.cargo-target`, so binaries land there, not in `target/`.

## Rules

- Never change the user's live setup from the host: wallpaper, registry values, the Windows theme, or any app's settings. System changes run in Windows Sandbox through `pwsh tools/sandbox/run.ps1 -Suite desktop|terminal|acceptance|dwm`, which builds a release, opens a Sandbox window for a few minutes, and fails when `tintcast report --json` differs before and after Restore or when a suite check fails; results land in `tools/sandbox/out\`. The `terminal` and `acceptance` suites need `spike/sandbox/fetch-terminal.ps1` once. The `dwm` suite takes `-Variant '{"terminal":true,"import":true,"themes":[...],"gap":ms,"cycles":n}'`, traces every registry writer with ETW into `timeline.txt`, and fails unless each Restore moves the DWM values to the originals in one step. The `desktop` suite takes `-Variant '{"solid":true}'` to start from a solid color background. On the host, only `tintcast report`, `plan`, `list`, and `status` are safe, and tests never run `apply`, `undo`, or `restore` through the real binary: they call `tintcast_cli::run` with `FakeServices` and `tintcast_windows::fake::FakeDesktop`.
- Do not comment code.
- Commit and push only when the user asks.

## GPUI

GPUI comes from Zed's git repository at the commit Paneflow uses (`37faef8`), with the `[patch.crates-io]` entries Paneflow carries. Bump the commit and the patches together, from Paneflow's `Cargo.toml`. Read the API in the Cargo git checkout of that commit; `C:\dev\zed` is a newer checkout of the same repository.

## Undocumented Windows calls

- `IThemeManager2` (CLSID `9324da94-50ec-4a14-a770-e90ca03e7c8f`) applies a `.theme` file, which carries the accent color and the light or dark mode. Its layout comes from namazso's SecureUxTheme (GPL-3.0-or-later) and Auto Dark Mode. It must run on an STA thread.
- `uxtheme.dll` exports `GetUserColorPreference` by name on build 26300, but not `SetUserColorPreference`.
- Restore through recorded registry values and wallpaper paths, never through `IThemeManager2`: its "Unsaved theme" entry follows the current state. See `spike/FINDINGS.md`.
- Win + Ctrl + Shift + Space, Omarchy's theme key, is reserved by Windows. Tintcast's defaults are Win + Ctrl + Alt + Space for the switcher and Win + Ctrl + Alt + B for the next background.
- Windows Terminal reloads only when its `settings.json` changes, never on a fragment change alone.
- `IDesktopWallpaper::SetWallpaper(monitor, "")` brings back Windows' default wallpaper (Spotlight in the Sandbox), not a solid color. Removing the picture goes through `SystemParametersInfoW(SPI_SETDESKWALLPAPER)` with an empty string.
- COM is entered once per thread and never left (`com::Apartment`): `windows-rs` caches WinRT activation factories, so a `CoUninitialize` between two `UISettings` calls crashes the process.

## Scene

Tintcast ports logic from InsaneArts/Scene, a macOS app under the MIT License. `C:\dev\Scene` is reference material only: never commit there. When Scene code is ported, add its MIT notice to `NOTICE`. Tintcast needs its own wallpapers; Scene's images keep their own licenses.

## License

GPL-3.0-or-later. Outside contributions need a contributor agreement first, so the author can still license Tintcast in other ways.
