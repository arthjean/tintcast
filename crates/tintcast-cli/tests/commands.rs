#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tintcast_cli::{Context, FAILED, SUCCESS, USAGE_ERROR, run};
use tintcast_windows::System;
use tintcast_windows::engine::fake::{CallKind, FakeServices};
use tintcast_windows::engine::{DataDir, EngineLock, LibraryOptions, RegistryValue};
use tintcast_windows::fake::FakeDesktop;
use tintcast_windows::keys::{DWM, PERSONALIZE, WATCHED};

static LOCKS: AtomicUsize = AtomicUsize::new(0);

const LOCAL: &str = r"C:\Users\u\AppData\Local";
const ONE: &str = r"\\?\DISPLAY#ONE";

struct Cli {
    _temp: tempfile::TempDir,
    root: PathBuf,
    fake: Arc<FakeServices>,
    context: Context,
}

struct Output {
    code: u8,
    out: String,
    err: String,
}

fn lock_name() -> String {
    format!(
        r"Local\Tintcast.Engine.cli-test-{}-{}",
        std::process::id(),
        LOCKS.fetch_add(1, Ordering::Relaxed)
    )
}

fn cli_with_lock(lock: EngineLock) -> Cli {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Tintcast");
    let fake = Arc::new(FakeServices::with_monitors(&[ONE]));
    fake.set_monitor_wallpaper(ONE, Some(Path::new(r"C:\Windows\Web\img0.jpg")));
    fake.set_registry(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    fake.set_registry(DWM, "AccentColor", Some(RegistryValue::Dword(0xffd4_7800)));
    let desktop = Arc::new(FakeDesktop::new(LOCAL).following(Arc::clone(&fake)));
    let system = System {
        services: fake.clone(),
        desktop,
    };
    let data_root = root.clone();
    let context = Context {
        system: Box::new(move || Ok(system.clone())),
        data: Box::new(move || DataDir::at(data_root.clone()).map_err(|e| e.to_string())),
        library: LibraryOptions {
            bundled_size: (64, 36),
            ..LibraryOptions::default()
        },
        lock,
    };
    Cli {
        _temp: temp,
        root,
        fake,
        context,
    }
}

fn cli() -> Cli {
    cli_with_lock(EngineLock::new(lock_name(), Duration::from_secs(5)))
}

impl Cli {
    fn run(&self, args: &[&str]) -> Output {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run(&args, &self.context, &mut out, &mut err);
        Output {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn snapshot(&self) -> Vec<String> {
        let mut values: Vec<String> = WATCHED
            .iter()
            .map(|(key, name)| format!("{key}\\{name}={:?}", self.fake.registry(key, name)))
            .collect();
        values.push(format!("{:?}", self.fake.monitor_wallpaper(ONE)));
        values.push(format!("{:?}", self.fake.position()));
        values
    }

    fn wallpaper_name(&self) -> String {
        self.fake
            .monitor_wallpaper(ONE)
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
}

#[test]
fn plan_prints_the_preview_and_writes_nothing() {
    let cli = cli();
    let output = cli.run(&["plan", "Daylight"]);
    assert_eq!(output.code, SUCCESS, "{}", output.err);
    assert!(
        output
            .out
            .starts_with("Plan for Daylight (wallpaper 1 of 3):"),
        "{}",
        output.out
    );
    assert!(
        output
            .out
            .contains("\nMode and accent\n  Set SystemUsesLightTheme to 1")
    );
    assert!(output.out.contains("\nWallpaper\n  Show daylight-"));
    assert!(
        output
            .out
            .contains("Windows Terminal: skipped. Windows Terminal is not installed.")
    );
    assert!(
        output
            .out
            .trim_end()
            .ends_with("planned. Nothing was changed.")
    );
    assert!(cli.fake.writes().is_empty());
    assert!(!cli.root.join("ledger.json").exists());
}

#[test]
fn an_unknown_theme_or_wallpaper_is_bad_usage() {
    let cli = cli();
    let output = cli.run(&["apply", "Moondust"]);
    assert_eq!(output.code, USAGE_ERROR);
    assert_eq!(
        output.err.trim(),
        "No theme named Moondust. Run tintcast list."
    );
    let output = cli.run(&["apply", "ironwood", "--wallpaper", "9"]);
    assert_eq!(output.code, USAGE_ERROR);
    assert_eq!(
        output.err.trim(),
        "Ironwood has 3 wallpapers. Pick --wallpaper 1 to 3."
    );
    assert!(cli.fake.writes().is_empty());
    let usage = cli.run(&[]);
    assert_eq!(usage.code, USAGE_ERROR);
    assert!(
        usage
            .err
            .starts_with("usage: tintcast <command>\n\nCommands:\n")
    );
}

#[test]
fn apply_undo_and_restore_go_through_the_engine() {
    let cli = cli();
    let before = cli.snapshot();
    let first = cli.run(&["apply", "ironwood", "--wallpaper", "2"]);
    assert_eq!(first.code, SUCCESS, "{}{}", first.out, first.err);
    assert!(
        first.out.starts_with("Applied Ironwood in "),
        "{}",
        first.out
    );
    assert!(first.out.contains("Mode and accent: Applied\n"));
    assert!(first.out.contains("Wallpaper: Applied\n"));
    let ironwood_two = cli.wallpaper_name();
    assert!(ironwood_two.starts_with("ironwood-"));
    assert_eq!(
        cli.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(0))
    );

    let status = cli.run(&["status", "--json"]);
    let status: serde_json::Value = serde_json::from_str(&status.out).unwrap();
    assert_eq!(status["version"], 1);
    assert_eq!(status["current"]["theme_id"], "ironwood");
    assert_eq!(status["current"]["wallpaper"], 2);
    assert!(status["ledger_entries"].as_u64().unwrap() >= 9);
    assert_eq!(status["interrupted"], serde_json::json!([]));

    assert_eq!(cli.run(&["apply", "Linen"]).code, SUCCESS);
    assert!(cli.wallpaper_name().starts_with("linen-"));
    let undo = cli.run(&["undo"]);
    assert_eq!(undo.code, SUCCESS, "{}", undo.err);
    assert!(undo.out.starts_with("Applied Ironwood in "), "{}", undo.out);
    assert_eq!(cli.wallpaper_name(), ironwood_two);
    let human = cli.run(&["status"]);
    assert!(
        human
            .out
            .starts_with("Current theme: Ironwood (wallpaper 2), applied "),
        "{}",
        human.out
    );

    let undo = cli.run(&["undo"]);
    assert_eq!(undo.code, SUCCESS);
    assert!(undo.out.starts_with("Restored "), "{}", undo.out);
    assert_eq!(cli.snapshot(), before);
    let nothing = cli.run(&["undo"]);
    assert_eq!(nothing.out.trim(), "Nothing to undo");
    assert_eq!(
        cli.run(&["restore"]).out.trim(),
        "Nothing to restore: Tintcast holds no changed setting."
    );
}

#[test]
fn restore_reports_kept_changes_and_puts_the_rest_back() {
    let cli = cli();
    let before = cli.snapshot();
    assert_eq!(cli.run(&["apply", "inkwell"]).code, SUCCESS);
    cli.fake
        .set_registry(DWM, "AccentColor", Some(RegistryValue::Dword(0xff00_ff00)));
    let restore = cli.run(&["restore"]);
    assert_eq!(restore.code, SUCCESS, "{}", restore.err);
    assert!(
        restore
            .out
            .contains("  Kept your change: accent color (AccentColor)\n"),
        "{}",
        restore.out
    );
    assert!(
        restore
            .out
            .contains("  Restored: Windows light or dark mode\n")
    );
    assert!(
        restore
            .out
            .contains(&format!("  Restored: wallpaper on display {ONE}\n"))
    );
    let mut after = cli.snapshot();
    let accent = WATCHED
        .iter()
        .position(|(key, name)| *key == DWM && *name == "AccentColor")
        .unwrap();
    after[accent] = before[accent].clone();
    assert_eq!(after, before);
}

#[test]
fn a_failed_integration_exits_with_one_and_names_the_value() {
    let cli = cli();
    cli.fake.fail(
        CallKind::WriteRegistry,
        Some("AccentColorMenu"),
        "Windows blocked the change to AccentColorMenu.",
    );
    let output = cli.run(&["apply", "ironwood"]);
    assert_eq!(output.code, FAILED);
    assert!(output.out.starts_with("Applied Ironwood in "));
    assert!(output.out.contains("with a failed part."));
    assert!(
        output
            .out
            .contains("\nMode and accent: Failed. Set AccentColorMenu to ")
            && output.out.contains(
                "Explorer\\Accent: Windows blocked the change to AccentColorMenu. Tintcast rolled back this part.\n"
            ),
        "{}",
        output.out
    );
    assert!(output.out.contains("Wallpaper: Applied"));
    assert_eq!(
        cli.fake.registry(PERSONALIZE, "AppsUseLightTheme"),
        Some(RegistryValue::Dword(1))
    );
}

#[test]
fn a_second_apply_waits_then_reports_another_apply_is_running() {
    let name = lock_name();
    let cli = cli_with_lock(EngineLock::new(name.clone(), Duration::from_millis(300)));
    let (held, wait_for_hold) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let guard = EngineLock::new(name, Duration::from_secs(1))
            .acquire()
            .unwrap();
        held.send(()).unwrap();
        wait_for_release.recv().unwrap();
        drop(guard);
    });
    wait_for_hold.recv().unwrap();
    let output = cli.run(&["apply", "ironwood"]);
    release.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(output.code, FAILED);
    assert_eq!(output.err.trim(), "Another apply is running.");
    assert!(cli.fake.writes().is_empty());
}

#[test]
fn report_prints_the_watched_values_as_json() {
    let cli = cli();
    let output = cli.run(&["report", "--json"]);
    assert_eq!(output.code, SUCCESS);
    let report: serde_json::Value = serde_json::from_str(&output.out).unwrap();
    assert_eq!(report["build"], "fake");
    let watched = report["watched"].as_object().unwrap();
    assert_eq!(
        watched[&format!(r"reg:HKCU\{PERSONALIZE}\AppsUseLightTheme")],
        serde_json::json!({"type": "dword", "data": 1})
    );
    assert_eq!(
        watched[&format!("wallpaper:{ONE}")],
        r"C:\Windows\Web\img0.jpg"
    );
    assert_eq!(watched["wallpaper-position"], "fill");
    assert_eq!(watched["ui-accent"], "#0078d4");
    assert_eq!(watched["terminal-fragment"], serde_json::Value::Null);
    assert!(cli.fake.writes().is_empty());
    let human = cli.run(&["report"]);
    assert!(human.out.starts_with("Windows build: fake\n"));
}
