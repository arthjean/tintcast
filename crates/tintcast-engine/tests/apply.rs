#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use support::*;
use tintcast_engine::fake::{CallKind, FakeServices};
use tintcast_engine::{
    ApplyRequest, Engine, EngineError, Integration, LedgerEntry, Operation, Outcome, Plan,
    PlannedOutcome, RegistryValue, ResourceState, StepState, sha256_hex, stage,
};

fn ledger_on_disk(data: &tintcast_engine::DataDir) -> Vec<LedgerEntry> {
    let Ok(bytes) = std::fs::read(data.ledger()) else {
        return Vec::new();
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    serde_json::from_value(value["entries"].clone()).unwrap()
}

type Snapshot = Vec<(String, StepState)>;

fn owns(resource: &str, write_target: &str) -> bool {
    resource == write_target
        || write_target
            .strip_prefix("file:")
            .is_some_and(|path| resource.starts_with(&format!("json:{path}#")))
}

fn journal_steps_on_disk(data: &tintcast_engine::DataDir) -> Snapshot {
    let names = journal_files(data);
    assert_eq!(names.len(), 1, "one journal file during a run: {names:?}");
    let bytes = std::fs::read(data.journal().join(&names[0])).unwrap();
    let run: tintcast_engine::JournalRun = serde_json::from_slice(&bytes).unwrap();
    run.steps
        .into_iter()
        .map(|s| (s.resource, s.state))
        .collect()
}

#[test]
fn the_plan_lists_each_integration_with_summaries_or_a_skip_reason_and_writes_nothing() {
    let (temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&[r"\\?\DISPLAY#ONE"]));
    seed_user_setup(&fake);
    let mut integrations = standard(&data);
    integrations.push(Box::new(
        Scripted::new("editor", "Code editor", |_, _| Ok(Plan::changes(vec![]))).not_installed(),
    ));
    integrations.push(Box::new(Scripted::new("contrast", "Contrast", |_, _| {
        Ok(Plan::skip(
            "A contrast theme is on, so Tintcast leaves mode and accent alone.",
        ))
    })));
    let engine = Engine::new(data, fake.clone(), integrations).with_lock(unique_lock());
    fake.clear_calls();

    let planned = engine.plan(&ApplyRequest::new(moondust()));

    let ids: Vec<_> = planned.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["system", "wallpaper", "terminal", "editor", "contrast"]
    );
    assert_eq!(
        planned[0].summaries()[0],
        r"Set AppsUseLightTheme to 0 in HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
    );
    assert_eq!(planned[1].summaries().len(), 2);
    assert_eq!(
        planned[3].outcome,
        PlannedOutcome::Skip("Code editor is not installed.".into())
    );
    assert_eq!(
        planned[4].outcome,
        PlannedOutcome::Skip(
            "A contrast theme is on, so Tintcast leaves mode and accent alone.".into()
        )
    );
    assert!(fake.writes().is_empty());
    assert!(!fake.calls().is_empty());
    assert!(!temp.path().join("Tintcast").join("ledger.json").exists());
}

#[test]
fn originals_and_file_bytes_are_recorded_and_each_step_journaled_before_its_write() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&[r"\\?\DISPLAY#ONE"]));
    seed_user_setup(&fake);
    let observations: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = observations.clone();
    let watched = data.clone();
    let services = Arc::new(Hooked::before(fake.clone(), move |resource| {
        let ledger = ledger_on_disk(&watched);
        let entry = ledger
            .iter()
            .find(|e| owns(&e.resource, resource))
            .unwrap_or_else(|| panic!("{resource} is not in ledger.json before its write"));
        if let ResourceState::File { sha256 } = &entry.original {
            assert!(watched.backups().join(sha256).exists());
        }
        if let Some(sha) = &entry.file_backup {
            assert!(watched.backups().join(sha).exists());
        }
        let steps = journal_steps_on_disk(&watched);
        let (last, state) = steps.last().unwrap();
        assert!(owns(last, resource), "{resource} is the newest step");
        assert_eq!(
            *state,
            StepState::Pending,
            "{resource} is journaled as pending before its write"
        );
        seen.lock().unwrap().push(resource.to_owned());
    }));
    let engine = Engine::new(data.clone(), services, standard(&data)).with_lock(unique_lock());

    let report = engine.apply(&ApplyRequest::new(moondust())).unwrap();

    assert!(!report.failed(), "{report:?}");
    assert_eq!(observations.lock().unwrap().len(), 8);
    assert!(
        journal_files(&data).is_empty(),
        "the journal is deleted at the end"
    );
    let ledger = engine.ledger_entries().unwrap();
    let original = |resource: &str| {
        ledger
            .iter()
            .find(|e| e.resource == resource)
            .map(|e| e.original.clone())
            .unwrap()
    };
    assert_eq!(
        original(&format!(r"reg:HKCU\{PERSONALIZE}\AppsUseLightTheme")),
        ResourceState::Registry {
            value: RegistryValue::Dword(1)
        }
    );
    assert_eq!(
        original(&format!(r"reg:HKCU\{DWM}\AccentColor")),
        ResourceState::Absent
    );
    let (fragment, settings) = terminal_paths();
    assert_eq!(
        original(&format!("file:{}", fragment.display())),
        ResourceState::Absent
    );
    let settings_entry = ledger
        .iter()
        .find(|e| e.resource.starts_with("json:"))
        .unwrap();
    let backup = settings_entry.file_backup.clone().unwrap();
    assert_eq!(backup, sha256_hex(SETTINGS.as_bytes()));
    assert_eq!(
        std::fs::read(data.backups().join(backup)).unwrap(),
        SETTINGS.as_bytes()
    );
    let written = String::from_utf8(fake.file(&settings).unwrap()).unwrap();
    assert!(written.contains("// My Terminal settings") && written.contains("/* keep */"));
}

#[test]
fn each_step_is_journaled_as_pending_then_applied() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&["one"]));
    let states: Arc<Mutex<Vec<Snapshot>>> = Arc::default();
    let seen = states.clone();
    let watched = data.clone();
    let services = Arc::new(Hooked::before(fake, move |_| {
        seen.lock().unwrap().push(journal_steps_on_disk(&watched))
    }));
    let integrations: Vec<Box<dyn Integration>> = vec![Box::new(system_integration())];
    let engine = Engine::new(data.clone(), services, integrations).with_lock(unique_lock());

    engine.apply(&ApplyRequest::new(moondust())).unwrap();

    let states = states.lock().unwrap();
    assert_eq!(states.len(), 4);
    for (index, snapshot) in states.iter().enumerate() {
        assert_eq!(snapshot.len(), index + 1);
        for (_, state) in &snapshot[..index] {
            assert_eq!(*state, StepState::Applied);
        }
        assert_eq!(snapshot[index].1, StepState::Pending);
    }
    assert!(journal_files(&data).is_empty());
}

#[test]
fn a_failed_third_operation_rolls_back_its_integration_alone() {
    let f = fixture(vec![
        Box::new(system_integration()),
        Box::new(wallpaper_integration(std::path::PathBuf::from(r"C:\w"))),
    ]);
    seed_user_setup(&f.fake);
    f.fake.fail(
        CallKind::WriteRegistry,
        Some("AccentColorMenu"),
        "Access is denied.",
    );

    let report = f.engine.apply(&ApplyRequest::new(moondust())).unwrap();

    let system = report.result("system").unwrap();
    match &system.outcome {
        Outcome::Failed {
            operation,
            error,
            rollback_errors,
        } => {
            assert_eq!(
                operation.as_deref(),
                Some(
                    r"Set AccentColorMenu to 3153936 in HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Accent"
                )
            );
            assert_eq!(error, "Access is denied.");
            assert!(rollback_errors.is_empty());
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    assert!(system.outcome.message().contains("AccentColorMenu"));
    assert!(
        system
            .outcome
            .message()
            .ends_with("Accent: Access is denied. Tintcast rolled back this part.")
    );
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        f.fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    let registry_writes: Vec<_> = f
        .fake
        .writes()
        .into_iter()
        .filter(|c| c.kind == CallKind::WriteRegistry)
        .map(|c| c.target)
        .collect();
    assert_eq!(
        registry_writes,
        [
            format!(r"{PERSONALIZE}\AppsUseLightTheme"),
            format!(r"{PERSONALIZE}\SystemUsesLightTheme"),
            format!(r"{ACCENT}\AccentColorMenu"),
            format!(r"{PERSONALIZE}\SystemUsesLightTheme"),
            format!(r"{PERSONALIZE}\AppsUseLightTheme"),
        ]
    );
    let ledger = f.engine.ledger_entries().unwrap();
    assert!(ledger.iter().all(|e| e.integration == "wallpaper"));
    assert_eq!(ledger.len(), 2);
    assert_eq!(
        report.result("wallpaper").unwrap().outcome,
        Outcome::Applied
    );
    assert_eq!(
        f.fake.monitor_wallpaper(r"\\?\DISPLAY#ONE"),
        Some(std::path::PathBuf::from(r"C:\w\moondust-0.png"))
    );
    assert!(report.failed());
    let history = f.engine.history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].integrations, ["wallpaper"]);
}

#[test]
fn a_failed_operation_on_a_resource_touched_before_keeps_its_ledger_entry() {
    let f = fixture(vec![Box::new(system_integration())]);
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    f.fake.fail(
        CallKind::WriteRegistry,
        Some("AccentColorMenu"),
        "Access is denied.",
    );

    let report = f.engine.apply(&ApplyRequest::new(daylight())).unwrap();

    assert!(report.failed());
    let ledger = f.engine.ledger_entries().unwrap();
    assert_eq!(ledger.len(), 4);
    let apps = ledger
        .iter()
        .find(|e| e.resource.ends_with("AppsUseLightTheme"))
        .unwrap();
    assert_eq!(
        apps.original,
        ResourceState::Registry {
            value: RegistryValue::Dword(1)
        }
    );
    assert_eq!(
        apps.applied,
        ResourceState::Registry {
            value: RegistryValue::Dword(0)
        }
    );
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(0))
    );
}

#[test]
fn each_apply_appends_a_history_entry_and_history_keeps_fifty() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine
        .apply(&ApplyRequest::new(daylight()).with_wallpaper(2))
        .unwrap();

    let history = f.engine.history().unwrap();
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    assert_eq!(entry.theme_id, "daylight");
    assert_eq!(entry.wallpaper, 2);
    assert!(entry.at <= jiff::Timestamp::now());
    assert_eq!(entry.integrations, ["system", "wallpaper", "terminal"]);
    assert!(entry.applied.iter().any(|v| v.resource
        == format!(r"reg:HKCU\{PERSONALIZE}\AppsUseLightTheme")
        && v.value
            == ResourceState::Registry {
                value: RegistryValue::Dword(1)
            }));
    assert!(entry.applied.iter().any(|v| matches!(
        &v.value,
        ResourceState::Wallpaper { image } if image.ends_with("daylight-2.png")
    )));

    for index in 0..55 {
        let theme = if index % 2 == 0 {
            moondust()
        } else {
            daylight()
        };
        f.engine.apply(&ApplyRequest::new(theme)).unwrap();
    }
    let history = f.engine.history().unwrap();
    assert_eq!(history.len(), 50);
    assert_eq!(history.last().unwrap().theme_id, "moondust");
}

#[test]
fn ledger_and_history_files_carry_version_one() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    for path in [f.data.ledger(), f.data.history()] {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["version"], 1);
    }
}

#[test]
fn a_ledger_write_stopped_before_its_rename_leaves_the_previous_ledger_intact() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    let before = std::fs::read(f.data.ledger()).unwrap();
    let history_before = std::fs::read(f.data.history()).unwrap();

    std::mem::forget(stage(&f.data.ledger(), b"{ \"version\": 1, \"entr").unwrap());
    std::mem::forget(stage(&f.data.history(), b"[").unwrap());

    assert_eq!(std::fs::read(f.data.ledger()).unwrap(), before);
    assert_eq!(std::fs::read(f.data.history()).unwrap(), history_before);
    assert_eq!(f.engine.ledger_entries().unwrap().len(), 8);
    assert_eq!(f.engine.history().unwrap().len(), 1);
    let report = f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    assert!(!report.failed());
    assert!(
        f.engine.startup().unwrap().interrupted.is_empty(),
        "leftover temporary files are not journals"
    );
}

#[test]
fn a_second_apply_waits_for_the_lock_then_fails_with_another_apply_is_running() {
    let f = standard_fixture();
    let (held, wait_for_hold) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel::<()>();
    let lock = f.lock.clone();
    let holder = thread::spawn(move || {
        let guard = lock.acquire().unwrap();
        held.send(()).unwrap();
        wait_for_release.recv().unwrap();
        drop(guard);
    });
    wait_for_hold.recv().unwrap();

    let error = f.engine.apply(&ApplyRequest::new(moondust())).unwrap_err();
    assert!(matches!(error, EngineError::Busy));
    assert_eq!(error.to_string(), "Another apply is running.");
    assert!(matches!(f.engine.restore().unwrap_err(), EngineError::Busy));
    assert!(matches!(
        f.engine.undo(&library).unwrap_err(),
        EngineError::Busy
    ));
    assert!(f.fake.writes().is_empty());

    release.send(()).unwrap();
    holder.join().unwrap();
    assert!(
        !f.engine
            .apply(&ApplyRequest::new(moondust()))
            .unwrap()
            .failed()
    );
}

#[test]
fn an_unreadable_ledger_blocks_applies_and_restore_with_its_path() {
    let f = standard_fixture();
    std::fs::write(f.data.ledger(), b"not json").unwrap();
    let message = format!(
        "Tintcast can't read its restore records in {}. Restore is unavailable until the file is fixed.",
        f.data.ledger().display()
    );
    assert_eq!(
        f.engine
            .apply(&ApplyRequest::new(moondust()))
            .unwrap_err()
            .to_string(),
        message
    );
    assert_eq!(f.engine.restore().unwrap_err().to_string(), message);
    assert_eq!(f.engine.startup().unwrap().ledger_error, Some(message));
    assert!(f.fake.writes().is_empty());
    assert_eq!(std::fs::read(f.data.ledger()).unwrap(), b"not json");
}

#[test]
fn a_failing_call_reaches_the_caller_as_an_error_value() {
    let f = standard_fixture();
    seed_user_setup(&f.fake);
    f.fake.fail(
        CallKind::Monitors,
        None,
        "The display service is not running.",
    );
    let planned = f.engine.plan(&ApplyRequest::new(moondust()));
    assert_eq!(
        planned[1].outcome,
        PlannedOutcome::Error("The display service is not running.".into())
    );
    let report = f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    assert!(report.result("wallpaper").unwrap().outcome.is_failure());
    assert_eq!(report.result("system").unwrap().outcome, Outcome::Applied);

    f.fake.clear_failures();
    f.fake.fail(
        CallKind::ReadRegistry,
        Some("AccentColor"),
        "The registry is locked.",
    );
    let report = f.engine.apply(&ApplyRequest::new(daylight())).unwrap();
    assert!(report.result("system").unwrap().outcome.is_failure());

    f.fake
        .fail(CallKind::ReadFile, None, "The disk is not ready.");
    let restore = f.engine.restore().unwrap();
    assert!(restore.failed());
    assert!(
        restore
            .items
            .iter()
            .any(|i| i.result.message().contains("The disk is not ready."))
    );
}

#[test]
fn operation_is_a_closed_set_of_five_cases() {
    fn case(op: &Operation) -> &'static str {
        match op {
            Operation::SetRegistryValue { .. } => "registry",
            Operation::SetWallpaper { .. } => "wallpaper",
            Operation::SetWallpaperPosition { .. } => "position",
            Operation::WriteManagedFile { .. } => "file",
            Operation::SetJsonValue { .. } => "json",
        }
    }
    let op = Operation::SetJsonValue {
        path: Path::new("s.json").into(),
        pointer: "/a".into(),
        value: None,
    };
    assert_eq!(case(&op), "json");
}

#[test]
fn notice_carries_scene_mit_notice() {
    let notice =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../NOTICE"))
            .unwrap()
            .replace("\r\n", "\n");
    assert!(notice.contains("InsaneArts/Scene"));
    assert!(notice.contains("MIT License"));
    assert!(notice.contains("Copyright (c) 2026 InsaneArts"));
    assert!(notice.contains(
        "The above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software."
    ));
}
