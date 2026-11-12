#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Mutex;

static ENGINE_LOCK: Mutex<()> = Mutex::new(());

fn tintcast(local_app_data: &Path, args: &[&str]) -> Output {
    let _serial = ENGINE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    Command::new(env!("CARGO_BIN_EXE_tintcast"))
        .args(args)
        .env("LOCALAPPDATA", local_app_data)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn import_then_list_goes_through_the_data_folder() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    fs::create_dir_all(&local).unwrap();
    let theme = temp.path().join("src").join("tokyo-night");
    fs::create_dir_all(theme.join("backgrounds")).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tintcast-theme/tests/fixtures/omarchy/tokyo-night/colors.toml");
    fs::copy(fixture, theme.join("colors.toml")).unwrap();
    image::RgbImage::from_pixel(32, 18, image::Rgb([20, 30, 40]))
        .save(theme.join("backgrounds").join("1-plain.png"))
        .unwrap();
    fs::write(theme.join("backgrounds").join("2-bad.jpg"), b"nope").unwrap();

    let first = tintcast(&local, &["import", theme.to_str().unwrap()]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    let text = stdout(&first);
    assert!(
        text.contains("Imported Tokyo Night as tokyo-night (1 wallpaper)"),
        "{text}"
    );
    assert!(text.contains("2-bad.jpg was skipped: "), "{text}");
    assert!(text.contains("1 of 1 themes imported."), "{text}");
    assert!(
        local
            .join("Tintcast/themes/tokyo-night/colors.toml")
            .is_file()
    );

    let again = tintcast(&local, &["import", theme.to_str().unwrap()]);
    assert!(stdout(&again).contains("Skipped Tokyo Night: Already in your library"));
    let replaced = tintcast(&local, &["import", "--replace", theme.to_str().unwrap()]);
    assert!(stdout(&replaced).contains("Replaced Tokyo Night as tokyo-night"));

    fs::remove_dir_all(temp.path().join("src")).unwrap();
    let list = tintcast(&local, &["list"]);
    assert_eq!(list.status.code(), Some(0));
    let lines: Vec<_> = stdout(&list).lines().map(str::to_owned).collect();
    assert_eq!(
        lines,
        [
            "Daylight  (daylight, light, 3 wallpapers, bundled)",
            "Inkwell  (inkwell, dark, 3 wallpapers, bundled)",
            "Ironwood  (ironwood, dark, 3 wallpapers, bundled)",
            "Linen  (linen, light, 3 wallpapers, bundled)",
            "Tokyo Night  (tokyo-night, dark, 1 wallpaper)",
        ]
    );
}

#[test]
fn a_folder_without_themes_fails_and_bad_usage_exits_two() {
    let temp = tempfile::tempdir().unwrap();
    let empty = temp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let output = tintcast(temp.path(), &["import", empty.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(&format!("No colors.toml found in {}", empty.display())),
        "{error}"
    );
    assert_eq!(tintcast(temp.path(), &["import"]).status.code(), Some(2));
    assert_eq!(tintcast(temp.path(), &[]).status.code(), Some(2));
}

#[test]
fn read_only_commands_run_against_windows_without_changing_it() {
    let temp = tempfile::tempdir().unwrap();
    let report = tintcast(temp.path(), &["report", "--json"]);
    assert_eq!(report.status.code(), Some(0), "{report:?}");
    let value: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    let watched = value["watched"].as_object().unwrap();
    assert!(watched.contains_key(
        r"reg:HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme"
    ));
    assert!(watched.contains_key("wallpaper-position"));
    assert_eq!(watched["terminal-fragment"], serde_json::Value::Null);
    assert!(!temp.path().join("Tintcast").exists());

    let unknown = tintcast(temp.path(), &["apply", "Moondust"]);
    assert_eq!(unknown.status.code(), Some(2), "{unknown:?}");
    assert!(
        String::from_utf8_lossy(&unknown.stderr)
            .contains("No theme named Moondust. Run tintcast list.")
    );

    let plan = tintcast(temp.path(), &["plan", "linen", "--wallpaper", "2"]);
    assert_eq!(plan.status.code(), Some(0), "{plan:?}");
    let text = stdout(&plan);
    assert!(
        text.starts_with("Plan for Linen (wallpaper 2 of 3):"),
        "{text}"
    );
    assert!(text.contains("Windows Terminal: skipped. Windows Terminal is not installed."));
    assert!(text.trim_end().ends_with("Nothing was changed."));
    assert!(!temp.path().join(r"Tintcast\ledger.json").exists());

    let status = tintcast(temp.path(), &["status", "--json"]);
    let value: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["current"], serde_json::Value::Null);
    assert_eq!(value["ledger_entries"], 0);
}
