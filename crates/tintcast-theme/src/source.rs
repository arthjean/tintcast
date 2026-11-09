use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::WallpaperPosition;
use crate::color::Rgba;
use crate::error::ThemeError;
use crate::limits::{MAX_COLOR_FILE_BYTES, MAX_COLOR_KEYS, MAX_NAME_CHARS, MAX_WALLPAPERS};
use crate::palette::{Mode, Palette, is_known_key};
use crate::wallpaper::is_image_name;

pub const COLORS_FILE: &str = "colors.toml";
pub const ALACRITTY_FILE: &str = "alacritty.toml";
pub const SETTINGS_FILE: &str = "tintcast.toml";
pub const LIGHT_MARKER: &str = "light.mode";
pub const BACKGROUNDS: &str = "backgrounds";

const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeFiles {
    pub colors_name: &'static str,
    pub colors: Vec<u8>,
    pub settings: Option<Vec<u8>>,
    pub light_marker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTheme {
    pub slug: String,
    pub name: String,
    pub author: Option<String>,
    pub palette: Palette,
    pub position: WallpaperPosition,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeFolder {
    pub path: PathBuf,
    pub files: ThemeFiles,
    pub theme: ParsedTheme,
    pub backgrounds: Vec<PathBuf>,
    pub skipped: Vec<String>,
}

pub fn slugify(name: &str) -> Option<String> {
    let mut slug = String::new();
    let mut gap = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.extend(c.to_lowercase());
        } else {
            gap = true;
        }
    }
    if slug.is_empty() || RESERVED_NAMES.contains(&slug.as_str()) {
        None
    } else {
        Some(slug)
    }
}

pub fn display_name(slug: &str) -> String {
    slug.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

pub fn points_outside(entry: &str) -> String {
    format!("{entry} was skipped: it points outside the theme folder.")
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> ThemeError + '_ {
    move |error| ThemeError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

enum Entry {
    Missing,
    Link,
    File(fs::Metadata),
    Folder,
}

fn entry(path: &Path) -> Result<Entry, ThemeError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) => Ok(Entry::Link),
        Ok(meta) if meta.is_dir() => Ok(Entry::Folder),
        Ok(meta) => Ok(Entry::File(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Entry::Missing),
        Err(error) => Err(io_error(path)(error)),
    }
}

pub fn read_at_most(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_limited(path: &Path, meta: &fs::Metadata, name: &str) -> Result<Vec<u8>, ThemeError> {
    if meta.len() > MAX_COLOR_FILE_BYTES {
        return Err(ThemeError::FileTooLarge {
            file: name.to_owned(),
        });
    }
    let bytes = read_at_most(path, MAX_COLOR_FILE_BYTES).map_err(io_error(path))?;
    if bytes.len() as u64 > MAX_COLOR_FILE_BYTES {
        return Err(ThemeError::FileTooLarge {
            file: name.to_owned(),
        });
    }
    Ok(bytes)
}

pub fn is_theme_folder(path: &Path) -> bool {
    [COLORS_FILE, ALACRITTY_FILE]
        .iter()
        .any(|name| matches!(entry(&path.join(name)), Ok(Entry::File(_))))
}

pub fn load_folder(path: &Path) -> Result<ThemeFolder, ThemeError> {
    let folder_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug = slugify(&folder_name).ok_or_else(|| ThemeError::Unnamable(folder_name.clone()))?;
    let mut skipped = Vec::new();
    let mut colors = None;
    for name in [COLORS_FILE, ALACRITTY_FILE] {
        let file = path.join(name);
        match entry(&file)? {
            Entry::File(meta) => {
                colors = Some((name, read_limited(&file, &meta, name)?));
                break;
            }
            Entry::Link => skipped.push(points_outside(name)),
            Entry::Missing | Entry::Folder => {}
        }
    }
    let (colors_name, colors) = colors.ok_or_else(|| ThemeError::NoColors {
        path: path.to_path_buf(),
    })?;
    let settings_path = path.join(SETTINGS_FILE);
    let settings = match entry(&settings_path)? {
        Entry::File(meta) => Some(read_limited(&settings_path, &meta, SETTINGS_FILE)?),
        Entry::Link => {
            skipped.push(points_outside(SETTINGS_FILE));
            None
        }
        Entry::Missing | Entry::Folder => None,
    };
    let light_marker = match entry(&path.join(LIGHT_MARKER))? {
        Entry::File(_) => true,
        Entry::Link => {
            skipped.push(points_outside(LIGHT_MARKER));
            false
        }
        Entry::Missing | Entry::Folder => false,
    };
    let files = ThemeFiles {
        colors_name,
        colors,
        settings,
        light_marker,
    };
    let theme = parse(&slug, &files)?;
    let backgrounds = list_backgrounds(path, &mut skipped)?;
    if backgrounds.len() > MAX_WALLPAPERS {
        return Err(ThemeError::TooManyWallpapers(backgrounds.len()));
    }
    Ok(ThemeFolder {
        path: path.to_path_buf(),
        files,
        theme,
        backgrounds,
        skipped,
    })
}

fn list_backgrounds(path: &Path, skipped: &mut Vec<String>) -> Result<Vec<PathBuf>, ThemeError> {
    let folder = path.join(BACKGROUNDS);
    match entry(&folder)? {
        Entry::Folder => {}
        Entry::Link => {
            skipped.push(points_outside(BACKGROUNDS));
            return Ok(Vec::new());
        }
        Entry::Missing | Entry::File(_) => return Ok(Vec::new()),
    }
    let mut found = Vec::new();
    for item in fs::read_dir(&folder).map_err(io_error(&folder))? {
        let item = item.map_err(io_error(&folder))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let file = item.path();
        match entry(&file)? {
            Entry::Link => skipped.push(points_outside(&format!("{BACKGROUNDS}\\{name}"))),
            Entry::File(_) if is_image_name(&name) => found.push((name.to_lowercase(), file)),
            Entry::File(_) | Entry::Folder | Entry::Missing => {}
        }
    }
    found.sort();
    Ok(found.into_iter().map(|(_, file)| file).collect())
}

fn table(file: &str, bytes: &[u8]) -> Result<toml::Table, ThemeError> {
    let invalid = |message: String| ThemeError::InvalidToml {
        file: file.to_owned(),
        message,
    };
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("it is not UTF-8 text".into()))?;
    let table: toml::Table = toml::from_str(text).map_err(|e| invalid(e.message().to_owned()))?;
    if count_keys(&table) > MAX_COLOR_KEYS {
        return Err(ThemeError::TooManyKeys {
            file: file.to_owned(),
        });
    }
    Ok(table)
}

fn count_keys(table: &toml::Table) -> usize {
    fn nested(value: &toml::Value) -> usize {
        match value {
            toml::Value::Table(t) => count_keys(t),
            toml::Value::Array(items) => items.iter().map(nested).sum(),
            _ => 0,
        }
    }
    table.len() + table.values().map(nested).sum::<usize>()
}

fn color(file: &str, key: &str, value: &toml::Value) -> Result<Rgba, ThemeError> {
    value
        .as_str()
        .and_then(|text| Rgba::parse(text).ok())
        .ok_or_else(|| ThemeError::InvalidColor {
            file: file.to_owned(),
            key: key.to_owned(),
            value: value.to_string(),
        })
}

struct Colors {
    named: BTreeMap<String, Rgba>,
    mode: Option<Mode>,
    warnings: Vec<String>,
}

fn omarchy_colors(bytes: &[u8]) -> Result<Colors, ThemeError> {
    let file = COLORS_FILE;
    let mut colors = Colors {
        named: BTreeMap::new(),
        mode: None,
        warnings: Vec::new(),
    };
    for (key, value) in table(file, bytes)? {
        if key == "mode" {
            colors.mode = Some(value.as_str().and_then(Mode::parse).ok_or_else(|| {
                ThemeError::InvalidMode {
                    file: file.to_owned(),
                    value: value.to_string(),
                }
            })?);
        } else if is_known_key(&key) {
            colors.named.insert(key.clone(), color(file, &key, &value)?);
        } else {
            colors
                .warnings
                .push(format!("{file}: the unknown key {key} was ignored."));
        }
    }
    Ok(colors)
}

fn alacritty_colors(bytes: &[u8]) -> Result<Colors, ThemeError> {
    const NAMES: [&str; 8] = [
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
    ];
    let file = ALACRITTY_FILE;
    let root = table(file, bytes)?;
    let section = |name: &str| {
        root.get("colors")
            .and_then(|c| c.get(name))
            .and_then(toml::Value::as_table)
    };
    let read = |key: &str, value: &toml::Value| {
        let normalized = match value.as_str() {
            Some(text) => match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                Some(digits) => toml::Value::String(format!("#{digits}")),
                None => value.clone(),
            },
            None => value.clone(),
        };
        color(file, key, &normalized)
    };
    let mut named = BTreeMap::new();
    if let Some(primary) = section("primary") {
        for key in ["background", "foreground"] {
            if let Some(value) = primary.get(key) {
                named.insert(key.to_owned(), read(key, value)?);
            }
        }
    }
    for (offset, name) in [(0, "normal"), (8, "bright")] {
        let Some(table) = section(name) else { continue };
        for (i, key) in NAMES.iter().enumerate() {
            if let Some(value) = table.get(*key) {
                named.insert(format!("color{}", offset + i), read(key, value)?);
            }
        }
    }
    if let Some(selection) = section("selection") {
        for (key, target) in [
            ("background", "selection_background"),
            ("text", "selection_foreground"),
        ] {
            if let Some(Ok(value)) = selection.get(key).map(|v| read(key, v)) {
                named.insert(target.to_owned(), value);
            }
        }
    }
    if let Some(&blue) = named.get("color4") {
        named.insert("blue".to_owned(), blue);
    }
    Ok(Colors {
        named,
        mode: None,
        warnings: Vec::new(),
    })
}

struct Settings {
    name: Option<String>,
    author: Option<String>,
    position: WallpaperPosition,
    accent: Option<Rgba>,
}

fn settings(bytes: Option<&[u8]>, warnings: &mut Vec<String>) -> Result<Settings, ThemeError> {
    let mut settings = Settings {
        name: None,
        author: None,
        position: WallpaperPosition::default(),
        accent: None,
    };
    let Some(bytes) = bytes else {
        return Ok(settings);
    };
    let invalid = |key: &str, message: &str| ThemeError::InvalidSetting {
        key: key.to_owned(),
        message: message.to_owned(),
    };
    for (key, value) in table(SETTINGS_FILE, bytes)? {
        let text = || {
            value
                .as_str()
                .map(str::trim)
                .ok_or_else(|| invalid(&key, "must be text"))
        };
        match key.as_str() {
            "name" => settings.name = Some(text()?.to_owned()).filter(|n| !n.is_empty()),
            "author" => settings.author = Some(text()?.to_owned()).filter(|a| !a.is_empty()),
            "position" => {
                let wanted = text()?;
                settings.position = WallpaperPosition::ALL
                    .into_iter()
                    .find(|p| p.label().eq_ignore_ascii_case(wanted))
                    .ok_or_else(|| {
                        invalid(&key, "must be Fill, Fit, Stretch, Center, Tile, or Span")
                    })?;
            }
            "accent" => {
                settings.accent = Some(
                    Rgba::parse(text()?)
                        .map_err(|_| invalid(&key, "must be a #rgb or #rrggbb color"))?,
                );
            }
            _ => warnings.push(format!(
                "{SETTINGS_FILE}: the unknown key {key} was ignored."
            )),
        }
    }
    Ok(settings)
}

pub fn parse(slug: &str, files: &ThemeFiles) -> Result<ParsedTheme, ThemeError> {
    let colors = if files.colors_name == ALACRITTY_FILE {
        alacritty_colors(&files.colors)?
    } else {
        omarchy_colors(&files.colors)?
    };
    let mut warnings = colors.warnings;
    let settings = settings(files.settings.as_deref(), &mut warnings)?;
    let mut palette = Palette::resolve(
        colors.named,
        colors.mode,
        files.light_marker,
        files.colors_name,
    )?;
    if let Some(accent) = settings.accent {
        palette = palette.with_accent(accent);
    }
    let name = settings.name.unwrap_or_else(|| display_name(slug));
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ThemeError::NameTooLong);
    }
    Ok(ParsedTheme {
        slug: slug.to_owned(),
        name,
        author: settings.author,
        palette,
        position: settings.position,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(colors: &str) -> ThemeFiles {
        ThemeFiles {
            colors_name: COLORS_FILE,
            colors: colors.as_bytes().to_vec(),
            settings: None,
            light_marker: false,
        }
    }

    const MINIMAL: &str = "background = \"#101010\"\nforeground = \"#f0f0f0\"\n";

    #[test]
    fn slugs_and_names_come_from_folder_names() {
        assert_eq!(
            slugify("Catppuccin Latte").as_deref(),
            Some("catppuccin-latte")
        );
        assert_eq!(slugify("retro-82").as_deref(), Some("retro-82"));
        assert_eq!(slugify("  --My__Theme!! ").as_deref(), Some("my-theme"));
        assert_eq!(slugify("..\\.."), None);
        assert_eq!(slugify("CON"), None);
        assert_eq!(display_name("catppuccin-latte"), "Catppuccin Latte");
        assert_eq!(display_name("retro-82"), "Retro 82");
    }

    #[test]
    fn reads_stop_one_byte_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big.bin");
        fs::write(&file, vec![7u8; 100]).unwrap();
        assert_eq!(read_at_most(&file, 10).unwrap().len(), 11);
        assert_eq!(read_at_most(&file, 1000).unwrap().len(), 100);
    }

    #[test]
    fn unknown_keys_are_ignored_with_a_warning() {
        let theme = parse(
            "t",
            &files(&format!("{MINIMAL}active_border_color = \"#fff\"\n")),
        )
        .unwrap();
        assert_eq!(
            theme.warnings,
            ["colors.toml: the unknown key active_border_color was ignored."]
        );
    }

    #[test]
    fn invalid_colors_and_modes_reject_the_theme_with_the_key_named() {
        let error = parse("t", &files(&format!("{MINIMAL}red = \"rgb(1,2,3)\"\n"))).unwrap_err();
        assert_eq!(
            error.to_string(),
            "colors.toml: red = \"rgb(1,2,3)\" is not a #rgb or #rrggbb color."
        );
        let error = parse("t", &files(&format!("{MINIMAL}blue = 12\n"))).unwrap_err();
        assert!(error.to_string().contains("blue = 12"));
        let error = parse("t", &files(&format!("{MINIMAL}mode = \"dim\"\n"))).unwrap_err();
        assert_eq!(
            error.to_string(),
            "colors.toml: mode must be \"light\" or \"dark\", not \"dim\"."
        );
    }

    #[test]
    fn more_than_one_hundred_keys_is_rejected() {
        let mut text = MINIMAL.to_owned();
        for i in 0..99 {
            text.push_str(&format!("extra_{i} = \"#000\"\n"));
        }
        let error = parse("t", &files(&text)).unwrap_err();
        assert_eq!(error.to_string(), "colors.toml has more than 100 keys.");
        let mut text = MINIMAL.to_owned();
        for i in 0..98 {
            text.push_str(&format!("extra_{i} = \"#000\"\n"));
        }
        assert!(parse("t", &files(&text)).is_ok());
    }

    #[test]
    fn a_name_longer_than_sixty_four_characters_is_rejected() {
        let mut theme = files(MINIMAL);
        theme.settings = Some(format!("name = \"{}\"\n", "x".repeat(65)).into_bytes());
        assert_eq!(
            parse("t", &theme).unwrap_err().to_string(),
            "The theme name is longer than 64 characters."
        );
        theme.settings = Some(format!("name = \"{}\"\n", "é".repeat(64)).into_bytes());
        assert!(parse("t", &theme).is_ok());
    }

    #[test]
    fn tintcast_toml_sets_name_author_position_and_accent() {
        let mut theme = files(&format!("{MINIMAL}blue = \"#0000ff\"\n"));
        theme.settings = Some(
            b"name = \"Night Shift\"\nauthor = \"Arthur\"\nposition = \"span\"\naccent = \"#3fa34d\"\nfoo = 1\n"
                .to_vec(),
        );
        let parsed = parse("night-shift", &theme).unwrap();
        assert_eq!(parsed.name, "Night Shift");
        assert_eq!(parsed.author.as_deref(), Some("Arthur"));
        assert_eq!(parsed.position, WallpaperPosition::Span);
        assert_eq!(parsed.palette.accent, Rgba::rgb(0x3f, 0xa3, 0x4d));
        assert_eq!(
            parsed.warnings,
            ["tintcast.toml: the unknown key foo was ignored."]
        );
        let default = parse("night-shift", &files(MINIMAL)).unwrap();
        assert_eq!(default.name, "Night Shift");
        assert_eq!(default.position, WallpaperPosition::Fill);
        theme.settings = Some(b"position = \"zoom\"\n".to_vec());
        assert!(parse("t", &theme).is_err());
    }

    #[test]
    fn alacritty_tables_are_read_when_colors_toml_is_missing() {
        let text = "[colors.primary]\nbackground = \"0x1d1f21\"\nforeground = \"#c5c8c6\"\n\
            [colors.normal]\nblack = \"#000000\"\nred = \"#cc6666\"\nblue = \"#81a2be\"\n\
            [colors.bright]\nwhite = \"#ffffff\"\n[colors.selection]\ntext = \"CellForeground\"\n";
        let parsed = parse(
            "t",
            &ThemeFiles {
                colors_name: ALACRITTY_FILE,
                colors: text.as_bytes().to_vec(),
                settings: None,
                light_marker: false,
            },
        )
        .unwrap();
        let palette = parsed.palette;
        assert_eq!(palette.background, Rgba::rgb(0x1d, 0x1f, 0x21));
        assert_eq!(palette.ansi[0], Rgba::rgb(0, 0, 0));
        assert_eq!(palette.ansi[1], Rgba::rgb(0xcc, 0x66, 0x66));
        assert_eq!(palette.ansi[15], Rgba::rgb(255, 255, 255));
        assert_eq!(palette.accent, Rgba::rgb(0x81, 0xa2, 0xbe));
        assert_eq!(palette.mode, Mode::Dark);
    }
}
