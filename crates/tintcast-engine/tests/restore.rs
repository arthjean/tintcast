#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use support::*;
use tintcast_engine::fake::{CallKind, FakeServices};
use tintcast_engine::{
    ApplyRequest, Engine, EngineError, Integration, Operation, Plan, RegistryValue, RestoreResult,
    UndoReport, WallpaperPosition,
};

const MONITOR: &str = r"\\?\DISPLAY#ONE";

fn reg(key: &str, name: &str) -> String {
    format!(r"reg:HKCU\{key}\{name}")
}

#[test]
fn a_resource_that_still_holds_tintcast_value_gets_its_original_back() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (fragment, settings) = terminal_paths();
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    assert!(f.fake.file(&fragment).is_some());
    assert_eq!(
        f.fake.registry(DWM, "AccentColor"),
        Some(RegistryValue::Dword(shade(&moondust())))
    );

    let report = f.engine.restore().unwrap();

    assert!(
        report
            .items
            .iter()
            .all(|i| i.result == RestoreResult::Restored),
        "{report:?}"
    );
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        f.fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(0x00_d7_78_00))
    );
    assert_eq!(
        f.fake.registry(DWM, "AccentColor"),
        None,
        "a value that did not exist is deleted"
    );
    assert_eq!(
        f.fake.file(&fragment),
        None,
        "a file Tintcast created is deleted"
    );
    assert_eq!(
        f.fake.monitor_wallpaper(MONITOR),
        Some(PathBuf::from(r"C:\Users\u\Pictures\beach.jpg"))
    );
    assert_eq!(f.fake.position(), WallpaperPosition::Fit);
    assert_eq!(f.fake.file(&settings).unwrap(), SETTINGS.as_bytes());
    assert!(f.engine.ledger_entries().unwrap().is_empty());
}

#[test]
fn a_resource_the_user_changed_since_is_kept() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.fake.set_registry(
        ACCENT,
        "AccentColorMenu",
        Some(RegistryValue::Dword(0x00_11_22_33)),
    );

    let report = f.engine.restore().unwrap();

    let item = report.item(&reg(ACCENT, "AccentColorMenu")).unwrap();
    assert_eq!(item.result, RestoreResult::KeptYourChange);
    assert_eq!(item.result.message(), "Kept your change");
    assert_eq!(
        f.fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(0x00_11_22_33))
    );
    assert_eq!(
        report
            .item(&reg(PERSONALIZE, "AppsUseLightTheme"))
            .unwrap()
            .result,
        RestoreResult::Restored
    );
    assert!(f.engine.ledger_entries().unwrap().is_empty());
}

#[test]
fn a_resource_already_at_its_original_is_reported_unchanged_and_not_written() {
    let f = fixture(vec![Box::new(system_integration())]);
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.fake.set_registry(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    f.fake.set_registry(
        PERSONALIZE,
        "SystemUsesLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    f.fake.set_registry(
        ACCENT,
        "AccentColorMenu",
        Some(RegistryValue::Dword(0x00_d7_78_00)),
    );
    f.fake.set_registry(DWM, "AccentColor", None);
    f.fake.clear_calls();

    let report = f.engine.restore().unwrap();

    assert!(
        report
            .items
            .iter()
            .all(|i| i.result == RestoreResult::Unchanged),
        "{report:?}"
    );
    assert_eq!(report.items[0].result.message(), "Unchanged");
    assert!(f.fake.writes().is_empty(), "{:?}", f.fake.writes());
}

#[test]
fn a_value_tintcast_wrote_that_equals_the_original_is_unchanged_without_a_write() {
    let f = fixture(vec![Box::new(Scripted::new("system", "Mode", |_, _| {
        Ok(Plan::changes(vec![dword(
            PERSONALIZE,
            "AppsUseLightTheme",
            1,
        )]))
    }))]);
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    f.fake.clear_calls();
    let report = f.engine.restore().unwrap();
    assert_eq!(report.items[0].result, RestoreResult::Unchanged);
    assert!(f.fake.writes().is_empty());
}

#[test]
fn restore_walks_newest_first_reloads_each_integration_once_and_keeps_failed_entries() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&[MONITOR, r"\\?\DISPLAY#TWO"]));
    seed_user_setup(&fake);
    fake.set_monitor_wallpaper(
        r"\\?\DISPLAY#TWO",
        Some(Path::new(r"C:\Users\u\Pictures\hills.jpg")),
    );
    let system = system_integration();
    let wallpaper = wallpaper_integration(data.wallpapers());
    let system_reloads = system.reloads.clone();
    let wallpaper_reloads = wallpaper.reloads.clone();
    let integrations: Vec<Box<dyn Integration>> = vec![Box::new(system), Box::new(wallpaper)];
    let engine = Engine::new(data, fake.clone(), integrations).with_lock(unique_lock());
    engine.apply(&ApplyRequest::new(moondust())).unwrap();
    engine.apply(&ApplyRequest::new(daylight())).unwrap();
    let sequences: Vec<_> = engine
        .ledger_entries()
        .unwrap()
        .iter()
        .map(|e| e.resource.clone())
        .collect();
    system_reloads.store(0, Ordering::SeqCst);
    wallpaper_reloads.store(0, Ordering::SeqCst);
    fake.disconnect_monitor(r"\\?\DISPLAY#TWO");
    fake.clear_calls();

    let report = engine.restore().unwrap();

    let walked: Vec<_> = report.items.iter().map(|i| i.resource.clone()).collect();
    let mut newest_first = sequences.clone();
    newest_first.reverse();
    assert_eq!(walked, newest_first);
    assert_eq!(system_reloads.load(Ordering::SeqCst), 1);
    assert_eq!(wallpaper_reloads.load(Ordering::SeqCst), 1);
    let failed = report.item(r"wallpaper:\\?\DISPLAY#TWO").unwrap();
    assert_eq!(
        failed.result,
        RestoreResult::Failed("Display not connected".into())
    );
    let ledger = engine.ledger_entries().unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].resource, r"wallpaper:\\?\DISPLAY#TWO");
    assert_eq!(ledger[0].error.as_deref(), Some("Display not connected"));
    assert_eq!(
        fake.monitor_wallpaper(MONITOR),
        Some(PathBuf::from(r"C:\Users\u\Pictures\beach.jpg"))
    );

    fake.connect_monitor(r"\\?\DISPLAY#TWO");
    let again = engine.restore().unwrap();
    assert_eq!(again.items.len(), 1);
    assert_eq!(again.items[0].result, RestoreResult::Restored);
    assert_eq!(
        fake.monitor_wallpaper(r"\\?\DISPLAY#TWO"),
        Some(PathBuf::from(r"C:\Users\u\Pictures\hills.jpg"))
    );
    assert!(engine.ledger_entries().unwrap().is_empty());
}

#[test]
fn undo_with_two_or_more_entries_applies_the_previous_one_again() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine
        .apply(&ApplyRequest::new(moondust()).with_wallpaper(1))
        .unwrap();
    f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    f.engine.apply(&ApplyRequest::new(ember())).unwrap();

    let report = f.engine.undo(&library).unwrap();

    match report {
        UndoReport::Reapplied(report) => assert_eq!(report.theme_id, "daylight"),
        other => panic!("unexpected undo {other:?}"),
    }
    assert_eq!(
        f.fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(shade(&daylight())))
    );
    let history = f.engine.history().unwrap();
    let ids: Vec<_> = history.iter().map(|h| h.theme_id.as_str()).collect();
    assert_eq!(ids, ["moondust", "daylight"]);

    f.engine.undo(&library).unwrap();
    assert_eq!(
        f.fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(shade(&moondust())))
    );
    assert_eq!(
        f.fake.monitor_wallpaper(MONITOR).unwrap(),
        f.data.wallpapers().join("moondust-1.png"),
        "Undo shows the wallpaper that entry showed"
    );
    assert_eq!(f.engine.history().unwrap().len(), 1);
}

#[test]
fn undo_with_one_entry_restores_the_original_setup() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();

    let report = f.engine.undo(&library).unwrap();

    assert!(matches!(report, UndoReport::Restored(_)));
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert!(f.engine.ledger_entries().unwrap().is_empty());
    assert!(f.engine.history().unwrap().is_empty());
}

#[test]
fn undo_with_no_history_reports_nothing_to_undo_and_writes_nothing() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.fake.clear_calls();
    let report = f.engine.undo(&library).unwrap();
    assert_eq!(report, UndoReport::NothingToUndo);
    assert_eq!(UndoReport::NOTHING_TO_UNDO, "Nothing to undo");
    assert!(f.fake.writes().is_empty());
}

#[test]
fn undo_refuses_a_theme_that_left_the_library_and_keeps_history() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    f.fake.clear_calls();
    let error = f.engine.undo(&|_| None).unwrap_err();
    assert!(matches!(error, EngineError::ThemeMissing(_)));
    assert!(
        error
            .to_string()
            .starts_with("Moondust is no longer in your library")
    );
    assert!(f.fake.writes().is_empty());
    assert_eq!(f.engine.history().unwrap().len(), 2);
}

#[test]
fn a_json_key_the_user_deleted_is_not_created_again() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    let edited = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    let without =
        tintcast_engine::jsonc::set(&edited, "/profiles/defaults/colorScheme", None).unwrap();
    f.fake.put_file(&settings, without.as_bytes());

    let report = f.engine.restore().unwrap();

    let item = report
        .items
        .iter()
        .find(|i| i.resource.starts_with("json:"))
        .unwrap();
    assert_eq!(item.result, RestoreResult::Unchanged);
    let after = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    assert_eq!(
        tintcast_engine::jsonc::get(&after, "/profiles/defaults/colorScheme").unwrap(),
        None
    );
}

#[test]
fn a_json_key_the_user_deleted_over_an_original_value_is_kept_deleted() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    let with_scheme = tintcast_engine::jsonc::set(
        SETTINGS,
        "/profiles/defaults/colorScheme",
        Some(&serde_json::json!("Campbell")),
    )
    .unwrap();
    f.fake.put_file(&settings, with_scheme.as_bytes());
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    let edited = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    let without =
        tintcast_engine::jsonc::set(&edited, "/profiles/defaults/colorScheme", None).unwrap();
    f.fake.put_file(&settings, without.as_bytes());

    let report = f.engine.restore().unwrap();

    let item = report
        .items
        .iter()
        .find(|i| i.resource.starts_with("json:"))
        .unwrap();
    assert_eq!(item.result, RestoreResult::KeptYourChange);
    let after = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    assert_eq!(
        tintcast_engine::jsonc::get(&after, "/profiles/defaults/colorScheme").unwrap(),
        None
    );
}

#[test]
fn restore_of_a_registry_write_windows_denies_keeps_the_entry_with_its_error() {
    let f = fixture(vec![Box::new(system_integration())]);
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.fake.fail(
        CallKind::WriteRegistry,
        Some("AppsUseLightTheme"),
        "Access is denied.",
    );

    let report = f.engine.restore().unwrap();

    assert!(report.failed());
    let item = report.item(&reg(PERSONALIZE, "AppsUseLightTheme")).unwrap();
    assert_eq!(
        item.result.message(),
        "Could not restore: Access is denied."
    );
    let ledger = f.engine.ledger_entries().unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].error.as_deref(), Some("Access is denied."));
}

#[test]
fn a_managed_file_that_existed_gets_its_original_bytes_from_backups() {
    let path = PathBuf::from(r"C:\Users\u\.config\tool\colors.json");
    let target = path.clone();
    let f = fixture(vec![Box::new(Scripted::new(
        "tool",
        "Tool",
        move |request, _| {
            Ok(Plan::changes(vec![Operation::WriteManagedFile {
                path: target.clone(),
                contents: request.theme.name.clone().into_bytes(),
            }]))
        },
    ))]);
    f.fake.put_file(&path, b"user bytes\r\n");
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    assert_eq!(f.fake.file(&path).unwrap(), b"Daylight");
    let report = f.engine.restore().unwrap();
    assert_eq!(report.items[0].result, RestoreResult::Restored);
    assert_eq!(f.fake.file(&path).unwrap(), b"user bytes\r\n");
}

#[test]
fn restore_removes_parent_objects_tintcast_created_and_returns_the_original_bytes() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    let original =
        "{\n    // no defaults yet\n    \"profiles\": {\n        \"list\": []\n    }\n}\n";
    f.fake.put_file(&settings, original.as_bytes());
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    assert!(
        String::from_utf8(f.fake.file(&settings).unwrap())
            .unwrap()
            .contains("\"defaults\"")
    );

    let report = f.engine.restore().unwrap();

    assert!(!report.failed(), "{report:?}");
    assert_eq!(
        String::from_utf8(f.fake.file(&settings).unwrap()).unwrap(),
        original
    );
}

#[test]
fn restore_keeps_a_parent_object_that_was_there_before() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    let original = "{\n    \"profiles\": {\n        \"defaults\": {}\n    }\n}\n";
    f.fake.put_file(&settings, original.as_bytes());
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();

    f.engine.restore().unwrap();

    assert_eq!(
        String::from_utf8(f.fake.file(&settings).unwrap()).unwrap(),
        original
    );
}
