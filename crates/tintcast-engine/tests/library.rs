#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use support::{Scripted, data_dir, unique_lock};
use tintcast_engine::fake::FakeServices;
use tintcast_engine::{
    ALREADY_IN_LIBRARY, ApplyRequest, BUNDLED_REFUSED, DataDir, Engine, ImportOutcome, Library,
    LibraryError, LibraryOptions, Mode, Operation, Origin, Plan, Rgba, Theme,
};

const SMALL: (u32, u32) = (96, 54);

fn options() -> LibraryOptions {
    LibraryOptions {
        lock: unique_lock(),
        bundled_size: SMALL,
    }
}

fn open(data: &DataDir) -> Library {
    Library::open_with(data.clone(), options()).unwrap()
}

fn png(seed: u8) -> Vec<u8> {
    let image = image::RgbImage::from_fn(64, 36, |x, y| {
        image::Rgb([seed, (x * 3) as u8, (y * 5) as u8])
    });
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn huge_png_header() -> Vec<u8> {
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    header.extend_from_slice(&20_000u32.to_be_bytes());
    header.extend_from_slice(&100u32.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0, 0xec, 0xef, 0x97, 0x30]);
    header
}

fn write_theme(dir: &Path, colors: &str, wallpapers: &[(&str, Vec<u8>)]) {
    fs::create_dir_all(dir.join("backgrounds")).unwrap();
    fs::write(dir.join("colors.toml"), colors).unwrap();
    for (name, bytes) in wallpapers {
        fs::write(dir.join("backgrounds").join(name), bytes).unwrap();
    }
}

const NIGHT: &str = "mode = \"dark\"\naccent = \"#3fa34d\"\nbackground = \"#101418\"\nforeground = \"#e0e4e8\"\nblue = \"#5080c0\"\n";

fn junction(link: &Path, target: &Path) {
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
}

fn files_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().unwrap().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn first_use_lists_four_bundled_themes_with_three_generated_wallpapers_each() {
    let (_temp, data) = data_dir();
    let library = open(&data);
    let names: Vec<_> = library.themes().iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Daylight", "Inkwell", "Ironwood", "Linen"]);
    for theme in library.themes() {
        assert_eq!(theme.origin, Origin::Bundled);
        assert_eq!(theme.wallpapers.len(), 3);
        for stored in &theme.wallpapers {
            let name = stored
                .image
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            assert!(name.starts_with(&format!("{}-", theme.id)), "{name}");
            assert_eq!(stored.image.parent().unwrap(), data.wallpapers());
            let image = image::open(&stored.image).unwrap();
            assert_eq!((image.width(), image.height()), SMALL);
            let thumb = image::open(&stored.thumbnail).unwrap();
            assert_eq!((thumb.width(), thumb.height()), (480, 270));
        }
    }
    assert!(
        library.report().problems.is_empty(),
        "{:?}",
        library.report()
    );
    let (_other_temp, other) = data_dir();
    let again = open(&other);
    for (a, b) in library.themes().iter().zip(again.themes()) {
        for (x, y) in a.wallpapers.iter().zip(&b.wallpapers) {
            assert_eq!(x.image.file_name(), y.image.file_name());
            assert!(fs::read(&x.image).unwrap() == fs::read(&y.image).unwrap());
        }
    }
}

#[test]
fn the_default_bundled_wallpaper_size_is_4k() {
    assert_eq!(LibraryOptions::default().bundled_size, (3840, 2160));
}

#[test]
fn bundled_themes_cannot_be_removed_or_imported_over() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    assert!(matches!(
        library.remove("inkwell"),
        Err(LibraryError::Bundled)
    ));
    assert_eq!(
        library.remove("inkwell").unwrap_err().to_string(),
        "Bundled themes can't be changed."
    );
    let source = temp.path().join("src").join("Inkwell");
    write_theme(&source, NIGHT, &[]);
    for replace in [false, true] {
        let report = library.import(&source, replace).unwrap();
        assert_eq!(
            report.items[0].outcome,
            ImportOutcome::Skipped(BUNDLED_REFUSED.into())
        );
    }
    assert!(!data.themes().join("inkwell").exists());
    assert!(library.get("inkwell").unwrap().is_bundled());
}

#[test]
fn importing_one_theme_copies_it_and_reports_rejected_wallpapers() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let source = temp.path().join("src").join("Night Shift");
    write_theme(
        &source,
        NIGHT,
        &[
            ("1-first.png", png(10)),
            ("2-second.png", png(20)),
            ("3-broken.webp", b"RIFF\x10\0\0\0WEBPVP8 garbage".to_vec()),
            ("4-huge.png", huge_png_header()),
        ],
    );
    fs::write(
        source.join("tintcast.toml"),
        "author = \"Arthur\"\nposition = \"span\"\n",
    )
    .unwrap();
    fs::write(source.join("neovim.lua"), "os.execute('calc')").unwrap();
    let report = library.import(&source, false).unwrap();
    let item = &report.items[0];
    assert_eq!(item.slug.as_deref(), Some("night-shift"));
    assert_eq!(item.name, "Night Shift");
    assert_eq!(
        item.outcome,
        ImportOutcome::Imported {
            wallpapers: 2,
            replaced: false
        }
    );
    let rejected: Vec<_> = item.rejected.iter().map(ToString::to_string).collect();
    assert_eq!(rejected.len(), 2);
    assert!(
        rejected[0].starts_with("3-broken.webp was skipped: "),
        "{rejected:?}"
    );
    assert_eq!(rejected[1], "4-huge.png was skipped: larger than 16384 px.");
    let copy = data.themes().join("night-shift");
    assert_eq!(files_in(&copy), ["colors.toml", "tintcast.toml"]);
    assert_eq!(
        files_in(&copy.join("backgrounds")),
        ["1-first.png", "2-second.png"]
    );
    fs::remove_dir_all(temp.path().join("src")).unwrap();
    let reopened = open(&data);
    let theme = reopened.find("Night Shift").unwrap();
    assert_eq!(theme.origin, Origin::Imported);
    assert_eq!(theme.author.as_deref(), Some("Arthur"));
    assert_eq!(theme.palette.mode, Mode::Dark);
    assert_eq!(theme.palette.accent, Rgba::rgb(0x3f, 0xa3, 0x4d));
    assert_eq!(theme.wallpapers.len(), 2);
    assert!(
        theme
            .wallpapers
            .iter()
            .all(|w| w.image.is_file() && w.thumbnail.is_file())
    );
    assert!(reopened.report().rejected.is_empty());
}

#[test]
fn importing_a_folder_of_themes_imports_each_subfolder() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let root = temp.path().join("themes-src");
    write_theme(&root.join("alpha"), NIGHT, &[("a.png", png(1))]);
    write_theme(
        &root.join("beta"),
        "background = \"#fafafa\"\nforeground = \"#202020\"\n",
        &[],
    );
    fs::create_dir_all(root.join("not-a-theme")).unwrap();
    fs::write(root.join("README.md"), "hi").unwrap();
    let report = library.import(&root, false).unwrap();
    let slugs: Vec<_> = report
        .items
        .iter()
        .map(|i| i.slug.clone().unwrap())
        .collect();
    assert_eq!(slugs, ["alpha", "beta"]);
    assert_eq!(report.imported(), 2);
    assert_eq!(library.get("beta").unwrap().palette.mode, Mode::Light);
    assert_eq!(library.themes().len(), 6);
}

#[test]
fn an_alacritty_theme_imports_from_its_color_tables() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let source = temp.path().join("old-school");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("alacritty.toml"),
        "[colors.primary]\nbackground = \"#1d1f21\"\nforeground = \"#c5c8c6\"\n[colors.normal]\nblue = \"#81a2be\"\n",
    )
    .unwrap();
    let report = library.import(&source, false).unwrap();
    assert!(matches!(
        report.items[0].outcome,
        ImportOutcome::Imported { .. }
    ));
    assert!(
        data.themes()
            .join("old-school")
            .join("alacritty.toml")
            .is_file()
    );
    let theme = open(&data).get("old-school").cloned().unwrap();
    assert_eq!(theme.palette.accent, Rgba::rgb(0x81, 0xa2, 0xbe));
}

#[test]
fn a_theme_already_in_the_library_needs_replace() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let source = temp.path().join("night");
    write_theme(&source, NIGHT, &[("a.png", png(1))]);
    library.import(&source, false).unwrap();
    fs::write(
        source.join("colors.toml"),
        NIGHT.replace("#101418", "#202428"),
    )
    .unwrap();
    let report = library.import(&source, false).unwrap();
    assert_eq!(
        report.items[0].outcome,
        ImportOutcome::Skipped(ALREADY_IN_LIBRARY.into())
    );
    assert_eq!(
        library.get("night").unwrap().palette.background,
        Rgba::rgb(0x10, 0x14, 0x18)
    );
    let report = library.import(&source, true).unwrap();
    assert_eq!(
        report.items[0].outcome,
        ImportOutcome::Imported {
            wallpapers: 1,
            replaced: true
        }
    );
    assert_eq!(
        library.get("night").unwrap().palette.background,
        Rgba::rgb(0x20, 0x24, 0x28)
    );
    let leftovers: Vec<_> = fs::read_dir(data.themes())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["night"]);
}

#[test]
fn a_folder_with_no_theme_imports_nothing() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let empty = temp.path().join("empty");
    fs::create_dir_all(empty.join("sub")).unwrap();
    let error = library.import(&empty, false).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("No colors.toml found in {}", empty.display())
    );
    assert_eq!(fs::read_dir(data.themes()).unwrap().count(), 0);
}

#[test]
fn junctions_are_skipped_and_nothing_outside_the_source_is_read() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let outside = temp.path().join("outside");
    write_theme(&outside.join("secret"), NIGHT, &[("leak.png", png(99))]);
    let root = temp.path().join("src");
    write_theme(&root.join("good"), NIGHT, &[("a.png", png(3))]);
    junction(&root.join("evil"), &outside.join("secret"));
    junction(
        &root.join("good").join("backgrounds").join("more"),
        &outside.join("secret").join("backgrounds"),
    );
    let sneaky = root.join("sneaky");
    fs::create_dir_all(&sneaky).unwrap();
    fs::write(sneaky.join("colors.toml"), NIGHT).unwrap();
    junction(
        &sneaky.join("backgrounds"),
        &outside.join("secret").join("backgrounds"),
    );
    let report = library.import(&root, false).unwrap();
    assert_eq!(
        report.skipped_entries,
        ["evil was skipped: it points outside the theme folder."]
    );
    let slugs: Vec<_> = report
        .items
        .iter()
        .map(|i| i.slug.clone().unwrap())
        .collect();
    assert_eq!(slugs, ["good", "sneaky"]);
    assert_eq!(
        report.items[0].skipped_entries,
        ["backgrounds\\more was skipped: it points outside the theme folder."]
    );
    assert_eq!(
        report.items[1].skipped_entries,
        ["backgrounds was skipped: it points outside the theme folder."]
    );
    assert_eq!(library.get("good").unwrap().wallpapers.len(), 1);
    assert!(library.get("sneaky").unwrap().wallpapers.is_empty());
    assert!(library.get("secret").is_none() && library.get("evil").is_none());
}

#[test]
fn a_stored_wallpaper_is_reused_without_decoding_and_named_per_theme() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    let bytes = png(42);
    write_theme(
        &temp.path().join("src").join("one"),
        NIGHT,
        &[("a.png", bytes.clone())],
    );
    write_theme(
        &temp.path().join("src").join("two"),
        NIGHT,
        &[("a.png", bytes)],
    );
    library.import(&temp.path().join("src"), false).unwrap();
    let one = library.get("one").unwrap().wallpapers[0].clone();
    let two = library.get("two").unwrap().wallpapers[0].clone();
    assert_ne!(one.image, two.image);
    assert!(
        one.image
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("one-")
    );
    assert!(
        two.image
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("two-")
    );
    fs::write(&one.image, b"marker").unwrap();
    let reopened = open(&data);
    assert_eq!(reopened.get("one").unwrap().wallpapers[0].image, one.image);
    assert_eq!(fs::read(&one.image).unwrap(), b"marker");
}

fn wallpaper_engine(data: &DataDir) -> Engine {
    let fake = Arc::new(FakeServices::with_monitors(&[r"\\?\DISPLAY#ONE"]));
    let integration = Scripted::new("wallpaper", "Wallpaper", |request, services| {
        let image = request.theme.wallpapers[request.wallpaper].image.clone();
        let monitors = services.monitors().map_err(|e| e.to_string())?;
        Ok(Plan::changes(
            monitors
                .into_iter()
                .map(|monitor| Operation::SetWallpaper {
                    monitor,
                    image: image.clone(),
                })
                .collect(),
        ))
    });
    Engine::new(data.clone(), fake, vec![Box::new(integration)]).with_lock(unique_lock())
}

#[test]
fn startup_deletes_stored_wallpapers_nothing_references() {
    let (temp, data) = data_dir();
    let mut library = open(&data);
    write_theme(
        &temp.path().join("src").join("night"),
        NIGHT,
        &[("a.png", png(1)), ("b.png", png(2))],
    );
    library
        .import(&temp.path().join("src").join("night"), false)
        .unwrap();
    let night: Theme = library.get("night").cloned().unwrap();
    let engine = wallpaper_engine(&data);
    let report = engine
        .apply(&ApplyRequest::new(night.clone()).with_wallpaper(1))
        .unwrap();
    assert!(!report.failed());
    let stray = data.wallpapers().join("gone-0123456789abcdef.png");
    fs::write(&stray, b"old").unwrap();
    fs::write(
        data.wallpapers()
            .join("thumbs")
            .join("gone-0123456789abcdef.png"),
        b"old",
    )
    .unwrap();
    library.remove("night").unwrap();
    let reopened = open(&data);
    let pruned: Vec<PathBuf> = reopened.report().pruned.clone();
    assert!(pruned.contains(&stray));
    assert!(!stray.exists());
    assert!(
        night.wallpapers[1].image.is_file(),
        "the ledger still references it"
    );
    assert!(!night.wallpapers[0].image.exists());
    assert!(!night.wallpapers[0].thumbnail.exists());
    for theme in reopened.themes() {
        for stored in &theme.wallpapers {
            assert!(stored.image.is_file() && stored.thumbnail.is_file());
        }
    }
    let ledger = data.ledger();
    fs::write(&ledger, b"{ not json").unwrap();
    let leftover = data.wallpapers().join("other-0123456789abcdef.png");
    fs::write(&leftover, b"x").unwrap();
    let blocked = open(&data);
    assert!(blocked.report().prune_skipped.is_some());
    assert!(leftover.exists());
}
