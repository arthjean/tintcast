#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use tintcast_engine::fake::{CallKind, FakeServices};
use tintcast_engine::{
    ApplyRequest, DISPLAY_NOT_CONNECTED, DataDir, Engine, EngineLock, Outcome, Palette,
    PlannedOutcome, RegistryValue, RestoreResult, Rgba, SystemServices, Theme, Wallpaper,
    WallpaperPosition,
};
use tintcast_windows::fake::FakeDesktop;
use tintcast_windows::keys::{ACCENT, BACKGROUND, COLORS, DWM, PERSONALIZE};
use tintcast_windows::terminal::{COMMAND_PROMPT, WINDOWS_POWERSHELL, fragment_path};
use tintcast_windows::{
    BackgroundKind, Desktop, System, appearance, describe, registry_write_error, terminal,
    wallpaper,
};

static LOCKS: AtomicUsize = AtomicUsize::new(0);

const LOCAL: &str = r"C:\Users\u\AppData\Local";
const ONE: &str = r"\\?\DISPLAY#ONE";
const TWO: &str = r"\\?\DISPLAY#TWO";
const THREE: &str = r"\\?\DISPLAY#THREE";

struct Rig {
    _temp: tempfile::TempDir,
    fake: Arc<FakeServices>,
    desktop: Arc<FakeDesktop>,
    engine: Engine,
}

fn settings_path() -> PathBuf {
    Path::new(LOCAL)
        .join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json")
}

fn fragment() -> PathBuf {
    fragment_path(Path::new(LOCAL))
}

fn rig(monitors: &[&str]) -> Rig {
    let temp = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeServices::with_monitors(monitors));
    for (index, monitor) in monitors.iter().enumerate() {
        fake.set_monitor_wallpaper(
            monitor,
            Some(Path::new(&format!(
                r"C:\Windows\Web\Wallpaper\img{index}.jpg"
            ))),
        );
    }
    fake.set_registry(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    fake.set_registry(
        PERSONALIZE,
        "SystemUsesLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    fake.set_registry(
        PERSONALIZE,
        "ColorPrevalence",
        Some(RegistryValue::Dword(0)),
    );
    fake.set_registry(DWM, "AccentColor", Some(RegistryValue::Dword(0xffd4_7800)));
    fake.set_registry(
        ACCENT,
        "AccentPalette",
        Some(RegistryValue::Binary(vec![0x11; 32])),
    );
    fake.set_registry(
        COLORS,
        BACKGROUND,
        Some(RegistryValue::String("0 0 0".into())),
    );
    let desktop = Arc::new(FakeDesktop::new(LOCAL).following(Arc::clone(&fake)));
    let system = System {
        services: fake.clone(),
        desktop: desktop.clone() as Arc<dyn Desktop>,
    };
    let data = DataDir::at(temp.path().join("Tintcast")).unwrap();
    let lock = EngineLock::new(
        format!(
            r"Local\Tintcast.Engine.windows-test-{}-{}",
            std::process::id(),
            LOCKS.fetch_add(1, Ordering::Relaxed)
        ),
        Duration::from_secs(5),
    );
    let engine = system.engine(data).with_lock(lock);
    Rig {
        _temp: temp,
        fake,
        desktop,
        engine,
    }
}

fn theme(id: &str, name: &str, background: Rgba, foreground: Rgba, accent: Rgba) -> Theme {
    let mut theme = Theme::new(
        id,
        name,
        Palette::new(background, foreground).with_accent(accent),
    );
    theme.wallpapers = (1..=3)
        .map(|n| Wallpaper {
            source: format!("{n}.png"),
            image: PathBuf::from(format!(r"{LOCAL}\Tintcast\wallpapers\{id}-{n}.png")),
            thumbnail: PathBuf::from(format!(r"{LOCAL}\Tintcast\wallpapers\thumbs\{id}-{n}.png")),
        })
        .collect();
    theme
}

fn ironwood() -> Theme {
    theme(
        "ironwood",
        "Ironwood",
        Rgba::rgb(0x1a, 0x1b, 0x1d),
        Rgba::rgb(0xdc, 0xd8, 0xd0),
        Rgba::rgb(0xd9, 0x4f, 0x70),
    )
}

fn linen() -> Theme {
    theme(
        "linen",
        "Linen",
        Rgba::rgb(0xf6, 0xf1, 0xe7),
        Rgba::rgb(0x2b, 0x2d, 0x31),
        Rgba::rgb(0x3b, 0x6a, 0xa8),
    )
}

fn dword(value: u32) -> Option<RegistryValue> {
    Some(RegistryValue::Dword(value))
}

fn snapshot(fake: &FakeServices) -> Vec<String> {
    let mut values = Vec::new();
    for (key, name) in tintcast_windows::keys::WATCHED {
        values.push(format!("{key}\\{name}={:?}", fake.registry(key, name)));
    }
    for monitor in [ONE, TWO, THREE] {
        values.push(format!("{monitor}={:?}", fake.monitor_wallpaper(monitor)));
    }
    values.push(format!("position={:?}", fake.position()));
    values.push(format!("settings={:?}", fake.file(&settings_path())));
    values.push(format!("fragment={:?}", fake.file(&fragment())));
    values
}

fn outcome<'a>(report: &'a tintcast_engine::ApplyReport, id: &str) -> &'a Outcome {
    &report.result(id).unwrap().outcome
}

#[test]
fn the_plan_sets_mode_and_the_exact_accent_without_writing() {
    let rig = rig(&[ONE]);
    let plan = rig.engine.plan(&ApplyRequest::new(ironwood()));
    assert!(rig.fake.writes().is_empty());
    let appearance = plan.iter().find(|p| p.id == appearance::ID).unwrap();
    let PlannedOutcome::Changes { operations, .. } = &appearance.outcome else {
        panic!("{:?}", appearance.outcome);
    };
    let names: Vec<_> = operations
        .iter()
        .map(|op| op.resource().rsplit('\\').next().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "AppsUseLightTheme",
            "SystemUsesLightTheme",
            "AccentPalette",
            "AccentColorMenu",
            "StartColorMenu",
            "AccentColor",
            "ColorizationColor",
            "ColorizationAfterglow",
        ]
    );
    assert!(!names.iter().any(|n| n == "ColorPrevalence"));
}

#[test]
fn applying_sets_the_registry_broadcasts_and_verifies_the_accent() {
    let rig = rig(&[ONE]);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    assert_eq!(outcome(&report, appearance::ID), &Outcome::Applied);
    assert_eq!(
        rig.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        dword(0)
    );
    assert_eq!(
        rig.fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        dword(0)
    );
    assert_eq!(rig.fake.registry(DWM, "AccentColor"), dword(0xff70_4fd9));
    assert_eq!(
        rig.fake.registry(ACCENT, "AccentColorMenu"),
        dword(0xff70_4fd9)
    );
    assert_eq!(
        rig.fake.registry(DWM, "ColorizationColor"),
        dword(0xc4d9_4f70)
    );
    assert_eq!(
        rig.fake.registry(DWM, "ColorizationAfterglow"),
        dword(0xc4d9_4f70)
    );
    assert_eq!(
        rig.fake.registry(ACCENT, "AccentPalette"),
        Some(RegistryValue::Binary(appearance::palette_bytes(Rgba::rgb(
            0xd9, 0x4f, 0x70
        ))))
    );
    assert_eq!(rig.fake.registry(PERSONALIZE, "ColorPrevalence"), dword(0));
    assert_eq!(rig.fake.broadcasts(), 1);
    assert_eq!(
        rig.desktop.accent_color().unwrap(),
        Rgba::rgb(0xd9, 0x4f, 0x70)
    );
}

#[test]
fn an_accent_that_windows_changes_is_reported_as_unverified() {
    let rig = rig(&[ONE]);
    rig.desktop
        .set_accent(Some(Ok(Rgba::rgb(0x1e, 0x89, 0x35))));
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let Outcome::Unverified(reason) = outcome(&report, appearance::ID) else {
        panic!("{report:?}");
    };
    assert_eq!(reason, "Windows shows the accent as #1e8935, not #d94f70");
}

#[test]
fn a_denied_registry_write_rolls_back_mode_and_accent_alone() {
    let rig = rig(&[ONE]);
    let before = snapshot(&rig.fake);
    let denied = registry_write_error("ColorizationColor", 0x8007_0005, "Access is denied.");
    rig.fake.fail(
        CallKind::WriteRegistry,
        Some("ColorizationColor"),
        &denied.to_string(),
    );
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let failed = outcome(&report, appearance::ID);
    assert!(failed.is_failure());
    assert!(
        failed
            .message()
            .contains("Windows blocked the change to ColorizationColor."),
        "{}",
        failed.message()
    );
    assert!(
        failed
            .message()
            .ends_with("Tintcast rolled back this part.")
    );
    let after = snapshot(&rig.fake);
    assert_eq!(after[..11], before[..11]);
    assert_eq!(outcome(&report, wallpaper::ID), &Outcome::Applied);
    assert_eq!(
        rig.fake.monitor_wallpaper(ONE),
        Some(PathBuf::from(format!(
            r"{LOCAL}\Tintcast\wallpapers\ironwood-1.png"
        )))
    );
    assert!(report.failed());
}

fn accent_state(get: impl Fn(&str, &str) -> Option<RegistryValue>) -> String {
    format!(
        "mode {:?}, palette {:?}, menu {:?}, start {:?}, dwm {:?} {:?} {:?}",
        get(PERSONALIZE, "SystemUsesLightTheme"),
        get(ACCENT, "AccentPalette"),
        get(ACCENT, "AccentColorMenu"),
        get(ACCENT, "StartColorMenu"),
        get(DWM, "AccentColor"),
        get(DWM, "ColorizationColor"),
        get(DWM, "ColorizationAfterglow"),
    )
}

fn shell_mirrors_the_palette_into_dwm(fake: &FakeServices) -> Arc<Mutex<Vec<String>>> {
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let observed = seen.clone();
    fake.on_commit(move |registry| {
        let get = |key: &str, name: &str| registry.get(&(key.to_owned(), name.to_owned())).cloned();
        observed.lock().unwrap().push(accent_state(get));
        let Some(RegistryValue::Binary(palette)) = get(ACCENT, "AccentPalette") else {
            return;
        };
        let [r, g, b] = [palette[12], palette[13], palette[14]].map(u32::from);
        let colorization = RegistryValue::Dword(0xc400_0000 | r << 16 | g << 8 | b);
        let dwm = |name: &str| (DWM.to_owned(), name.to_owned());
        registry.insert(
            dwm("AccentColor"),
            RegistryValue::Dword(0xff00_0000 | b << 16 | g << 8 | r),
        );
        registry.insert(dwm("ColorizationColor"), colorization.clone());
        registry.insert(dwm("ColorizationAfterglow"), colorization);
    });
    seen
}

#[test]
fn the_shell_mirroring_the_accent_never_sees_half_a_theme_and_restore_keeps_the_user_dwm_values() {
    let rig = rig(&[ONE]);
    let mut palette = vec![0; 32];
    palette[12..15].copy_from_slice(&[0x00, 0x78, 0xd4]);
    rig.fake.set_registry(
        ACCENT,
        "AccentPalette",
        Some(RegistryValue::Binary(palette)),
    );
    rig.fake
        .set_registry(ACCENT, "AccentColorMenu", dword(0xffd4_7800));
    rig.fake
        .set_registry(ACCENT, "StartColorMenu", dword(0xffc0_6700));
    rig.fake
        .set_registry(DWM, "ColorizationColor", dword(0xc400_78d4));
    rig.fake
        .set_registry(DWM, "ColorizationAfterglow", dword(0xc400_78d4));
    let before = snapshot(&rig.fake);
    let registry = |key: &str, name: &str| rig.fake.registry(key, name);
    let user = accent_state(registry);
    let seen = shell_mirrors_the_palette_into_dwm(&rig.fake);

    rig.engine.apply(&ApplyRequest::new(linen())).unwrap();
    let light = accent_state(registry);
    let colorization = rig
        .engine
        .ledger_entries()
        .unwrap()
        .into_iter()
        .find(|e| e.resource.ends_with(r"DWM\ColorizationColor"))
        .unwrap();
    assert_eq!(
        colorization.original,
        tintcast_engine::ResourceState::Registry {
            value: RegistryValue::Dword(0xc400_78d4)
        }
    );
    rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let dark = accent_state(registry);
    let report = rig.engine.restore().unwrap();

    assert!(
        report
            .items
            .iter()
            .all(|i| i.result == RestoreResult::Restored),
        "{report:?}"
    );
    assert_eq!(*seen.lock().unwrap(), [light, dark, user]);
    assert_eq!(snapshot(&rig.fake), before);
}

#[test]
fn a_contrast_theme_skips_mode_and_accent() {
    let rig = rig(&[ONE]);
    rig.desktop.set_contrast_theme(true);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    assert_eq!(
        outcome(&report, appearance::ID),
        &Outcome::Skipped(
            "A contrast theme is on, so Tintcast leaves mode and accent alone.".into()
        )
    );
    assert_eq!(
        rig.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        dword(1)
    );
    assert_eq!(outcome(&report, wallpaper::ID), &Outcome::Applied);
}

#[test]
fn restore_brings_back_every_value_and_keeps_an_accent_the_user_picked() {
    let rig = rig(&[ONE]);
    let before = snapshot(&rig.fake);
    rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    rig.engine.apply(&ApplyRequest::new(linen())).unwrap();
    rig.fake
        .set_registry(DWM, "AccentColor", dword(0xff00_ff00));
    let restore = rig.engine.restore().unwrap();
    let kept = restore
        .item(&format!(r"reg:HKCU\{DWM}\AccentColor"))
        .unwrap();
    assert_eq!(kept.result, RestoreResult::KeptYourChange);
    assert_eq!(
        format!("{}: {}", kept.result.message(), describe(&kept.resource)),
        "Kept your change: accent color (AccentColor)"
    );
    let mut after = snapshot(&rig.fake);
    let accent = after
        .iter()
        .position(|v| v.starts_with(&format!("{DWM}\\AccentColor=")))
        .unwrap();
    after[accent] = before[accent].clone();
    assert_eq!(after, before);
    assert!(rig.fake.broadcasts() >= 3);
}

#[test]
fn every_monitor_gets_the_selected_wallpaper_and_restore_puts_each_back() {
    for monitors in [&[ONE, TWO][..], &[ONE, TWO, THREE][..]] {
        let rig = rig(monitors);
        let before = snapshot(&rig.fake);
        let mut theme = ironwood();
        theme.position = WallpaperPosition::Fit;
        let request = ApplyRequest::new(theme).with_wallpaper(1);
        let plan = rig.engine.plan(&request);
        let wallpaper = plan.iter().find(|p| p.id == wallpaper::ID).unwrap();
        let summaries = wallpaper.summaries();
        assert_eq!(summaries.len(), monitors.len() + 1, "{summaries:?}");
        assert_eq!(summaries[0], "Set the wallpaper position to Fit");
        for (summary, monitor) in summaries[1..].iter().zip(monitors) {
            assert_eq!(
                summary,
                &format!("Show ironwood-2.png on display {monitor}")
            );
        }
        let report = rig.engine.apply(&request).unwrap();
        assert_eq!(outcome(&report, wallpaper::ID), &Outcome::Applied);
        for monitor in monitors {
            assert_eq!(
                rig.fake.monitor_wallpaper(monitor),
                Some(PathBuf::from(format!(
                    r"{LOCAL}\Tintcast\wallpapers\ironwood-2.png"
                )))
            );
        }
        assert_eq!(rig.fake.position(), WallpaperPosition::Fit);
        let restore = rig.engine.restore().unwrap();
        assert!(!restore.failed());
        assert_eq!(snapshot(&rig.fake), before);
    }
}

#[test]
fn a_display_unplugged_before_restore_keeps_its_entry_until_it_is_back() {
    let rig = rig(&[ONE, TWO]);
    let before = snapshot(&rig.fake);
    rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    rig.fake.disconnect_monitor(TWO);
    let first = rig.engine.restore().unwrap();
    assert_eq!(
        first.item(&format!("wallpaper:{ONE}")).unwrap().result,
        RestoreResult::Restored
    );
    let missing = first.item(&format!("wallpaper:{TWO}")).unwrap();
    assert_eq!(
        missing.result,
        RestoreResult::Failed(DISPLAY_NOT_CONNECTED.into())
    );
    assert_eq!(
        missing.result.message(),
        "Display not connected: its wallpaper will be restored when it's back."
    );
    let ledger = rig.engine.ledger_entries().unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].resource, format!("wallpaper:{TWO}"));
    rig.fake.connect_monitor(TWO);
    let second = rig.engine.restore().unwrap();
    assert_eq!(
        second.item(&format!("wallpaper:{TWO}")).unwrap().result,
        RestoreResult::Restored
    );
    assert!(rig.engine.ledger_entries().unwrap().is_empty());
    assert_eq!(snapshot(&rig.fake), before);
}

#[test]
fn slideshow_and_spotlight_backgrounds_are_announced_in_the_preview() {
    for (kind, word) in [
        (BackgroundKind::Slideshow, "slideshow"),
        (BackgroundKind::Spotlight, "Windows Spotlight background"),
    ] {
        let rig = rig(&[ONE]);
        rig.desktop.set_background_kind(kind);
        let plan = rig.engine.plan(&ApplyRequest::new(ironwood()));
        let wallpaper = plan.iter().find(|p| p.id == wallpaper::ID).unwrap();
        let PlannedOutcome::Changes { notes, .. } = &wallpaper.outcome else {
            panic!();
        };
        assert_eq!(
            notes,
            &[format!(
                "Your {word} stays off after Restore; Restore brings back the picture shown now."
            )]
        );
    }
}

#[test]
fn a_solid_color_comes_back_exactly_through_its_background_value() {
    let rig = rig(&[ONE]);
    rig.fake.set_monitor_wallpaper(ONE, None);
    rig.fake.set_registry(
        COLORS,
        BACKGROUND,
        Some(RegistryValue::String("30 60 90".into())),
    );
    rig.desktop.set_background_kind(BackgroundKind::SolidColor);
    let before = snapshot(&rig.fake);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let result = report.result(wallpaper::ID).unwrap();
    assert_eq!(result.outcome, Outcome::Applied);
    assert_eq!(
        result.operations[0],
        r#"Set Background to "30 60 90" in HKCU\Control Panel\Colors"#
    );
    assert_eq!(result.notes, ["Restore brings back your solid color."]);
    rig.engine.restore().unwrap();
    assert_eq!(rig.fake.monitor_wallpaper(ONE), None);
    assert_eq!(snapshot(&rig.fake), before);
}

#[test]
fn a_wallpaper_policy_skips_the_wallpaper() {
    let rig = rig(&[ONE]);
    rig.desktop.set_wallpaper_policy(true);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    assert_eq!(
        outcome(&report, wallpaper::ID),
        &Outcome::Skipped("A policy on this PC blocks wallpaper changes.".into())
    );
    assert_eq!(
        rig.fake.monitor_wallpaper(ONE),
        Some(PathBuf::from(r"C:\Windows\Web\Wallpaper\img0.jpg"))
    );
}

#[test]
fn a_wallpaper_read_error_fails_the_plan() {
    let rig = rig(&[ONE]);
    rig.fake.fail(CallKind::Wallpaper, Some(ONE), "busy");
    let plan = rig.engine.plan(&ApplyRequest::new(ironwood()));
    let wallpaper = plan.iter().find(|p| p.id == wallpaper::ID).unwrap();
    assert_eq!(wallpaper.outcome, PlannedOutcome::Error("busy".into()));
}

const SANDBOX_SETTINGS: &str = "{\n    \"$help\": \"https://aka.ms/terminal-documentation\",\n    \"$schema\": \"https://aka.ms/terminal-profiles-schema\",\n    \"actions\": [],\n    \"copyFormatting\": \"none\",\n    \"copyOnSelect\": false,\n    \"defaultProfile\": \"{61c54bbd-c2c6-5271-96e7-009a87ff44bf}\",\n    \"newTabMenu\": \n    [\n        {\n            \"type\": \"remainingProfiles\"\n        }\n    ],\n    \"profiles\": \n    {\n        \"defaults\": {},\n        \"list\": \n        [\n            {\n                \"commandline\": \"%SystemRoot%\\\\System32\\\\WindowsPowerShell\\\\v1.0\\\\powershell.exe\",\n                \"guid\": \"{61c54bbd-c2c6-5271-96e7-009a87ff44bf}\",\n                \"hidden\": false,\n                \"name\": \"Windows PowerShell\"\n            },\n            {\n                \"commandline\": \"%SystemRoot%\\\\System32\\\\cmd.exe\",\n                \"guid\": \"{0caa0dad-35be-5f56-a8ff-afceeeaa6101}\",\n                \"hidden\": false,\n                \"name\": \"Command Prompt\"\n            },\n            {\n                \"guid\": \"{b453ae62-4e3d-5e58-b989-0a998ec441b8}\",\n                \"hidden\": false,\n                \"name\": \"Azure Cloud Shell\",\n                \"source\": \"Windows.Terminal.Azure\"\n            }\n        ]\n    },\n    \"schemes\": [],\n    \"themes\": []\n}";

fn with_settings(rig: &Rig, text: &str) {
    rig.fake.put_file(&settings_path(), text.as_bytes());
}

fn fragment_json(rig: &Rig) -> serde_json::Value {
    serde_json::from_slice(&rig.fake.file(&fragment()).unwrap()).unwrap()
}

#[test]
fn without_terminal_settings_the_integration_is_skipped() {
    let rig = rig(&[ONE]);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    assert_eq!(
        outcome(&report, terminal::ID),
        &Outcome::Skipped("Windows Terminal is not installed.".into())
    );
    rig.desktop.set_local_app_data(None);
    let plan = rig.engine.plan(&ApplyRequest::new(ironwood()));
    let terminal = plan.iter().find(|p| p.id == terminal::ID).unwrap();
    assert_eq!(
        terminal.outcome,
        PlannedOutcome::Skip("Windows Terminal is not installed.".into())
    );
}

#[test]
fn default_settings_get_their_scheme_from_the_fragment_alone() {
    let rig = rig(&[ONE]);
    with_settings(&rig, SANDBOX_SETTINGS);
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    rig.fake.set_file_modified(&settings_path(), old).ok();
    let theme = ironwood();
    let report = rig.engine.apply(&ApplyRequest::new(theme.clone())).unwrap();
    assert_eq!(outcome(&report, terminal::ID), &Outcome::Applied);
    assert_eq!(
        rig.fake.file(&settings_path()).unwrap(),
        SANDBOX_SETTINGS.as_bytes()
    );
    assert!(rig.fake.file_modified(&settings_path()).unwrap() > old);
    let written = fragment_json(&rig);
    let scheme = &written["schemes"][0];
    assert_eq!(scheme["name"], "Tintcast Ironwood");
    assert_eq!(scheme["background"], "#1a1b1d");
    assert_eq!(scheme["foreground"], "#dcd8d0");
    assert_eq!(scheme["cursorColor"], "#d94f70");
    assert_eq!(scheme["selectionBackground"], theme.palette.selection.hex());
    assert_eq!(scheme["red"], theme.palette.ansi[1].hex());
    assert_eq!(scheme["brightWhite"], theme.palette.ansi[15].hex());
    let updates: Vec<_> = written["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            assert_eq!(p["colorScheme"], "Tintcast Ironwood");
            p["updates"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        updates,
        [
            "{b453ae62-4e3d-5e58-b989-0a998ec441b8}",
            WINDOWS_POWERSHELL,
            COMMAND_PROMPT
        ]
    );
    rig.engine.apply(&ApplyRequest::new(linen())).unwrap();
    assert_eq!(fragment_json(&rig)["schemes"][0]["name"], "Tintcast Linen");
    rig.engine.restore().unwrap();
    assert_eq!(rig.fake.file(&fragment()), None);
    assert_eq!(
        rig.fake.file(&settings_path()).unwrap(),
        SANDBOX_SETTINGS.as_bytes()
    );
}

const COMMENTED: &str = "// My terminal\n{\n    \"profiles\": {\n        \"defaults\": {\n            \"colorScheme\": \"Campbell\", // mine\n            \"font\": { \"face\": \"Cascadia Mono\" }\n        },\n        \"list\": [\n            { \"guid\": \"{61c54bbd-c2c6-5271-96e7-009a87ff44bf}\", \"name\": \"Windows PowerShell\" },\n            { \"guid\": \"{574e775e-4f2a-5b96-ac1e-a2962a402336}\", \"name\": \"PowerShell\", \"source\": \"Windows.Terminal.PowershellCore\", \"colorScheme\": \"One Half Dark\" }\n        ]\n    }\n}\n";

#[test]
fn a_user_default_scheme_is_replaced_in_place_and_restored_byte_for_byte() {
    let rig = rig(&[ONE]);
    with_settings(&rig, COMMENTED);
    let plan = rig.engine.plan(&ApplyRequest::new(ironwood()));
    let terminal = plan.iter().find(|p| p.id == terminal::ID).unwrap();
    let PlannedOutcome::Changes { operations, notes } = &terminal.outcome else {
        panic!("{:?}", terminal.outcome);
    };
    assert_eq!(operations.len(), 2);
    assert_eq!(
        operations[1].resource(),
        format!(
            "json:{}#/profiles/defaults/colorScheme",
            settings_path().display()
        )
    );
    assert_eq!(notes, &["PowerShell keeps its own scheme"]);
    rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let edited = String::from_utf8(rig.fake.file(&settings_path()).unwrap()).unwrap();
    assert_eq!(
        edited,
        COMMENTED.replace("\"Campbell\"", "\"Tintcast Ironwood\"")
    );
    assert!(fragment_json(&rig).get("profiles").is_none());
    rig.engine.apply(&ApplyRequest::new(linen())).unwrap();
    rig.engine.restore().unwrap();
    assert_eq!(
        rig.fake.file(&settings_path()).unwrap(),
        COMMENTED.as_bytes()
    );
    assert_eq!(rig.fake.file(&fragment()), None);
}

#[test]
fn a_hand_written_profile_adds_a_default_scheme_that_restore_removes_exactly() {
    let rig = rig(&[ONE]);
    let text = SANDBOX_SETTINGS.replace(
        "        ]\n    },",
        "            ,{ \"guid\": \"{22222222-2222-2222-2222-222222222222}\", \"name\": \"My SSH box\", \"commandline\": \"ssh box\" }\n        ]\n    },",
    );
    assert!(tintcast_engine::jsonc::parse(&text).unwrap().is_some());
    with_settings(&rig, &text);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    assert_eq!(outcome(&report, terminal::ID), &Outcome::Applied);
    let edited = String::from_utf8(rig.fake.file(&settings_path()).unwrap()).unwrap();
    assert_eq!(
        tintcast_engine::jsonc::get(&edited, "/profiles/defaults/colorScheme").unwrap(),
        Some(serde_json::json!("Tintcast Ironwood"))
    );
    assert!(fragment_json(&rig)["profiles"].is_array());
    rig.engine.restore().unwrap();
    assert_eq!(rig.fake.file(&settings_path()).unwrap(), text.as_bytes());
}

#[test]
fn a_settings_syntax_error_writes_nothing_for_terminal() {
    let rig = rig(&[ONE]);
    let broken = "{\n  \"profiles\": {\n    \"defaults\": { ]\n  }\n}\n";
    with_settings(&rig, broken);
    let report = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    let failed = outcome(&report, terminal::ID);
    assert!(failed.is_failure());
    assert!(
        failed
            .message()
            .starts_with("Windows Terminal's settings.json has a syntax error at line 3, column "),
        "{}",
        failed.message()
    );
    assert_eq!(rig.fake.file(&settings_path()).unwrap(), broken.as_bytes());
    assert_eq!(rig.fake.file(&fragment()), None);
    assert_eq!(outcome(&report, appearance::ID), &Outcome::Applied);
    assert_eq!(outcome(&report, wallpaper::ID), &Outcome::Applied);
}

#[test]
fn applying_the_current_theme_again_changes_nothing() {
    let rig = rig(&[ONE, TWO]);
    with_settings(&rig, SANDBOX_SETTINGS);
    rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    rig.fake.clear_calls();
    let again = rig.engine.apply(&ApplyRequest::new(ironwood())).unwrap();
    for result in &again.results {
        assert_eq!(
            result.outcome,
            Outcome::Skipped("Nothing to change".into()),
            "{}",
            result.id
        );
    }
    assert!(rig.fake.writes().is_empty());
    assert_eq!(rig.engine.history().unwrap().len(), 1);
}
