#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use tintcast_theme::{Mode, Rgba, ThemeError, WallpaperPosition, load_folder};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/omarchy")
}

fn fixture_folders() -> Vec<PathBuf> {
    let mut folders: Vec<_> = fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    folders
}

#[test]
fn all_twenty_three_omarchy_fixtures_load() {
    let folders = fixture_folders();
    assert_eq!(folders.len(), 23);
    for folder in folders {
        let loaded = load_folder(&folder).unwrap_or_else(|e| panic!("{}: {e}", folder.display()));
        let theme = loaded.theme;
        assert!(!theme.name.is_empty());
        assert_eq!(theme.position, WallpaperPosition::Fill);
        assert_eq!(theme.palette.ansi[0], theme.palette.background);
        assert_eq!(theme.palette.ansi[7], theme.palette.foreground);
        assert_eq!(theme.palette.ansi[8], theme.palette.muted);
        assert_eq!(
            Some(&theme.palette.accent),
            theme.palette.named.get("accent"),
            "{}",
            theme.slug
        );
    }
}

#[test]
fn catppuccin_maps_exactly() {
    let theme = load_folder(&fixtures().join("catppuccin")).unwrap().theme;
    let c = |t: &str| Rgba::parse(t).unwrap();
    assert_eq!(theme.slug, "catppuccin");
    assert_eq!(theme.name, "Catppuccin");
    assert_eq!(theme.palette.mode, Mode::Dark);
    assert_eq!(theme.palette.background, c("#1e1e2e"));
    assert_eq!(theme.palette.accent, c("#89b4fa"));
    assert_eq!(theme.palette.selection, c("#45475a"));
    assert_eq!(
        theme.palette.ansi.map(Rgba::hex),
        [
            "#1e1e2e", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#cdd6f4",
            "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#cdd6f4",
        ]
        .map(str::to_owned)
    );
}

#[test]
fn light_fixtures_are_light() {
    for name in ["catppuccin-latte", "flexoki-light", "white"] {
        let theme = load_folder(&fixtures().join(name)).unwrap().theme;
        assert_eq!(theme.palette.mode, Mode::Light, "{name}");
    }
}

fn theme_dir(colors: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sample-theme");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("colors.toml"), colors).unwrap();
    (root, dir)
}

#[test]
fn a_light_mode_marker_sets_light_when_mode_is_missing() {
    let (_root, dir) = theme_dir("background = \"#101010\"\nforeground = \"#eeeeee\"\n");
    assert_eq!(load_folder(&dir).unwrap().theme.palette.mode, Mode::Dark);
    fs::write(dir.join("light.mode"), "").unwrap();
    assert_eq!(load_folder(&dir).unwrap().theme.palette.mode, Mode::Light);
}

#[test]
fn a_theme_without_background_or_foreground_is_rejected() {
    let (_root, dir) = theme_dir("background = \"#101010\"\n");
    assert_eq!(
        load_folder(&dir).unwrap_err().to_string(),
        "colors.toml needs background and foreground."
    );
}

#[test]
fn a_colors_toml_over_sixty_four_kilobytes_is_rejected() {
    let mut text = "background = \"#101010\"\nforeground = \"#eeeeee\"\n".to_owned();
    while text.len() <= 64_000 {
        text.push_str("# padding padding padding padding padding padding padding\n");
    }
    let (_root, dir) = theme_dir(&text);
    assert_eq!(
        load_folder(&dir).unwrap_err(),
        ThemeError::FileTooLarge {
            file: "colors.toml".into()
        }
    );
    assert_eq!(
        load_folder(&dir).unwrap_err().to_string(),
        "colors.toml is larger than 64 KB."
    );
}

#[test]
fn backgrounds_are_listed_in_name_order_and_other_files_are_ignored() {
    let (_root, dir) = theme_dir("background = \"#101010\"\nforeground = \"#eeeeee\"\n");
    let backgrounds = dir.join("backgrounds");
    fs::create_dir_all(backgrounds.join("intros")).unwrap();
    for name in ["2-b.webp", "1-a.JPG", "intro.mp4", "10-c.png", "notes.txt"] {
        fs::write(backgrounds.join(name), b"x").unwrap();
    }
    for name in [
        "neovim.lua",
        "vscode.json",
        "icons.theme",
        "hyprland.conf.tpl",
    ] {
        fs::write(dir.join(name), b"os.execute('calc')").unwrap();
    }
    let loaded = load_folder(&dir).unwrap();
    let names: Vec<_> = loaded
        .backgrounds
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["1-a.JPG", "10-c.png", "2-b.webp"]);
    assert!(loaded.theme.warnings.is_empty());
}

#[test]
fn more_than_fifty_wallpapers_rejects_the_theme() {
    let (_root, dir) = theme_dir("background = \"#101010\"\nforeground = \"#eeeeee\"\n");
    let backgrounds = dir.join("backgrounds");
    fs::create_dir_all(&backgrounds).unwrap();
    for i in 0..51 {
        fs::write(backgrounds.join(format!("{i:02}.png")), b"x").unwrap();
    }
    assert_eq!(
        load_folder(&dir).unwrap_err().to_string(),
        "The theme has 51 wallpapers, more than 50."
    );
    fs::remove_file(backgrounds.join("50.png")).unwrap();
    assert_eq!(load_folder(&dir).unwrap().backgrounds.len(), 50);
}

#[test]
fn a_folder_without_colors_is_not_a_theme() {
    let root = tempfile::tempdir().unwrap();
    let error = load_folder(root.path()).unwrap_err();
    assert!(matches!(error, ThemeError::NoColors { .. }));
    assert!(error.to_string().starts_with("No colors.toml found in "));
}
