[PRD]
# PRD: Tintcast Phase 1

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-11-09 | Arthur Jean | Initial draft |

## Problem Statement

Developers on Windows 11 who want one coherent look across their desktop and their terminal have to set it piece by piece, and nothing puts their old setup back.

1. One look means five separate changes: the wallpaper on each monitor, light or dark mode, and the accent color in Settings, then a color scheme pasted into Windows Terminal's `settings.json` and picked by hand. Omarchy does this with one shortcut on Linux, and Scene does it on macOS; Windows has no equivalent.
2. Windows tools cover one slice each. Auto Dark Mode switches mode, accent, and wallpaper, but not terminal colors. winwal and wallust cover only the terminal, need Python or a manual first selection, and write `settings.json` directly with no undo.
3. No tool returns the user's original setup exactly, and Windows cannot either. In phase 0, applying the "Unsaved theme" again through `IThemeManager2` restored nothing, and a `.theme` file had its accent changed by Windows from `#3fa34d` to `#1e8935` (`spike/FINDINGS.md`).
4. Tools that rewrite config files lose the user's comments and later edits.

**Why now:** Phase 0 (2026-11-08) proved every system call Tintcast needs on Windows 11 24H2 and 26H2:
- per-monitor wallpaper in 21 ms;
- live light or dark mode, and the exact accent, through the registry;
- exact restore from recorded values;
- Windows Terminal schemes from a fragment, with live reload;
- Zed's GPUI running with a tray icon and a global shortcut.

Windows 10 left support on 2025-10-14, so the developer audience is on Windows 11. Scene shows the product works on macOS, and Omarchy's theme folders give users a catalog they already own.

## Overview

Tintcast phase 1 is a native Windows 11 tray app, written in Rust with Zed's GPUI. It applies one theme in one step to four targets: the wallpaper on every monitor, light or dark mode, the accent color, and Windows Terminal. Win + Ctrl + Alt + Space opens a switcher, and Return applies the selected theme. Win + Ctrl + Alt + B moves to the theme's next wallpaper. The main window lists every change before it happens. A console program, `tintcast.exe`, exposes the same engine to scripts and to the Windows Sandbox acceptance suite.

The engine follows Scene's design (MIT), rewritten in Rust:
- Integrations only plan changes, as a closed set of operations.
- Before a resource's first change, the engine records its original value in a ledger, and it journals every step.
- A failed integration is rolled back on its own.
- Restore works in three ways: a value Tintcast still holds goes back to the original, and a value the user changed since is kept.
- Restore never goes through `IThemeManager2`.

Key decisions:
- **Theme format.** The native format is Omarchy's folder layout (`colors.toml` plus `backgrounds/`) with an optional `tintcast.toml`, so an Omarchy theme imports without translation.
- **Themes are data.** Nothing in a theme folder is executed.
- **Wallpapers.** Images are decoded under fixed limits and stored again as PNG files in Tintcast's own folder.
- **Accent.** The accent goes through the registry with an 8-shade palette that Tintcast builds, so it stays exact.
- **Windows Terminal.**
  - Terminal gets its schemes from a Tintcast fragment, and the fragment's `updates` entries pick the scheme.
  - Tintcast edits `settings.json`, keeping comments and formatting, only when the user's own settings would override the fragment.
- **Bundled themes.** Phase 1 ships four original themes, with wallpapers generated from their palettes. Omarchy's images have unclear licenses, so they stay in the user's own imports.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Exact restore: watched values equal before apply and after Restore original setup, in the Sandbox suite | 100% | 100%, with the suite rerun on every Windows 11 servicing update |
| Apply duration, from Return in the switcher to the last integration verified, wallpaper already stored | p95 at most 1500 ms | p95 at most 1000 ms |
| Omarchy themes imported with their palette and every wallpaper | 22 or more of the 23 themes in `C:\dev\omarchy\themes` | 95% or more of a 100-theme community corpus |
| Applies with no failed integration, read from the local journal | 99% or more of Arthur's daily applies | 99% or more across 10 or more outside testers |

## Target Users

### Windows developer who themes their setup
- **Role:** A software developer on Windows 11 who lives in Windows Terminal, PowerShell, or WSL, and wants the desktop and terminal to match. Some use a tiling window manager such as GlazeWM or komorebi.
- **Behaviors:** Changes wallpaper and mode in Settings, pastes color schemes into Terminal's `settings.json`, and keeps a dotfiles repository.
- **Pain points:** Five places to change for one look. Terminal's scheme has to be picked by hand. Windows alters the accent it is given. There is no way back to the previous setup, and tools drop the comments in config files.
- **Current workaround:** Settings by hand, plus copied Terminal schemes; Auto Dark Mode for mode and wallpaper; winwal for terminal colors.
- **Success looks like:** One shortcut applies a theme everywhere within 1.5 s, and one action puts everything back as it was.

### Omarchy user on a Windows machine
- **Role:** A developer who runs Omarchy on Linux and also uses Windows, for work or games.
- **Behaviors:** Switches themes with Super + Ctrl + Shift + Space and owns a set of Omarchy themes, some of them custom.
- **Pain points:** Their themes stop at the Linux boundary.
- **Current workaround:** Recreating the colors by hand on Windows.
- **Success looks like:** They point Tintcast at their themes folder, and nearly the same shortcut (Alt replaces Shift) applies the same look on Windows.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Omarchy (Linux):** A theme is a folder with `colors.toml` and `backgrounds/`; templates render it into each app, and one shortcut opens a menu. Tintcast uses the same format and adds a preview, exact restore, and the Windows desktop.
- **Scene (macOS, InsaneArts, MIT):** The same product on macOS, with a preview, Undo, and three-way Restore. Tintcast follows its engine design; it does not reuse Scene's images.
- **Auto Dark Mode (Windows):** Switches mode, accent, and per-monitor wallpaper on a schedule. One release reverted a wallpaper seconds after setting it (fixed in 10.4.2.29). It does not theme the terminal and has no exact restore. Scheduling is outside phase 1.
- **winwal and wallust:** Terminal only. winwal needs Python, and wallust needs a manual first selection. Both write `settings.json` directly, with no undo.
- **tinty (tinted-theming):** A Rust command-line tool for base16 and base24 schemes across terminals and editors. It does not cover the Windows desktop, and its builder crate has had 29 breaking releases.
- **Market gap:** No Windows tool applies a look to the desktop and the terminal together, shows the changes first, and restores the original exactly.

### Best Practices Applied
- Record each original value before the first write, and restore value by value, so edits the user made in between survive. Sources: Scene's ledger, and the phase 0 restore result.
- Edit JSON-with-comments files through a concrete syntax tree, which keeps comments and formatting: jsonc-parser 0.34 with the `cst` feature. Replace a file by writing a temporary file, then renaming it over the original.
- Prefer Windows Terminal fragments, which leave `settings.json` untouched. Sources: Microsoft Learn, and phase 0 steps 07 and 08.
- Treat themes as data. Parse colors into numbers, never execute theme files, reject path traversal and links, and decode images under explicit limits. The `image` crate's `Limits` sets no width or height cap by default.

*Full research sources available in project documentation.*

## Assumptions & Constraints

### Assumptions (to validate)
- **GPUI switcher window.** At rev `37faef8`, GPUI can show a borderless topmost window (`WindowKind::PopUp`, which sets `WS_EX_TOOLWINDOW | WS_EX_TOPMOST`) on the cursor's monitor, and that window gets keyboard focus from a global shortcut. Phase 0 proved activation only for a normal window. US-013 validates this.
- **Terminal settings layering.** A user value at `profiles.defaults.colorScheme` in `settings.json` overrides a fragment's `updates` entry. This is inferred from Terminal's settings layering and was not tested in phase 0. US-012 validates it.
- **Wallpaper files.** `IDesktopWallpaper` accepts PNG files stored in `%LOCALAPPDATA%\Tintcast\wallpapers`. Phase 0 used a BMP file in Pictures.
- **Registry on 26H2.** The registry route for the accent behaves on 26H2 hosts as it did in the 24H2 Sandbox. No system write ever ran on the 26300 host.
- **Screen reader.** GPUI's AccessKit adapter on Windows exposes GPUI elements to Narrator. `gpui_windows` answers `WM_GETOBJECT` through AccessKit, but Tintcast has not tested it.
- **Reload after a rename.** Replacing `settings.json` through a rename triggers Terminal's reload. Terminal's file watcher listens for renames, according to its source; phase 0 tested in-place writes only.
- **Omarchy keys.** Omarchy's `colors.toml` keys stay as listed in US-005, and legacy themes with `color0` to `color15` keys still circulate.

### Hard Constraints
- **Platform.** Windows 11 24H2 (build 26100) or later, x64 only.
- **Toolchain.** Rust 1.98.0, edition 2024, static CRT.
- **GPUI source.** GPUI comes from Zed's git repository at the rev and `[patch.crates-io]` entries that Paneflow pins, never from crates.io.
- **Code rules.** The workspace lint policy applies: `unwrap_used` and `expect_used` fail the build outside tests. Code carries no comments.
- **Testing system changes.** System changes run only in Windows Sandbox during development. On the host, only read-only commands run, unless Arthur agrees to a specific run.
- **Write locations.** Tintcast writes only to HKCU and to the allowlist in FR-11. It never writes to HKLM or ProgramData.
- **License.** GPL-3.0-or-later. Code ported from Scene keeps Scene's MIT notice in `NOTICE`.
- **No network.** Phase 1 has no telemetry, no theme downloads, and no update checks.

## Quality Gates

These commands must pass for every user story:
- `cargo fmt --check` - formatting
- `cargo clippy --workspace --all-targets -- -D warnings` - lints, including the workspace restriction lints
- `cargo test --workspace` - unit and integration tests, with system calls behind the fake services

For stories that change Windows settings (EP-003, US-016), once US-009 has landed:
- `pwsh tools/sandbox/run.ps1 -Suite <story suite>` - the suite passes in Windows Sandbox, and its report diff, log, and screenshots are inspected

For UI stories (EP-004, EP-005):
- Capture the Tintcast window with `PrintWindow` with Windows in light mode and again in dark mode, then inspect both images. Then operate the screen with the keyboard only.

## Epics & User Stories

### EP-001: Engine with ledger, journal, and exact restore

This epic ports Scene's engine design: plan, preview, apply, rollback, three-way restore, undo, and recovery. System calls sit behind a trait, so all of this logic is tested without touching Windows.

**Definition of Done:** `cargo test -p tintcast-engine` runs against the fake services and covers apply, preview, rollback of a failed integration, three-way restore, undo, and recovery of an interrupted run. The tests include ports of Scene's six engine scenarios, and `NOTICE` carries Scene's MIT notice.

#### US-001: Workspace crates and the system seam
**Description:** As the Tintcast developer, I want the phase 1 crates and a `SystemServices` trait with an in-memory fake, so that engine logic is tested without changing Windows.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given the workspace, when it builds, then it contains these crates, with dependencies pointing one way only:
  - `crates/tintcast-theme`
  - `crates/tintcast-engine`, which depends on `tintcast-theme`
  - `crates/tintcast-windows`, which depends on the engine
  - `crates/tintcast-cli` (binary `tintcast`) and `crates/tintcast-app` (binary `tintcast-app`), which depend on `tintcast-windows`
- [ ] Given the new crates, when the workspace builds, then `spike/` still builds unchanged.
- [ ] `SystemServices` covers four groups of calls:
  - HKCU registry values: read, write, and delete.
  - Wallpaper per monitor: list monitors, get, set, get position, and set position.
  - Files: read, write atomically, delete, and set the last write time.
  - The `ImmersiveColorSet` settings-change broadcast.
- [ ] A `FakeServices` type keeps state in memory, records every call, and fails a named call on request.
- [ ] Given a call configured to fail, when the engine makes it, then the caller receives an error value, and no panic occurs.
- [ ] `DataDir` resolves `%LOCALAPPDATA%\Tintcast` with these entries:
  - `ledger.json`
  - `history.json`
  - `config.toml`
  - `journal\`
  - `backups\`
  - `wallpapers\`
  - `themes\`
  - `logs\`
- [ ] Tests point `DataDir` at a temporary folder.
- [ ] Given `%LOCALAPPDATA%` is unset or not writable, when the data folder opens, then the error names the path, and nothing is written anywhere else.

#### US-002: Plan, preview, and apply with ledger and journal
**Description:** As a user, I want Tintcast to list every change before it applies a theme, and to record what each change replaces, so that nothing changes without my seeing it and everything can be put back.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] `Operation` is a closed enum with exactly these five cases:
  - `SetRegistryValue`: an HKCU key, a value name, and a DWORD, binary, or absent value.
  - `SetWallpaper`: a monitor device path and an image path.
  - `SetWallpaperPosition`.
  - `WriteManagedFile`: a path and bytes.
  - `SetJsonValue`: a file path, a JSON pointer, and a value or a removal.
- [ ] Each case has a stable resource key and a one-line summary. Resource key examples: `reg:HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`, `wallpaper:<device path>`, and `json:<path>#/profiles/defaults/colorScheme`.
- [ ] An `Integration` trait has four steps: `detect` (installed or not, with a reason), `plan` (from a theme to operations), `reload` (after an apply or a restore), and `verify`.
- [ ] Given a theme, when the engine plans, then it returns each integration's operations with their summaries, or a skip reason, and the fake services record zero writes.
- [ ] Given an apply, when an operation touches a resource for the first time, then the original value, or its absence, is in `ledger.json` before the write. A file's original bytes are in `backups\` before the write.
- [ ] Every step is journaled as pending, then as applied or failed, in `journal\<run-id>.json`, and the journal file is deleted when the run ends.
- [ ] Given an integration whose third operation fails, when apply runs, then:
  - Its first two operations are reverted, in reverse order.
  - Its new ledger entries are dropped.
  - Every other integration keeps its changes.
  - The result names the failed operation's summary and its error.
- [ ] Each apply appends a history entry with the theme id, the wallpaper index, the time, and the applied values. History keeps the latest 50 entries.
- [ ] `ledger.json`, `history.json`, and journal files are written to a temporary file, then renamed over the target. Given a test that stops between those two steps, then the previous file is intact.
- [ ] The named mutex `Local\Tintcast.Engine` serializes applies across processes. Given another apply holds it for more than 10 s, then the second apply fails with "Another apply is running."
- [ ] `ledger.json` and `history.json` carry `"version": 1`.
- [ ] `NOTICE` contains Scene's MIT copyright notice.

#### US-003: Three-way restore and undo
**Description:** As a user, I want "Restore original setup" to put back every value Tintcast changed while keeping what I changed myself since, and Undo to step back one apply, so that trying a theme carries no risk.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given a resource that still holds Tintcast's value, when Restore runs, then the resource gets its ledger original back. A file Tintcast created is deleted, and a registry value that did not exist before is deleted.
- [ ] Given a resource the user changed after Tintcast wrote it, when Restore runs, then it is left as is and reported as "Kept your change".
- [ ] Given a resource that already holds its original value, when Restore runs, then nothing is written and it is reported as unchanged.
- [ ] Restore walks the ledger newest entry first, and runs each touched integration's `reload` once at the end. It removes the restored and kept entries, and keeps failed entries with their error.
- [ ] Undo depends on the history:
  - With two or more history entries, Undo applies the previous entry again.
  - With exactly one entry, Undo does the same as Restore original setup.
  - With none, Undo reports "Nothing to undo" and writes nothing.
- [ ] Given a JSON key that the user deleted after Tintcast set it, when Restore runs, then the key is not created again.
- [ ] Tests port three of Scene's scenarios: restore returns the original bytes, restore keeps later changes, and restore deletes a file that Tintcast created.

#### US-004: Recover an interrupted apply
**Description:** As a user whose PC crashed or lost power during an apply, I want Tintcast to notice the unfinished run and offer to roll it back, so that no half-applied theme stays behind.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given a journal file left by a run that did not finish, when the engine opens, then it reports that run as interrupted, with its applied and pending steps.
- [ ] Given an interrupted run, when the user chooses Roll back, then each applied step of that run goes back to its ledger original, in reverse order, and the journal file is removed.
- [ ] Given a journal file that does not parse, when the engine opens, then the file is renamed to `<run-id>.corrupt.json` and reported, and later applies still run.
- [ ] A test simulates a crash after step 2 of 5, and Roll back restores both changed resources.

---

### EP-002: Themes: format, wallpapers, Omarchy import, and bundled set

This epic lets Tintcast read Omarchy-layout theme folders into exact colors and safe wallpapers. It also ships four original themes, so the app is useful at first launch.

**Definition of Done:** The 23 Omarchy `colors.toml` files are copied as test fixtures, with Omarchy's MIT notice, and all of them load. Importing `C:\dev\omarchy\themes` brings in 22 or more themes with every wallpaper; this run uses a scratch data folder and is recorded in `docs/STATUS.md`. The four bundled themes load and pass the contrast test.

#### US-005: Theme model and loader
**Description:** As a user, I want any theme folder in Omarchy's layout to load into exact colors, so that every target gets the same palette.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A theme is a folder with these files:
  - `colors.toml`.
  - An optional `backgrounds/` folder (png, jpg, jpeg, webp).
  - An optional `tintcast.toml`, with a display name, an author, a wallpaper position (Fill by default; also Fit, Stretch, Center, Tile, Span), and an accent override.
- [ ] `colors.toml` accepts these keys:
  - The 26 Omarchy keys: `mode`, `accent`, `selection`, `muted`, `background`, `dark_background`, `darker_background`, `lighter_background`, `foreground`, `dark_foreground`, `light_foreground`, `bright_foreground`, `red`, `yellow`, `orange`, `green`, `cyan`, `blue`, `magenta`, `brown`, `bright_red`, `bright_yellow`, `bright_green`, `bright_cyan`, `bright_blue`, `bright_magenta`.
  - The legacy keys `color0` to `color15`, `purple`, `bright_purple`, `selection_background`, and `selection_foreground`.
- [ ] Color values must be `#rgb` or `#rrggbb` and are parsed into RGBA. Unknown keys are ignored and reported as warnings.
- [ ] Terminal colors are mapped as follows:

  | Slots | Source |
  |---|---|
  | 0 | `background` |
  | 1 to 6 | `red`, `green`, `yellow`, `blue`, `magenta`, `cyan` |
  | 7 | `foreground` |
  | 8 | `muted` |
  | 9 to 14 | the `bright_*` colors, each falling back to its normal color |
  | 15 | `bright_foreground`, falling back to `foreground` |

  Legacy `color0` to `color15` override this mapping when present.
- [ ] `mode` is `light` or `dark`. When it is missing, a `light.mode` file means light. Otherwise the background's relative luminance decides: above 0.5 means light.
- [ ] A missing `accent` falls back to `blue`.
- [ ] Given a `colors.toml` without `background` or `foreground`, when it loads, then the theme is rejected with "colors.toml needs background and foreground."
- [ ] A theme is rejected, with the limit named, beyond any of these limits:
  - `colors.toml` larger than 64 KB.
  - More than 100 keys.
  - A name longer than 64 characters.
  - More than 50 wallpapers.
- [ ] No file in a theme folder is executed. `neovim.lua`, `vscode.json`, `icons.theme`, `*.tpl`, and video files are ignored in phase 1.
- [ ] Tests load all 23 Omarchy `colors.toml` fixtures.

#### US-006: Wallpaper pipeline
**Description:** As a user, I want every theme image converted safely into a file Windows can show, so that WebP backgrounds work and a hostile image cannot crash Tintcast.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] PNG, JPEG, and WebP decode under these limits:
  - Width and height at most 16384 px each.
  - At most 512 MiB allocated.
  - Files at most 64 MB.
- [ ] An animated WebP gives its first frame.
- [ ] Each image is stored again as `wallpapers\<theme-id>-<content-hash>.png`. The same source for the same theme reuses the stored file without decoding it again.
- [ ] Each theme's copy has its own file name, so a path read back from Windows identifies Tintcast's wallpaper.
- [ ] A 480 by 270 thumbnail is stored in `wallpapers\thumbs\`.
- [ ] Given a corrupt image or one over a limit, when the theme loads, then that image is rejected with its file name and reason, and the theme keeps its other wallpapers.
- [ ] At startup, stored wallpapers that the ledger, the history, and the library no longer reference are deleted.
- [ ] A 3840 by 2160 WebP converts in at most 800 ms in a release build on the development machine. The time is logged by a test.

#### US-007: Import Omarchy themes
**Description:** As an Omarchy user, I want to point Tintcast at one theme folder or at a folder of themes, so that my themes appear in Tintcast with their wallpapers.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005, US-006

**Acceptance Criteria:**
- [ ] Given a folder that contains `colors.toml`, when it is imported, then it becomes one theme. Given a folder whose subfolders contain `colors.toml`, when it is imported, then each subfolder becomes a theme.
- [ ] Given a theme folder without `colors.toml` but with `alacritty.toml`, when it is imported, then its `[colors.primary]`, `[colors.normal]`, and `[colors.bright]` tables are read instead.
- [ ] Imported themes are copied to `themes\<slug>\`: the color files, `tintcast.toml` if present, and the stored wallpapers. Deleting the source folder afterwards changes nothing.
- [ ] Symbolic links, junctions, and paths that resolve outside the source folder are skipped and reported. Nothing outside the source folder is read.
- [ ] Given a theme whose slug is already in the library, when it is imported without `--replace`, then it is skipped as "Already in your library". With `--replace`, it replaces the existing copy.
- [ ] Given a folder that holds no theme, when it is imported, then the result is "No colors.toml found in <path>" and nothing is imported.
- [ ] The result lists each theme as imported, or skipped with its reason, and each rejected wallpaper.

#### US-008: Bundled themes
**Description:** As a new user, I want themes ready at first launch, so that I can try Tintcast before I import anything.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-005, US-006

**Acceptance Criteria:**
- [ ] The binary embeds four original themes: Moondust (dark) and Daylight (light) from phase 0, plus one more dark and one more light theme. Their names differ from every Omarchy theme name, from Scene's theme names, and from Catppuccin, Dracula, Nord, Gruvbox, Tokyo Night, and Rosé Pine.
- [ ] At first use, each theme gets three 3840 by 2160 wallpapers generated from its palette. The output is deterministic: the same palette gives the same bytes.
- [ ] A test checks contrast for every bundled palette: foreground on background is at least 4.5:1, and ANSI colors 1 to 6 and 9 to 14 are each at least 3:1 against background.
- [ ] Given a bundled theme, when the user deletes it or imports over its slug, then the action is refused with "Bundled themes can't be changed."

---

### EP-003: Windows integrations and acceptance suite

This epic connects the engine to Windows: light or dark mode, accent, wallpaper, and Windows Terminal. A command-line interface and a Sandbox suite prove every change end to end.

**Definition of Done:**
- In Windows Sandbox, the suite applies two bundled themes and one imported Omarchy theme, then runs Restore original setup.
- The final `tintcast report --json` matches the initial one for every watched registry value and every monitor's wallpaper, and Terminal's `settings.json` matches byte for byte.
- Screenshots show each theme applied.

#### US-009: Command line and Sandbox acceptance runner
**Description:** As the Tintcast developer, I want `tintcast` commands over the engine and a Sandbox runner that drives them, so that every system change is verified end to end without touching my own PC.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003, US-005

**Acceptance Criteria:**
- [ ] `tintcast.exe` is a console program with these subcommands:

  | Subcommand | What it does |
  |---|---|
  | `list` | Lists the themes |
  | `plan <theme>` | Prints the preview and writes nothing |
  | `apply <theme> [--wallpaper N]` | Applies the theme |
  | `undo` | Undoes the last apply |
  | `restore` | Restores the original setup |
  | `status [--json]` | Shows the current theme, the ledger entry count, and any interrupted run |
  | `import <path> [--replace]` | Imports themes |
  | `report [--json]` | Read-only system state, ported from the probe |

- [ ] Exit codes: 0 on success, 1 when an integration failed, 2 on bad usage.
- [ ] Given an unknown theme name, when `apply` runs, then it prints "No theme named <name>. Run tintcast list." and exits with 2.
- [ ] `tools/sandbox/run.ps1 -Suite <name>` ports `spike/sandbox/launch.ps1`:
  - It builds a release, stages `tintcast.exe`, the suite script, and fixtures, and runs them in Windows Sandbox with networking off.
  - It collects the reports, screenshots, and log.
- [ ] Given a suite whose final `report --json` differs from its initial report in any watched value, when the suite ends, then the runner prints the differing values and exits with a non-zero code.
- [ ] Without a subcommand, `tintcast.exe` starts `tintcast-app.exe` from its own folder. Until EP-004 lands, it prints the usage text instead.

#### US-010: Light or dark mode and accent color
**Description:** As a user, I want the theme's mode and exact accent applied to Windows live, so that Settings, the taskbar, and apps follow the theme without signing out.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] The plan sets these values:
  - `AppsUseLightTheme` and `SystemUsesLightTheme`, from the theme's mode.
  - `AccentColorMenu`, `StartColorMenu`, and an 8-shade `AccentPalette` built from the accent, as the probe builds it.
  - The DWM values `AccentColor`, `ColorizationColor`, and `ColorizationAfterglow`.
- [ ] The plan never changes `ColorPrevalence` ("Show accent color on Start and taskbar"), which stays the user's choice.
- [ ] `reload` broadcasts `WM_SETTINGCHANGE` with `ImmersiveColorSet` through `SendMessageTimeoutW`, with a 5000 ms timeout and abort-if-hung.
- [ ] `verify` reads the accent back through `UISettings`, and in the Sandbox suite it equals the theme's accent exactly.
- [ ] The Sandbox suite screenshot shows Settings and the taskbar repainted within 5 s of the apply.
- [ ] Given a registry write that Windows denies, when apply runs, then the integration rolls back and reports "Windows blocked the change to <value name>."
- [ ] Given a Windows contrast theme is on, when the plan runs, then the integration is skipped with "A contrast theme is on, so Tintcast leaves mode and accent alone."

#### US-011: Wallpaper on every monitor
**Description:** As a user with one or more monitors, I want the theme's wallpaper on each of them, so that the whole desktop matches.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-009, US-006

**Acceptance Criteria:**
- [ ] The plan holds one `SetWallpaper` per connected monitor, named by its `IDesktopWallpaper` device path. It uses the selected wallpaper (the theme's first by default), and the position from `tintcast.toml` (Fill by default).
- [ ] `verify` reads `GetWallpaper` for each monitor and compares the path with the stored file.
- [ ] Given a monitor disconnected between apply and Restore, when Restore runs, then:
  - The connected monitors are restored.
  - The missing monitor's entry stays in the ledger and is reported as "Display not connected".
  - A later Restore with the display connected restores it.
- [ ] Given the user's background is a slideshow, Windows Spotlight, or a solid color, when the plan runs, then the preview says "Your <slideshow> stays off after Restore; Restore brings back the picture shown now." Solid colors are restored exactly through the `Background` value of `HKCU\Control Panel\Colors`.
- [ ] Given the `NoChangingWallPaper` policy is set, when the plan runs, then the integration is skipped with "A policy on this PC blocks wallpaper changes."
- [ ] Unit tests with two and three fake monitors cover apply and restore. The Sandbox suite covers one monitor.

#### US-012: Windows Terminal color schemes
**Description:** As a Windows Terminal user, I want my open tabs to take the theme's colors while my `settings.json` stays as I wrote it, so that my terminal matches without losing my settings.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] `detect` finds every `settings.json` for Store stable, Store Preview, and the unpackaged build. With none, the integration is skipped with "Windows Terminal is not installed."
- [ ] `WriteManagedFile` writes `%LOCALAPPDATA%\Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json` with one scheme named "Tintcast <theme name>". The scheme has the 16 ANSI colors, `background`, `foreground`, `cursorColor` (the accent), and `selectionBackground` (`selection`).
- [ ] Given `settings.json` has no `colorScheme` in `profiles.defaults`, when the plan runs, then:
  - The fragment adds an `updates` entry with `colorScheme` for each profile in `profiles.list` that has a `source`, and for the Windows PowerShell and Command Prompt GUIDs.
  - `settings.json` stays byte for byte unchanged.
- [ ] The plan adds `SetJsonValue` on `/profiles/defaults/colorScheme` in either of these cases:
  - `settings.json` sets `profiles.defaults.colorScheme`.
  - A visible profile has neither a `source` nor its own `colorScheme`.
- [ ] That `SetJsonValue` edit goes through jsonc-parser's concrete syntax tree, and every other byte, comments included, stays the same.
- [ ] A profile with its own `colorScheme` keeps it, and the preview lists it as "keeps its own scheme".
- [ ] `reload` sets the last write time of `settings.json` to now without changing its content. In the Sandbox, the open tab is repainted within 5 s.
- [ ] Edits to `settings.json` are written to a temporary file in the same folder, then renamed over it. The Sandbox suite confirms that Terminal reloads after the rename.
- [ ] Given `settings.json` does not parse as JSONC, when the plan runs, then nothing is written, and the integration reports "Windows Terminal's settings.json has a syntax error at line <L>, column <C>."
- [ ] The Sandbox suite tests whether a user's `profiles.defaults.colorScheme` overrides a fragment's `updates` entry, and the result is recorded in `docs/STATUS.md`.
- [ ] Restore removes the fragment and reverts the JSON edit, and in the Sandbox the final `settings.json` bytes equal the original.

---

### EP-004: Tray, shortcuts, and switcher

This epic makes Tintcast an always-ready tray app: two global shortcuts and a keyboard-first switcher on the current monitor.

**Definition of Done:**
- In the Sandbox, the app starts in the tray.
- Win + Ctrl + Alt + Space opens the switcher within 150 ms p95, and Return applies a theme.
- Win + Ctrl + Alt + B changes the wallpaper.
- Idle CPU stays under 0.1% over 60 s.
- Both window captures (light and dark) have been inspected.

#### US-013: Validate assumption: GPUI shows a focused switcher window from a global shortcut
**Description:** As the Tintcast developer, I want proof that GPUI on Windows can show a borderless topmost window that takes keyboard focus from a global shortcut, so that the switcher design rests on a tested base.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A throwaway build in `spike/` opens a `WindowKind::PopUp` window, 960 by 400 logical px, centered on the monitor under the cursor, when Win + Ctrl + Alt + Space is pressed.
- [ ] Without a click, the window receives arrow keys and typed text, and the log shows at most 150 ms from the key press to the first frame.
- [ ] The window closes when it loses focus, by a click elsewhere or by Alt + Tab.
- [ ] The log records whether Narrator reads a labeled element through GPUI's AccessKit adapter.
- [ ] Given `WindowKind::PopUp` fails any check, then a normal window without a title bar, made topmost through `SetWindowPos`, is tested the same way. The approach that passes is recorded in `spike/FINDINGS.md`.

#### US-014: Tray app with a single instance and global shortcuts
**Description:** As a user, I want Tintcast to wait in the tray with its shortcuts ready while using no CPU when idle, so that it is always one key away and never slows my PC.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003, US-013

**Acceptance Criteria:**
- [ ] `tintcast-app.exe` runs the tray app. A second start finds the named mutex `Local\Tintcast.Instance`, asks the running instance to open its main window, and exits.
- [ ] The tray icon:
  - Its tooltip reads "Tintcast: <current theme>".
  - Its menu has Open Tintcast, Undo, Restore original setup…, and Quit Tintcast. US-016 adds Next background.
  - A left click opens the switcher.
- [ ] The app registers Win + Ctrl + Alt + Space for the switcher and Win + Ctrl + Alt + B for the next background.
- [ ] Given a shortcut that another app has taken, when the app starts, then the tray menu shows "Switcher shortcut unavailable: taken by another app", and the app keeps running.
- [ ] Tray, menu, and shortcut events reach the GPUI app through event handlers that wake its executor, with no polling timer. Idle CPU averages under 0.1% over 60 s, and the idle private working set is at most 120 MB.
- [ ] Closing the main window hides it. Quit unregisters the shortcuts and removes the tray icon.
- [ ] Given Explorer restarts, then the tray icon is back within 2 s, after the `TaskbarCreated` message.

#### US-015: Theme switcher
**Description:** As a user, I want a keyboard-first switcher on the monitor I am using, so that I change theme in two keystrokes without leaving my work.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-014, US-008

**Acceptance Criteria:**
- [ ] The switcher opens on the monitor under the cursor, centered, borderless, and topmost, using the approach US-013 recorded, with the current theme selected.
- [ ] Each theme is a card with a 16:9 wallpaper thumbnail, a strip of its 16 ANSI colors, and its name, in alphabetical order. The current theme is marked "Current".
- [ ] Keys:
  - Left and Right arrows move the selection and stop at the ends.
  - Home and End jump to the first and last card.
  - 1 to 9 select the Nth visible card.
  - Typing filters the cards by name (case-insensitive substring), and Backspace edits the filter.
  - Arrows, Return, and Esc keep their roles while a filter is typed.
- [ ] Return, or a click on a card, applies the selected theme and closes the switcher, and the tray tooltip updates.
- [ ] Esc clears a non-empty filter; with an empty filter, Esc closes the switcher. Losing focus closes it without applying.
- [ ] Ctrl + Z runs Undo.
- [ ] Given a filter that matches nothing, then the switcher shows "No theme matches '<text>'", and Return does nothing.
- [ ] Given an apply with a failed integration, then the switcher closes, and the main window opens on the result.
- [ ] With 200 themes in the library, the switcher opens within 150 ms p95, and thumbnails load as they scroll into view.
- [ ] With Windows "Animation effects" off, the switcher shows no transitions.
- [ ] Each card has the accessible name "<theme name>, <dark or light> theme", if US-013 found AccessKit working.

#### US-016: Next background
**Description:** As a user, I want a shortcut that moves to the current theme's next wallpaper, so that I can change the backdrop without changing the theme.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-011, US-014

**Acceptance Criteria:**
- [ ] Win + Ctrl + Alt + B, or the tray item Next background, applies the current theme's next wallpaper to every monitor through the wallpaper integration alone, wrapping around after the last one.
- [ ] Each step adds a history entry that Undo reverts, and the current theme stays the same.
- [ ] Given the current theme has one wallpaper, then nothing is applied, and the tray tooltip shows "<theme name> has one wallpaper" for 5 s.
- [ ] Given no theme has been applied, then nothing is applied, and the tooltip shows "Apply a theme first" for 5 s.

---

### EP-005: Main window: library, preview, and settings

This epic gives the full view: browse, preview the exact changes, apply, undo, restore, import, and change shortcuts.

**Definition of Done:**
- From the main window alone, using the keyboard only, a user can do each of these: browse, preview, apply, undo, restore, import, change a shortcut, and turn on start at sign-in.
- Captures of the window with Windows in light mode and in dark mode have been inspected.

#### US-017: Library, preview, and apply
**Description:** As a user, I want to see a theme and the exact list of changes before I apply it, and a clear report afterwards, so that I always know what Tintcast did to my PC.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-014, US-008

**Acceptance Criteria:**
- [ ] The main window opens at 1100 by 720 logical px, with a minimum of 760 by 520. On the left is the theme library with a search field; on the right are the selected theme's details.
- [ ] The detail view shows:
  - The theme's wallpapers, to pick one.
  - The palette.
  - A terminal sample in the theme's 16 colors.
  - A "Changes" list: the engine's plan grouped by integration, with skip reasons.
- [ ] "Apply theme" applies the theme, then shows each integration's outcome: applied; skipped, with its reason; or failed, with its error and "rolled back".
- [ ] The header has Undo. With an empty history, Undo is disabled and its tooltip says "Nothing to undo".
- [ ] "Restore original setup…" opens a confirmation:
  - Its title names the count, as in "Restore 14 settings to how they were before Tintcast?"
  - Its buttons are "Restore setup" and "Cancel", and focus starts on Cancel.
  - Afterwards, the result lists what was restored, what was kept as "Kept your change", and what failed.
- [ ] Given an interrupted run at startup, then a banner offers "Roll back" (US-004) and "Keep as is".
- [ ] Tintcast's own window follows Windows light or dark mode, with text contrast of at least 4.5:1 in both.
- [ ] Keyboard use:
  - Tab moves from the library to the details, then to the actions.
  - Arrow keys move through the library grid.
  - Return selects a theme.

#### US-018: Import from the main window
**Description:** As an Omarchy user, I want to import my themes from the main window, so that I do not need the command line.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007, US-017

**Acceptance Criteria:**
- [ ] "Import themes…" opens the Windows folder picker (`IFileOpenDialog` with `FOS_PICKFOLDERS`), runs the import, and shows its result list.
- [ ] Given a theme already in the library, then a dialog asks "Replace '<theme name>'?" with the buttons "Replace theme" and "Skip".
- [ ] Imported themes appear in the library and in the switcher without a restart.
- [ ] "Remove from library" deletes an imported theme. For a bundled theme, it is disabled. Removing the current theme leaves the applied settings and the ledger as they are.

#### US-019: Settings: shortcuts and start at sign-in
**Description:** As a user whose PC already uses a default shortcut, I want to choose my own shortcuts and have Tintcast start when I sign in, so that it fits my setup.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-014

**Acceptance Criteria:**
- [ ] The settings view records a new shortcut for the switcher and one for the next background when the user presses the keys, and Esc cancels the recording.
- [ ] A shortcut needs Win or Ctrl plus one key that is not a modifier.
- [ ] Given a shortcut that `RegisterHotKey` refuses, then the view shows "Taken by Windows or another app", and the previous shortcut stays.
- [ ] Given the same shortcut for both actions, then it is refused with "Already used for <action>".
- [ ] "Reset to default" brings back Win + Ctrl + Alt + Space and Win + Ctrl + Alt + B.
- [ ] "Start Tintcast when you sign in" sets the `Tintcast` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` to `"<path>\tintcast-app.exe" --background`, or removes it. With `--background`, the app starts in the tray without a window.
- [ ] Settings are saved to `config.toml` with an atomic write. Given a `config.toml` that does not parse, then it is kept as `config.corrupt.toml`, the defaults apply, and the settings view shows a notice.

---

## Functional Requirements

- FR-01: In one action, the system must apply a theme to four targets: the wallpaper on every connected monitor, light or dark mode, the accent color, and Windows Terminal.
- FR-02: Before an apply started from the main window, and for `tintcast plan`, the system must show every planned change.
- FR-03: Before a resource's first change, the system must record its original value or its absence.
- FR-04: Restore must return every resource that still holds Tintcast's value to its original, and must keep any resource the user changed since.
- FR-05: Undo must apply the previous history entry again.
- FR-06: When an integration fails, the system must roll back that integration alone.
- FR-07: The system must detect an interrupted apply at the next start and offer to roll it back.
- FR-08: The system must import Omarchy theme folders by copying them into its library.
- FR-09: The system must NOT execute any file from a theme folder.
- FR-10: The system must NOT write to HKLM, to ProgramData, or to any path outside FR-11's allowlist.
- FR-11: The write allowlist has three parts:
  - Folders: `%LOCALAPPDATA%\Tintcast\` and `%LOCALAPPDATA%\Microsoft\Windows Terminal\Fragments\Tintcast\`.
  - The Terminal `settings.json` files that `detect` found.
  - These HKCU locations: `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`, `Software\Microsoft\Windows\CurrentVersion\Explorer\Accent`, `Software\Microsoft\Windows\DWM`, `Control Panel\Colors\Background`, and the `Tintcast` value under `Software\Microsoft\Windows\CurrentVersion\Run`.
- FR-12: The system must NOT restore through `IThemeManager2`.
- FR-13: The default global shortcuts must be Win + Ctrl + Alt + Space for the switcher and Win + Ctrl + Alt + B for the next background, and both must be changeable.
- FR-14: Only one instance of the app may run per user session.
- FR-15: The system must NOT open network connections in phase 1.

## Non-Functional Requirements

- **Performance:**
  - Switcher: from the shortcut press to its first frame, at most 150 ms p95.
  - Apply: from start to last verify, at most 1500 ms p95, with the wallpaper already stored.
  - Wallpaper conversion: a 3840 by 2160 WebP in at most 800 ms.
  - Idle use: CPU under 0.1% averaged over 60 s, and a private working set of at most 120 MB.
- **Security:**
  - Writes happen only inside FR-11's allowlist, and no file from a theme is ever executed.
  - Images: at most 16384 px per side, 512 MiB of allocation, and 64 MB per file.
  - `colors.toml`: at most 64 KB and 100 keys.
  - Imports never follow a link outside their source folder.
  - Nothing goes over the network.
- **Accessibility:**
  - Every action in the switcher and the main window works from the keyboard alone.
  - Text contrast in Tintcast's own UI is at least 4.5:1 in both light and dark mode.
  - With Windows animation effects off, the UI shows no transitions.
  - Switcher cards and buttons have accessible names, if US-013 confirms AccessKit works.
- **Scalability:** The library holds up to 500 themes, and the switcher keeps its 150 ms p95 open time with 200 themes.
- **Reliability:**
  - After a crash or power loss, 100% of interrupted applies are detected at the next start.
  - Ledger, history, and config files are always replaced by a rename, never left partly written.
  - Each failed integration is rolled back while the others are kept.
  - The Sandbox suite passes 10 runs out of 10.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Empty search | The switcher filter matches no theme | Shows an empty state; Return does nothing | "No theme matches '<text>'" |
| 2 | First launch | No theme has been applied yet | The four bundled themes are listed; Undo and Next background do nothing | "Apply a theme first" |
| 3 | Apply in progress | An apply takes longer than 300 ms | The Apply button shows a spinner and stays pressed until each integration reports | "Applying <theme>…" |
| 4 | Integration failure | A registry write is denied by policy | That integration rolls back; the others keep their changes | "Windows blocked the change to <value name>." |
| 5 | Broken Terminal settings | `settings.json` does not parse as JSONC | The Terminal integration writes nothing; the others apply | "Windows Terminal's settings.json has a syntax error at line <L>, column <C>." |
| 6 | Terminal missing | No Terminal `settings.json` is found | The integration is skipped | "Windows Terminal is not installed." |
| 7 | User change after apply | The user picks another accent in Settings, then runs Restore | The accent is kept; everything else is restored | "Kept your change: accent color" |
| 8 | Concurrent apply | The command line applies while the app is applying | The second apply waits up to 10 s, then fails | "Another apply is running." |
| 9 | Monitor unplugged | A display is disconnected before Restore | Connected displays are restored, and the missing one's entry is kept | "Display not connected: its wallpaper will be restored when it's back." |
| 10 | Hostile or huge image | A 20000 px WebP, or a corrupt file | That image is rejected; the theme keeps its other wallpapers | "<file> was skipped: larger than 16384 px." |
| 11 | Path traversal in an import | A theme folder holds a junction pointing to `C:\Windows` | The entry is skipped and nothing outside the source is read | "<entry> was skipped: it points outside the theme folder." |
| 12 | Crash during an apply | Power is lost after step 2 of 5 | At the next start, a banner offers to roll back | "Tintcast was interrupted while applying <theme>." |
| 13 | Shortcut taken | Another app holds Win + Ctrl + Alt + Space | The app runs, and the tray menu flags the shortcut | "Switcher shortcut unavailable: taken by another app" |
| 14 | Slideshow or Spotlight background | The user's background is not a single picture | The preview warns; Restore brings back the current picture | "Your slideshow stays off after Restore; Restore brings back the picture shown now." |
| 15 | Contrast theme on | A Windows contrast theme is active | Mode and accent are skipped; the rest applies | "A contrast theme is on, so Tintcast leaves mode and accent alone." |
| 16 | Corrupt app files | `ledger.json` or `config.toml` does not parse | The file is kept with a `.corrupt` suffix and reported; applies are blocked only when the ledger is corrupt, because Restore depends on it | "Tintcast can't read its restore records in <path>. Restore is unavailable until the file is fixed." |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI's `PopUp` window does not get keyboard focus from a global shortcut on Windows | Med | High | US-013 tests it first, with a fallback window style measured the same way |
| 2 | A Windows update changes how the accent or mode registry values are read | Low | High | `verify` reads values back after each apply; the Sandbox suite reruns on each servicing update; the `.theme` route stays a documented fallback |
| 3 | The Sandbox build (24H2) behaves differently from Arthur's host (26H2) | Med | Med | One run on the host per milestone, with Arthur's consent and a Restore at the end |
| 4 | Terminal changes how fragments or `updates` work | Low | Med | The `settings.json` edit covers the same need; the suite pins the Terminal version installed on the host |
| 5 | SmartScreen or antivirus flags an unsigned binary that writes the registry | Med | Med | Static CRT, no packing or obfuscation; code signing is an open question before outside testers |
| 6 | The ledger is corrupted, so Restore cannot run | Low | High | Writes by rename, a version field, corrupt files kept aside, and applies blocked while the ledger is unreadable |
| 7 | The user changes Terminal settings in its UI while Tintcast writes | Low | Med | Read, modify, and rename in one step; three-way Restore keeps the user's later values |
| 8 | GPUI API changes when the Zed rev is bumped | Med | Med | The rev stays pinned and is bumped together with Paneflow |
| 9 | 59 points exceed the time available | Med | Med | P1 stories (US-016, US-018, US-019) are cut first; P0 alone delivers the core loop |
| 10 | Bundled wallpapers generated from palettes look plain | High | Med | Open question 1: real artwork before any public release |

## Non-Goals

What this version explicitly does NOT include:

- **Code editors.** VS Code, Neovim, and other editors wait for phase 2. Scene's JSONC and managed-block patterns are ready to port then.
- **Automatic switching.** No switching on a schedule or at sunrise (Auto Dark Mode's core feature), and no following Windows light or dark mode automatically.
- **Terminal window theme.** The Terminal tab row stays dark under a light scheme, as phase 0 showed.
- **Online services.** No online theme gallery, downloads, auto-update, or telemetry.
- **Packaging.** No installer, MSIX, winget package, or code signing; see the open questions.
- **Other platforms.** No Windows 10 and no ARM64 build.
- **Other look settings.** No changes to `ColorPrevalence`, cursors, sounds, icons (Omarchy's `icons.theme`), or the lock screen. No `.theme` apply except as a documented fallback.
- **Slideshow and Spotlight.** Restoring a slideshow or Windows Spotlight background exactly is out of phase 1.
- **Theme editor.** Users cannot create or edit themes inside Tintcast.

## Files NOT to Modify

- `C:\dev\Scene\**`: InsaneArts' repository, reference only; never commit there.
- `C:\dev\omarchy\**`: reference and import source only. Copy fixtures out of it; never edit it.
- `spike/**`: the phase 0 evidence. US-013 is the only exception: it adds one spike and rows to `spike/FINDINGS.md`.
- `spike/sandbox/cache/**`: downloaded Windows Terminal packages.
- In `Cargo.toml`, the GPUI revs and the `[patch.crates-io]` block, except in a coordinated bump with Paneflow.
- `LICENSE` and `rust-toolchain.toml`.

## Technical Considerations

Frame as questions for engineering input, not mandates:

- **Architecture:** The recommendation is five crates with one-way dependencies:

  | Crate | Role |
  |---|---|
  | `tintcast-theme` | Pure data |
  | `tintcast-engine` | Pure logic and the `SystemServices` seam |
  | `tintcast-windows` | Real services and integrations |
  | `tintcast-cli` | The command line |
  | `tintcast-app` | The GPUI app |

  Engineering to confirm whether the integrations should live in their own crate, apart from the services.
- **Threading:** `IDesktopWallpaper` and `UISettings` are COM objects, and the probe ran them on an STA thread. The recommendation is to run the engine on one dedicated STA worker thread that the UI reaches through a channel. Is a multithreaded (MTA) apartment safe for these calls instead?
- **Event delivery:** The plan is to replace phase 0's 30 ms polling: install `set_event_handler` on the tray, menu, and hotkey crates, and forward events into an async channel that a `cx.spawn` task awaits. Does GPUI's Windows dispatcher wake that task when the sender runs on another thread?
- **Binaries:** The recommendation is two programs: a console `tintcast.exe` and a windowed `tintcast-app.exe`. A single windowed exe calling `AttachConsole` returns the shell prompt before its output appears, which breaks scripts. Confirm the trade-off.
- **Data Model:**
  - Ledger, history, and journal files are JSON through serde, with `"version": 1`.
  - Settings are TOML.
  - Should the ledger store file backups by content hash, so repeated applies do not duplicate bytes?
- **Dependencies:**

  | Crate | Version or features |
  |---|---|
  | jsonc-parser | 0.34, `cst` feature |
  | image | `png`, `jpeg`, and `webp` features only |
  | toml, serde, serde_json | |
  | windows | 0.62.2 |
  | windows-registry | 0.6.1 |
  | global-hotkey | 0.9 |
  | tray-icon | 0.26 |

  tinted-builder was considered and rejected: 29 breaking releases, and a YAML scheme format Tintcast does not need.
- **Wallpaper format:** PNG keeps every pixel, but a 4K file weighs 10 to 25 MB. JPEG at quality 95 is about 2 MB. Does Windows compress again whatever it receives (its `TranscodedWallpaper` cache)? If it does, JPEG costs nothing visible.
- **Migration:** None; this is a new product. Restore original setup is the rollback for users. Backward compatibility of the data folder starts at `"version": 1`.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Exact restore of watched values | Phase 0 restore of registry and wallpaper values: 100% (Sandbox step 09) | 100% including Terminal `settings.json` bytes | Month-1 | Sandbox suite report diff |
| Apply duration p95, wallpaper stored | N/A (new); phase 0 parts took 21 to 134 ms each | At most 1500 ms (Month-1), at most 1000 ms (Month-6) | Month-1 and Month-6 | Journal timestamps over 20 Sandbox applies |
| Switcher open time p95 | N/A (new) | At most 150 ms with 200 themes | Month-1 | Timestamp log, from shortcut press to first frame |
| Omarchy import success | Scene imports 107 of 108 community `colors.toml` files | 22 or more of 23 (Month-1); 95% or more of 100 (Month-6) | Month-1 and Month-6 | Import result list on the corpus |
| Idle cost | Phase 0 shell polled every 30 ms (not measured) | CPU under 0.1%, at most 120 MB | Month-1 | Windows performance counters over 60 s |
| Applies without a failed integration | N/A (new) | 99% or more | Month-6 | Local journal outcomes, read by `tintcast status --json` |

## Open Questions

- **Wallpapers for the bundled themes.** Palette-generated wallpapers are a stopgap. Should the real artwork be commissioned, photographed, or generated with clear rights? Arthur decides before the first public build. The Month-6 tester goal depends on it.
- **Code signing.** Should Tintcast be signed (for example Azure Trusted Signing, about $10 a month) before outside testers, or tested unsigned behind a SmartScreen warning? Arthur decides before the first outside tester.
- **Distribution.** Portable zip, winget, or MSIX for the first testers? Arthur decides with the signing question. It shapes how start-at-sign-in finds the executable.
- **Names for the third and fourth bundled themes.** Arthur picks them during US-008.
- **Accent on Start and the taskbar.** Should a theme also turn on "Show accent color on Start and taskbar"? Phase 1 says no, to respect the user's choice. Arthur revisits after two weeks of daily use.
- **Contributor agreement.** Which agreement and tooling for outside pull requests? Arthur decides before the repository goes public. It keeps the option to license Tintcast in other ways.
[/PRD]
