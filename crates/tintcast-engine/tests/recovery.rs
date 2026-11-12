#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use support::*;
use tintcast_engine::fake::FakeServices;
use tintcast_engine::{
    ApplyRequest, Engine, EngineError, Integration, Plan, RegistryValue, RestoreResult, StepState,
    Theme,
};

fn step(
    id: &str,
    key: &'static str,
    name: &'static str,
    value: fn(&Theme) -> u32,
) -> Box<dyn Integration> {
    Box::new(Scripted::new(id, name, move |request, _| {
        Ok(Plan::changes(vec![dword(key, name, value(&request.theme))]))
    }))
}

fn five_steps() -> Vec<Box<dyn Integration>> {
    vec![
        step("apps", PERSONALIZE, "AppsUseLightTheme", light),
        step("system", PERSONALIZE, "SystemUsesLightTheme", light),
        step("menu", ACCENT, "AccentColorMenu", shade),
        step("start", ACCENT, "StartColorMenu", shade),
        step("dwm", DWM, "AccentColor", shade),
    ]
}

fn crash_after(
    writes: usize,
    fake: Arc<FakeServices>,
    data: &tintcast_engine::DataDir,
    lock: &tintcast_engine::EngineLock,
) {
    let count = Arc::new(AtomicUsize::new(0));
    let services = Arc::new(Hooked::before(fake, move |resource| {
        if count.fetch_add(1, Ordering::SeqCst) == writes {
            panic!("power lost before writing {resource}");
        }
    }));
    let engine = Engine::new(data.clone(), services, five_steps()).with_lock(lock.clone());
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        engine.apply(&ApplyRequest::new(moondust()))
    }));
    assert!(outcome.is_err(), "the apply stopped in the middle");
}

fn reopened(
    fake: Arc<FakeServices>,
    data: &tintcast_engine::DataDir,
    lock: &tintcast_engine::EngineLock,
) -> Engine {
    Engine::new(data.clone(), fake, five_steps()).with_lock(lock.clone())
}

#[test]
fn a_crash_after_step_two_of_five_is_reported_and_rolled_back() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    crash_after(2, fake.clone(), &data, &lock);
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(0))
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        Some(RegistryValue::Dword(0))
    );
    assert_eq!(
        journal_files(&data).len(),
        1,
        "the journal of the unfinished run stays"
    );

    let engine = reopened(fake.clone(), &data, &lock);
    let startup = engine.startup().unwrap();

    assert_eq!(startup.interrupted.len(), 1);
    let run = &startup.interrupted[0];
    assert_eq!(run.theme_name, "Moondust");
    assert_eq!(
        run.message(),
        "Tintcast was interrupted while applying Moondust."
    );
    let applied: Vec<_> = run.applied.iter().map(|s| s.resource.as_str()).collect();
    assert_eq!(
        applied,
        [
            format!(r"reg:HKCU\{PERSONALIZE}\AppsUseLightTheme"),
            format!(r"reg:HKCU\{PERSONALIZE}\SystemUsesLightTheme"),
        ]
    );
    let pending: Vec<_> = run.pending.iter().map(|s| s.resource.as_str()).collect();
    assert_eq!(pending, [format!(r"reg:HKCU\{ACCENT}\AccentColorMenu")]);

    fake.clear_calls();
    let report = engine.roll_back(&run.run_id).unwrap();

    let walked: Vec<_> = report
        .items
        .iter()
        .map(|i| (i.resource.as_str(), i.result.clone()))
        .collect();
    assert_eq!(
        walked,
        [
            (
                format!(r"reg:HKCU\{ACCENT}\AccentColorMenu").as_str(),
                RestoreResult::Unchanged
            ),
            (
                format!(r"reg:HKCU\{PERSONALIZE}\SystemUsesLightTheme").as_str(),
                RestoreResult::Restored
            ),
            (
                format!(r"reg:HKCU\{PERSONALIZE}\AppsUseLightTheme").as_str(),
                RestoreResult::Restored
            ),
        ]
    );
    let order: Vec<_> = fake.writes().into_iter().map(|c| c.target).collect();
    assert_eq!(
        order,
        [
            format!(r"{PERSONALIZE}\SystemUsesLightTheme"),
            format!(r"{PERSONALIZE}\AppsUseLightTheme"),
        ],
        "applied steps go back in reverse order"
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(0x00_d7_78_00))
    );
    assert!(
        journal_files(&data).is_empty(),
        "the journal file is removed"
    );
    assert!(engine.ledger_entries().unwrap().is_empty());
    assert!(engine.startup().unwrap().interrupted.is_empty());
}

#[test]
fn rolling_back_a_crash_keeps_what_an_earlier_apply_set() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    let engine = reopened(fake.clone(), &data, &lock);
    engine.apply(&ApplyRequest::new(daylight())).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let services = Arc::new(Hooked::before(fake.clone(), move |resource| {
        if count.fetch_add(1, Ordering::SeqCst) == 3 {
            panic!("power lost before writing {resource}");
        }
    }));
    let crashing = Engine::new(data.clone(), services, five_steps()).with_lock(lock.clone());
    assert!(
        catch_unwind(AssertUnwindSafe(
            || crashing.apply(&ApplyRequest::new(moondust()))
        ))
        .is_err()
    );

    let run = engine.startup().unwrap().interrupted.remove(0);
    engine.roll_back(&run.run_id).unwrap();

    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(shade(&daylight())))
    );
    let kept: Vec<_> = engine
        .ledger_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.resource)
        .collect();
    assert_eq!(
        kept,
        [
            format!(r"reg:HKCU\{ACCENT}\AccentColorMenu"),
            format!(r"reg:HKCU\{ACCENT}\StartColorMenu"),
            format!(r"reg:HKCU\{DWM}\AccentColor"),
        ],
        "entries whose value is back at the original leave the ledger, the others keep their originals"
    );
    engine.restore().unwrap();
    assert_eq!(
        fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(0x00_d7_78_00))
    );
    assert_eq!(fake.registry(DWM, "AccentColor"), None);
}

#[test]
fn a_journal_that_does_not_parse_is_set_aside_and_later_applies_still_run() {
    let f = fixture(five_steps());
    seed_user_setup(&f.fake);
    std::fs::write(
        f.data.journal().join("1234-9-0.json"),
        b"{ \"version\": 1, \"steps\": [",
    )
    .unwrap();

    let startup = f.engine.startup().unwrap();

    assert!(startup.interrupted.is_empty());
    assert_eq!(
        startup.corrupt_journals,
        [f.data.journal().join("1234-9-0.corrupt.json")]
    );
    assert_eq!(journal_files(&f.data), ["1234-9-0.corrupt.json"]);
    let report = f.engine.apply(&ApplyRequest::new(moondust())).unwrap();
    assert!(!report.failed());
    let again = f.engine.startup().unwrap();
    assert!(again.interrupted.is_empty());
    assert!(again.corrupt_journals.is_empty());
}

#[test]
fn keep_as_is_drops_the_journal_and_leaves_the_changes() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    crash_after(2, fake.clone(), &data, &lock);
    let engine = reopened(fake.clone(), &data, &lock);
    let run = engine.startup().unwrap().interrupted.remove(0);

    engine.keep_interrupted(&run.run_id).unwrap();

    assert!(journal_files(&data).is_empty());
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(0))
    );
    assert!(matches!(
        engine.roll_back(&run.run_id).unwrap_err(),
        EngineError::UnknownRun(_)
    ));
    engine.restore().unwrap();
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
}

#[test]
fn startup_waits_for_a_live_apply_instead_of_reporting_it() {
    let f = fixture(five_steps());
    let guard = f.lock.acquire().unwrap();
    std::thread::scope(|scope| {
        let result = scope.spawn(|| f.engine.startup()).join().unwrap();
        assert!(matches!(result, Err(EngineError::Busy)));
    });
    drop(guard);
    assert!(f.engine.startup().is_ok());
}

fn crash_right_after_write(
    writes: usize,
    fake: Arc<FakeServices>,
    data: &tintcast_engine::DataDir,
    lock: &tintcast_engine::EngineLock,
) {
    let count = Arc::new(AtomicUsize::new(0));
    let services = Arc::new(Hooked::after(fake, move |resource| {
        if count.fetch_add(1, Ordering::SeqCst) == writes {
            panic!("power lost right after writing {resource}");
        }
    }));
    let crashing = Engine::new(data.clone(), services, five_steps()).with_lock(lock.clone());
    assert!(
        catch_unwind(AssertUnwindSafe(
            || crashing.apply(&ApplyRequest::new(moondust()))
        ))
        .is_err()
    );
}

#[test]
fn restore_after_a_crash_puts_back_a_write_the_ledger_had_not_confirmed() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    crash_right_after_write(1, fake.clone(), &data, &lock);
    assert_eq!(
        fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        Some(RegistryValue::Dword(0)),
        "the second write landed before the crash"
    );
    let engine = reopened(fake.clone(), &data, &lock);

    let report = engine.restore().unwrap();

    assert!(
        report
            .items
            .iter()
            .all(|i| i.result == RestoreResult::Restored),
        "{report:?}"
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "SystemUsesLightTheme"),
        Some(RegistryValue::Dword(1))
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
}

#[test]
fn keep_as_is_after_a_crash_right_after_a_write_still_lets_restore_put_it_back() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    let engine = reopened(fake.clone(), &data, &lock);
    engine.apply(&ApplyRequest::new(daylight())).unwrap();
    crash_right_after_write(2, fake.clone(), &data, &lock);
    assert_eq!(
        fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(shade(&moondust())))
    );
    let run = engine.startup().unwrap().interrupted.remove(0);

    engine.keep_interrupted(&run.run_id).unwrap();

    let entry = engine
        .ledger_entries()
        .unwrap()
        .into_iter()
        .find(|e| e.resource.ends_with("AccentColorMenu"))
        .unwrap();
    assert_eq!(entry.pending, None);
    assert_eq!(
        entry.applied,
        tintcast_engine::ResourceState::Registry {
            value: RegistryValue::Dword(shade(&moondust()))
        }
    );
    let report = engine.restore().unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|i| i.result != RestoreResult::KeptYourChange),
        "{report:?}"
    );
    assert_eq!(
        fake.registry(ACCENT, "AccentColorMenu"),
        Some(RegistryValue::Dword(0x00_d7_78_00))
    );
    assert_eq!(fake.registry(DWM, "AccentColor"), None);
}

#[test]
fn a_run_id_that_is_not_a_journal_name_reaches_no_file_outside_the_journal() {
    let f = fixture(five_steps());
    seed_user_setup(&f.fake);
    crash_after(2, f.fake.clone(), &f.data, &f.lock);
    let run = f.engine.startup().unwrap().interrupted.remove(0);
    let outside = f.data.root().join("outside.json");
    std::fs::copy(
        f.data.journal().join(format!("{}.json", run.run_id)),
        &outside,
    )
    .unwrap();

    for id in [r"..\outside", "../outside", "", "a.b"] {
        assert!(matches!(
            f.engine.roll_back(id).unwrap_err(),
            EngineError::UnknownRun(_)
        ));
        assert!(matches!(
            f.engine.keep_interrupted(id).unwrap_err(),
            EngineError::UnknownRun(_)
        ));
    }

    assert!(outside.exists());
    assert_eq!(
        f.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(0))
    );
}

#[test]
fn a_crash_inside_a_registry_transaction_leaves_none_of_it_written() {
    let (_temp, data) = data_dir();
    let fake = Arc::new(FakeServices::new());
    seed_user_setup(&fake);
    let lock = unique_lock();
    let count = Arc::new(AtomicUsize::new(0));
    let services = Arc::new(Hooked::before(fake.clone(), move |resource| {
        if count.fetch_add(1, Ordering::SeqCst) == 2 {
            panic!("power lost before writing {resource}");
        }
    }));
    let crashing = Engine::new(data.clone(), services, vec![Box::new(system_integration())])
        .with_lock(lock.clone());
    assert!(
        catch_unwind(AssertUnwindSafe(
            || crashing.apply(&ApplyRequest::new(moondust()))
        ))
        .is_err()
    );
    assert_eq!(
        fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1)),
        "the transaction never committed"
    );

    let engine = Engine::new(
        data.clone(),
        fake.clone(),
        vec![Box::new(system_integration())],
    )
    .with_lock(lock);
    let run = engine.startup().unwrap().interrupted.remove(0);
    assert!(run.applied.is_empty());
    assert_eq!(run.pending.len(), 4);
    assert!(run.pending.iter().all(|s| s.state == StepState::Pending));
    fake.clear_calls();
    let report = engine.roll_back(&run.run_id).unwrap();

    assert!(
        report
            .items
            .iter()
            .all(|i| i.result == RestoreResult::Unchanged),
        "{report:?}"
    );
    assert!(fake.writes().is_empty());
    assert!(engine.ledger_entries().unwrap().is_empty());
    assert!(journal_files(&data).is_empty());
}
