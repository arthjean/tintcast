#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;

use support::*;
use tintcast_engine::fake::CallKind;
use tintcast_engine::{
    ApplyRequest, JournalRun, JournalStep, Operation, Outcome, PlannedOutcome, RegistryValue,
    ResourceState, RestoreResult, StepState, UndoReport, VERSION,
};

#[test]
fn apply_writes_only_owned_resources_and_reports_per_integration() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let request = ApplyRequest::new(moondust());
    let owned: BTreeSet<String> = f
        .engine
        .plan(&request)
        .iter()
        .flat_map(|p| match &p.outcome {
            PlannedOutcome::Changes { operations, .. } => operations
                .iter()
                .map(|op| match op {
                    Operation::SetJsonValue { path, .. } => format!("file:{}", path.display()),
                    Operation::SetRegistryValue { key, name, .. } => format!(r"{key}\{name}"),
                    Operation::SetWallpaper { monitor, .. } => monitor.clone(),
                    Operation::SetWallpaperPosition { .. } => "position".to_owned(),
                    Operation::WriteManagedFile { path, .. } => format!("file:{}", path.display()),
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();
    f.fake.clear_calls();

    let report = f.engine.apply(&request).unwrap();

    for write in f.fake.writes() {
        let target = match write.kind {
            CallKind::WriteFile | CallKind::DeleteFile => format!("file:{}", write.target),
            _ => write.target.clone(),
        };
        assert!(
            owned.contains(&target),
            "{write:?} is not a planned resource"
        );
    }
    for id in ["system", "wallpaper", "terminal"] {
        assert_eq!(report.result(id).unwrap().outcome, Outcome::Applied, "{id}");
    }
    let (_, settings) = terminal_paths();
    let text = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    assert!(text.contains("// My Terminal settings") && text.contains("/* keep */"));
    assert!(text.contains("\"colorScheme\": \"Tintcast Moondust\""));
    assert!(f.engine.startup().unwrap().interrupted.is_empty());
}

#[test]
fn restore_returns_files_to_their_original_bytes() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (fragment, settings) = terminal_paths();
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.engine
        .apply(&ApplyRequest::new(daylight()).with_wallpaper(1))
        .unwrap();

    let report = f.engine.restore().unwrap();

    assert!(!report.items.is_empty());
    assert_eq!(f.fake.file(&settings).unwrap(), SETTINGS.as_bytes());
    assert_eq!(f.fake.file(&fragment), None);
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        f.fake.monitor_wallpaper(r"\\?\DISPLAY#ONE"),
        Some(PathBuf::from(r"C:\Users\u\Pictures\beach.jpg"))
    );
    assert!(f.engine.ledger_entries().unwrap().is_empty());
}

#[test]
fn restore_keeps_changes_made_after_tintcast() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    let text = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    let mine = tintcast_engine::jsonc::set(
        &text,
        "/profiles/defaults/colorScheme",
        Some(&serde_json::json!("Solarized Light")),
    )
    .unwrap();
    f.fake.put_file(&settings, mine.as_bytes());
    f.fake.set_registry(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(RegistryValue::Dword(1)),
    );

    let report = f.engine.restore().unwrap();

    assert!(
        report
            .items
            .iter()
            .any(|i| i.resource.starts_with("json:") && i.result == RestoreResult::KeptYourChange)
    );
    assert!(
        report
            .items
            .iter()
            .any(|i| i.resource.ends_with("AppsUseLightTheme")
                && i.result == RestoreResult::Unchanged)
    );
    let after = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    assert!(after.contains("Solarized Light"));
    assert!(after.contains("// My Terminal settings"));
}

#[test]
fn a_failed_integration_is_rolled_back_and_the_others_succeed() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.fake.fail(
        CallKind::SetWallpaperPosition,
        None,
        "The wallpaper service stopped.",
    );

    let report = f.engine.apply(&ApplyRequest::new(moondust())).unwrap();

    assert!(report.result("wallpaper").unwrap().outcome.is_failure());
    assert!(report.result("terminal").unwrap().outcome.changed());
    assert!(report.result("system").unwrap().outcome.changed());
    assert_eq!(
        f.fake.monitor_wallpaper(r"\\?\DISPLAY#ONE"),
        Some(PathBuf::from(r"C:\Users\u\Pictures\beach.jpg")),
        "the wallpaper set before the failure was put back"
    );
    assert!(
        f.engine
            .ledger_entries()
            .unwrap()
            .iter()
            .all(|e| e.integration != "wallpaper")
    );
}

#[test]
fn undo_returns_to_the_previous_theme() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (fragment, settings) = terminal_paths();
    f.engine.apply(&ApplyRequest::new(ember())).unwrap();
    let ember_fragment = f.fake.file(&fragment);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    assert_ne!(f.fake.file(&fragment), ember_fragment);

    let report = f.engine.undo(&library).unwrap();

    match report {
        UndoReport::Reapplied(report) => assert_eq!(report.theme_name, "Ember"),
        other => panic!("unexpected undo {other:?}"),
    }
    assert_eq!(f.fake.file(&fragment), ember_fragment);
    f.engine.undo(&|_| None).unwrap();
    assert_eq!(f.fake.file(&settings).unwrap(), SETTINGS.as_bytes());
    assert_eq!(f.fake.file(&fragment), None);
}

#[test]
fn an_interrupted_run_is_detected() {
    let f = standard_fixture();
    let run = JournalRun {
        version: VERSION,
        run_id: "crashed".into(),
        theme_id: "moondust".into(),
        theme_name: "Moondust".into(),
        started: jiff::Timestamp::now(),
        steps: vec![JournalStep {
            integration: "terminal".into(),
            resource: r"file:C:\x".into(),
            summary: r"Write C:\x".into(),
            operation: Operation::WriteManagedFile {
                path: PathBuf::from(r"C:\x"),
                contents: Vec::new(),
            },
            before: ResourceState::Absent,
            file_backup: None,
            state: StepState::Pending,
        }],
    };
    std::fs::write(
        f.data.journal().join("crashed.json"),
        serde_json::to_vec(&run).unwrap(),
    )
    .unwrap();

    let startup = f.engine.startup().unwrap();

    let ids: Vec<_> = startup
        .interrupted
        .iter()
        .map(|r| r.run_id.as_str())
        .collect();
    assert_eq!(ids, ["crashed"]);
    assert_eq!(startup.interrupted[0].pending.len(), 1);
}

#[test]
fn a_settings_file_tintcast_created_is_removed_on_restore() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    let (_, settings) = terminal_paths();
    f.fake.remove_file(&settings);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    let created = String::from_utf8(f.fake.file(&settings).unwrap()).unwrap();
    assert!(created.contains("Tintcast Moondust"));

    let report = f.engine.restore().unwrap();

    assert!(!report.failed(), "{report:?}");
    assert_eq!(f.fake.file(&settings), None);
}
