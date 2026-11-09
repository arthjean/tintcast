#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use tintcast_theme::{BUNDLED, Mode, generate, is_bundled, slugify};

const SCENE_NAMES: &[&str] = &[
    "Abyss",
    "Acme",
    "Aetheria",
    "Amber",
    "Aurora",
    "Basalt",
    "Blueprint",
    "Catppuccin",
    "Cold Aisle",
    "Concrete",
    "Confetti",
    "Control Room",
    "Core Memory",
    "Cyberdeck",
    "Deep Field",
    "Deprecated",
    "Dunes",
    "Ethereal",
    "Everforest",
    "Flexoki",
    "Glacier",
    "Gloss",
    "Greenbar",
    "Grid",
    "Gruvbox",
    "Guru",
    "Hackerman",
    "Handheld",
    "Hi-Fi",
    "Honeypot",
    "Indigo",
    "Kanagawa",
    "Kernel",
    "Lamplight",
    "Last Horizon",
    "Lumon",
    "Lupine",
    "Mainframe",
    "Manual",
    "Marginalia",
    "Matcha",
    "Matte Black",
    "Miasma",
    "Moondust",
    "Moss",
    "Neon Noir",
    "Night Orbit",
    "Nord",
    "Osaka Jade",
    "Overprint",
    "Packet",
    "Phosphor",
    "Primary",
    "Prism",
    "Punchcard",
    "Radiolaria",
    "Rain Bridge",
    "Rasterbar",
    "Ready",
    "Red Planet",
    "Red Team",
    "Retro 82",
    "Ristretto",
    "Rose Pine",
    "Safelight",
    "Solarpunk",
    "Solitude",
    "Teletext",
    "Tokyo Night",
    "Vantablack",
    "Vaporwave",
    "Vector",
    "Wafer",
    "Waveform",
    "White",
    "Zero Day",
    "Dracula",
    "Rosé Pine",
    "Catppuccin Latte",
    "Flexoki Light",
];

#[test]
fn four_bundled_themes_two_dark_and_two_light() {
    let themes: Vec<_> = BUNDLED.iter().map(|b| b.parse().unwrap()).collect();
    let names: Vec<_> = themes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Daylight", "Inkwell", "Ironwood", "Linen"]);
    let dark = themes
        .iter()
        .filter(|t| t.palette.mode == Mode::Dark)
        .count();
    assert_eq!(dark, 2);
    for theme in &themes {
        assert!(
            theme.warnings.is_empty(),
            "{}: {:?}",
            theme.name,
            theme.warnings
        );
        assert!(is_bundled(&theme.slug));
        assert_eq!(theme.author.as_deref(), Some("Tintcast"));
    }
}

#[test]
fn bundled_names_differ_from_omarchy_scene_and_well_known_themes() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/omarchy");
    let mut taken: Vec<String> = fs::read_dir(fixtures)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(taken.len(), 23);
    taken.extend(SCENE_NAMES.iter().map(|n| (*n).to_owned()));
    let taken: Vec<String> = taken.iter().filter_map(|n| slugify(n)).collect();
    for bundled in BUNDLED {
        let name = bundled.parse().unwrap().name;
        let slug = slugify(&name).unwrap();
        assert!(!taken.contains(&slug), "{name} is already a theme name");
        assert!(
            !taken.iter().any(|t| slug.contains(t.as_str())),
            "{name} contains a known theme name"
        );
    }
}

#[test]
fn bundled_palettes_meet_contrast_targets() {
    for bundled in BUNDLED {
        let theme = bundled.parse().unwrap();
        let p = &theme.palette;
        let text = p.foreground.contrast(p.background);
        assert!(text >= 4.5, "{}: foreground {text:.2}:1", theme.name);
        for slot in (1..=6).chain(9..=14) {
            let ratio = p.ansi[slot].contrast(p.background);
            assert!(ratio >= 3.0, "{}: ANSI {slot} {ratio:.2}:1", theme.name);
        }
    }
}

#[test]
fn bundled_wallpapers_are_three_deterministic_4k_images() {
    let theme = BUNDLED[1].parse().unwrap();
    assert_eq!(generate::VARIANTS, 3);
    assert_eq!((generate::WIDTH, generate::HEIGHT), (3840, 2160));
    let first = generate::wallpaper(&theme.palette, 0, generate::WIDTH, generate::HEIGHT);
    assert_eq!(first.dimensions(), (3840, 2160));
    let again = generate::wallpaper(&theme.palette, 0, generate::WIDTH, generate::HEIGHT);
    assert!(first.as_raw() == again.as_raw());
}
