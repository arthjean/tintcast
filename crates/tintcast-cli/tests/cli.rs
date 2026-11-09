#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn tintcast(local_app_data: &Path, args: &[&str]) -> Output {
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
