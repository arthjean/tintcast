use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tintcast_theme::limits::MAX_IMAGE_FILE_BYTES;
use tintcast_theme::wallpaper::{self, RejectedWallpaper, WallpaperError};
use tintcast_theme::{
    BACKGROUNDS, BUNDLED, LIGHT_MARKER, Origin, Palette, SETTINGS_FILE, Theme, ThemeFolder,
    Wallpaper, generate, is_bundled, is_link, is_theme_folder, load_folder, points_outside,
    read_at_most,
};

use crate::atomic::write_atomic;
use crate::data_dir::DataDir;
use crate::error::EngineError;
use crate::lock::EngineLock;
use crate::operation::sha256_hex;
use crate::store::Store;

static STAGING: AtomicU64 = AtomicU64::new(0);

pub const BUNDLED_REFUSED: &str = "Bundled themes can't be changed.";
pub const ALREADY_IN_LIBRARY: &str = "Already in your library";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("Bundled themes can't be changed.")]
    Bundled,
    #[error("No theme named {0}. Run tintcast list.")]
    NotFound(String),
    #[error("No colors.toml found in {}", .0.display())]
    NothingToImport(PathBuf),
    #[error("Tintcast can't use {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Engine(#[from] EngineError),
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> LibraryError + '_ {
    move |source| LibraryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug, Clone)]
pub struct LibraryOptions {
    pub lock: EngineLock,
    pub bundled_size: (u32, u32),
}

impl Default for LibraryOptions {
    fn default() -> Self {
        Self {
            lock: EngineLock::default(),
            bundled_size: (generate::WIDTH, generate::HEIGHT),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadReport {
    pub problems: Vec<String>,
    pub rejected: Vec<RejectedWallpaper>,
    pub pruned: Vec<PathBuf>,
    pub prune_skipped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported { wallpapers: usize, replaced: bool },
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportItem {
    pub source: PathBuf,
    pub slug: Option<String>,
    pub name: String,
    pub outcome: ImportOutcome,
    pub rejected: Vec<RejectedWallpaper>,
    pub skipped_entries: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub items: Vec<ImportItem>,
    pub skipped_entries: Vec<String>,
}

impl ImportReport {
    pub fn imported(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.outcome, ImportOutcome::Imported { .. }))
            .count()
    }
}

pub struct Library {
    data: DataDir,
    options: LibraryOptions,
    themes: Vec<Theme>,
    report: LoadReport,
}

fn content_key(bytes: &[u8]) -> String {
    sha256_hex(bytes)[..16].to_owned()
}

fn generated_key(palette: &Palette, variant: usize, (width, height): (u32, u32)) -> String {
    let mut text = format!(
        "tintcast-generated v{} variant {variant} {width}x{height} {} {} {} {}",
        generate::GENERATOR_VERSION,
        palette.mode.label(),
        palette.background,
        palette.foreground,
        palette.accent
    );
    for color in palette.ansi {
        text.push(' ');
        text.push_str(&color.hex());
    }
    content_key(text.as_bytes())
}

impl Library {
    pub fn open(data: DataDir) -> Result<Self, LibraryError> {
        Self::open_with(data, LibraryOptions::default())
    }

    pub fn open_with(data: DataDir, options: LibraryOptions) -> Result<Self, LibraryError> {
        let mut library = Self {
            data,
            options,
            themes: Vec::new(),
            report: LoadReport::default(),
        };
        let _guard = library.options.lock.acquire()?;
        library.reload();
        library.prune_locked();
        Ok(library)
    }

    pub fn data_dir(&self) -> &DataDir {
        &self.data
    }

    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    pub fn report(&self) -> &LoadReport {
        &self.report
    }

    pub fn get(&self, id: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.id == id)
    }

    pub fn find(&self, query: &str) -> Option<&Theme> {
        let query = query.trim();
        self.get(query).or_else(|| {
            self.themes
                .iter()
                .find(|t| t.id.eq_ignore_ascii_case(query) || t.name.eq_ignore_ascii_case(query))
        })
    }

    pub fn lookup(&self, id: &str) -> Option<Theme> {
        self.get(id).cloned()
    }

    fn thumbs(&self) -> PathBuf {
        self.data.wallpapers().join("thumbs")
    }

    fn stored(&self, theme_id: &str, key: &str) -> (PathBuf, PathBuf) {
        let name = format!("{theme_id}-{key}.png");
        (self.data.wallpapers().join(&name), self.thumbs().join(name))
    }

    fn store(
        &self,
        theme_id: &str,
        source: &str,
        key: &str,
        make: impl FnOnce() -> Result<wallpaper::Converted, WallpaperError>,
    ) -> Result<Wallpaper, RejectedWallpaper> {
        let (image, thumbnail) = self.stored(theme_id, key);
        if !(image.is_file() && thumbnail.is_file()) {
            let reject = |reason| RejectedWallpaper {
                file: source.to_owned(),
                reason,
            };
            let converted = make().map_err(reject)?;
            write_atomic(&image, &converted.png)
                .and_then(|()| write_atomic(&thumbnail, &converted.thumbnail))
                .map_err(|e| reject(WallpaperError::Store(e.to_string())))?;
        }
        Ok(Wallpaper {
            source: source.to_owned(),
            image,
            thumbnail,
        })
    }

    fn store_source(
        &self,
        theme_id: &str,
        path: &Path,
    ) -> Result<(Wallpaper, Vec<u8>), RejectedWallpaper> {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let reject = |reason| RejectedWallpaper {
            file: file.clone(),
            reason,
        };
        let meta = fs::symlink_metadata(path)
            .map_err(|e| reject(WallpaperError::Corrupt(e.to_string())))?;
        if is_link(&meta) {
            return Err(reject(WallpaperError::Corrupt(
                "it points outside the theme folder".into(),
            )));
        }
        if meta.len() > MAX_IMAGE_FILE_BYTES {
            return Err(reject(WallpaperError::FileTooLarge));
        }
        let bytes = read_at_most(path, MAX_IMAGE_FILE_BYTES)
            .map_err(|e| reject(WallpaperError::Corrupt(e.to_string())))?;
        let key = content_key(&bytes);
        let stored = self.store(theme_id, &file, &key, || wallpaper::convert(&bytes))?;
        Ok((stored, bytes))
    }

    fn bundled_themes(&mut self) -> Vec<Theme> {
        let size = self.options.bundled_size;
        let mut themes = Vec::new();
        for bundled in &BUNDLED {
            let parsed = match bundled.parse() {
                Ok(parsed) => parsed,
                Err(error) => {
                    self.report
                        .problems
                        .push(format!("Bundled theme {}: {error}", bundled.slug));
                    continue;
                }
            };
            let palette = &parsed.palette;
            let slug = parsed.slug.as_str();
            let results: Vec<_> = std::thread::scope(|scope| {
                let jobs: Vec<_> = (0..generate::VARIANTS)
                    .map(|variant| {
                        let key = generated_key(palette, variant, size);
                        let source = format!("{slug}-{}.png", variant + 1);
                        let this = &*self;
                        scope.spawn(move || {
                            this.store(slug, &source, &key, || {
                                wallpaper::encode(&generate::wallpaper(
                                    palette, variant, size.0, size.1,
                                ))
                            })
                        })
                    })
                    .collect();
                jobs.into_iter().map(|job| job.join()).collect()
            });
            let mut wallpapers = Vec::new();
            for result in results {
                match result {
                    Ok(Ok(stored)) => wallpapers.push(stored),
                    Ok(Err(rejected)) => self.report.rejected.push(rejected),
                    Err(_) => self.report.problems.push(format!(
                        "Bundled theme {}: a wallpaper could not be generated.",
                        parsed.slug
                    )),
                }
            }
            themes.push(Theme::from_parsed(parsed, Origin::Bundled, wallpapers));
        }
        themes
    }

    fn imported_theme(&mut self, folder: &Path) -> Option<Theme> {
        let label = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let loaded = match load_folder(folder) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.report
                    .problems
                    .push(format!("themes\\{label}: {error}"));
                return None;
            }
        };
        if is_bundled(&loaded.theme.slug) || loaded.theme.slug != label {
            self.report.problems.push(format!(
                "themes\\{label}: this folder is not a library theme, so it was ignored."
            ));
            return None;
        }
        let mut wallpapers = Vec::new();
        for source in &loaded.backgrounds {
            match self.store_source(&loaded.theme.slug, source) {
                Ok((stored, _)) => wallpapers.push(stored),
                Err(rejected) => self.report.rejected.push(rejected),
            }
        }
        self.report.problems.extend(loaded.skipped.iter().cloned());
        Some(Theme::from_parsed(
            loaded.theme,
            Origin::Imported,
            wallpapers,
        ))
    }

    fn reload(&mut self) {
        self.report.problems.clear();
        self.report.rejected.clear();
        let mut themes = self.bundled_themes();
        let root = self.data.themes();
        let mut folders: Vec<PathBuf> = match fs::read_dir(&root) {
            Ok(listing) => listing
                .filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter(|e| {
                    fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir() && !is_link(&m))
                })
                .map(|e| e.path())
                .collect(),
            Err(error) => {
                self.report
                    .problems
                    .push(format!("Tintcast can't read {}: {error}", root.display()));
                Vec::new()
            }
        };
        folders.sort();
        for folder in folders {
            if let Some(theme) = self.imported_theme(&folder) {
                themes.push(theme);
            }
        }
        themes.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        self.themes = themes;
    }

    fn prune_locked(&mut self) {
        self.report.pruned.clear();
        self.report.prune_skipped = None;
        let engine_refs = match Store::new(self.data.clone()).referenced_files() {
            Ok(refs) => refs,
            Err(error) => {
                self.report.prune_skipped = Some(error.to_string());
                return;
            }
        };
        let key = |path: &Path| path.to_string_lossy().to_lowercase();
        let mut keep: BTreeSet<String> = engine_refs.iter().map(|p| key(p)).collect();
        for theme in &self.themes {
            for stored in &theme.wallpapers {
                keep.insert(key(&stored.image));
                keep.insert(key(&stored.thumbnail));
            }
        }
        for folder in [self.data.wallpapers(), self.thumbs()] {
            let Ok(listing) = fs::read_dir(&folder) else {
                continue;
            };
            for entry in listing.filter_map(|e| e.ok()) {
                let path = entry.path();
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                let is_file = entry.file_type().is_ok_and(|t| t.is_file());
                if hidden || !is_file || keep.contains(&key(&path)) {
                    continue;
                }
                if fs::remove_file(&path).is_ok() {
                    self.report.pruned.push(path);
                }
            }
        }
    }

    pub fn import(&mut self, path: &Path, replace: bool) -> Result<ImportReport, LibraryError> {
        let root = fs::canonicalize(path).map_err(io_error(path))?;
        if !root.is_dir() {
            return Err(LibraryError::NothingToImport(path.to_path_buf()));
        }
        let mut report = ImportReport::default();
        let candidates = if is_theme_folder(&root) {
            vec![root.clone()]
        } else {
            let mut found = Vec::new();
            for entry in fs::read_dir(&root).map_err(io_error(&root))? {
                let entry = entry.map_err(io_error(&root))?;
                let child = entry.path();
                let Ok(meta) = fs::symlink_metadata(&child) else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_link(&meta) {
                    report.skipped_entries.push(points_outside(&name));
                } else if meta.is_dir() && is_theme_folder(&child) {
                    found.push(child);
                }
            }
            found.sort();
            found
        };
        if candidates.is_empty() {
            return Err(LibraryError::NothingToImport(path.to_path_buf()));
        }
        let _guard = self.options.lock.acquire()?;
        for folder in candidates {
            let item = self.import_one(&folder, replace);
            report.items.push(item);
        }
        self.reload();
        Ok(report)
    }

    fn import_one(&self, folder: &Path, replace: bool) -> ImportItem {
        let label = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut item = ImportItem {
            source: folder.to_path_buf(),
            slug: None,
            name: label,
            outcome: ImportOutcome::Skipped(String::new()),
            rejected: Vec::new(),
            skipped_entries: Vec::new(),
            warnings: Vec::new(),
        };
        let loaded = match load_folder(folder) {
            Ok(loaded) => loaded,
            Err(error) => {
                item.outcome = ImportOutcome::Skipped(error.to_string());
                return item;
            }
        };
        item.slug = Some(loaded.theme.slug.clone());
        item.name = loaded.theme.name.clone();
        item.skipped_entries = loaded.skipped.clone();
        item.warnings = loaded.theme.warnings.clone();
        let target = self.data.themes().join(&loaded.theme.slug);
        let exists = target.exists();
        if is_bundled(&loaded.theme.slug) {
            item.outcome = ImportOutcome::Skipped(BUNDLED_REFUSED.into());
            return item;
        }
        if exists && !replace {
            item.outcome = ImportOutcome::Skipped(ALREADY_IN_LIBRARY.into());
            return item;
        }
        item.outcome = match self.copy_in(&loaded, &target, exists, &mut item.rejected) {
            Ok(wallpapers) => ImportOutcome::Imported {
                wallpapers,
                replaced: exists,
            },
            Err(error) => ImportOutcome::Skipped(error.to_string()),
        };
        item
    }

    fn copy_in(
        &self,
        loaded: &ThemeFolder,
        target: &Path,
        exists: bool,
        rejected: &mut Vec<RejectedWallpaper>,
    ) -> Result<usize, LibraryError> {
        let slug = &loaded.theme.slug;
        let stamp = format!(
            "{slug}-{}-{}",
            std::process::id(),
            STAGING.fetch_add(1, Ordering::Relaxed)
        );
        let staging = self.data.themes().join(format!(".import-{stamp}"));
        let result = self.stage(loaded, &staging, rejected).and_then(|count| {
            if exists {
                let old = self.data.themes().join(format!(".old-{stamp}"));
                fs::rename(target, &old).map_err(io_error(target))?;
                if let Err(error) = fs::rename(&staging, target) {
                    let _ = fs::rename(&old, target);
                    return Err(io_error(target)(error));
                }
                let _ = fs::remove_dir_all(&old);
            } else {
                fs::rename(&staging, target).map_err(io_error(target))?;
            }
            Ok(count)
        });
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }

    fn stage(
        &self,
        loaded: &ThemeFolder,
        staging: &Path,
        rejected: &mut Vec<RejectedWallpaper>,
    ) -> Result<usize, LibraryError> {
        let backgrounds = staging.join(BACKGROUNDS);
        fs::create_dir_all(&backgrounds).map_err(io_error(&backgrounds))?;
        let write = |name: &str, bytes: &[u8]| {
            let path = staging.join(name);
            fs::write(&path, bytes).map_err(io_error(&path))
        };
        write(loaded.files.colors_name, &loaded.files.colors)?;
        if let Some(settings) = &loaded.files.settings {
            write(SETTINGS_FILE, settings)?;
        }
        if loaded.files.light_marker {
            write(LIGHT_MARKER, b"")?;
        }
        let mut count = 0;
        for source in &loaded.backgrounds {
            match self.store_source(&loaded.theme.slug, source) {
                Ok((stored, bytes)) => {
                    let path = backgrounds.join(&stored.source);
                    fs::write(&path, bytes).map_err(io_error(&path))?;
                    count += 1;
                }
                Err(rejection) => rejected.push(rejection),
            }
        }
        Ok(count)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), LibraryError> {
        if is_bundled(id) || self.get(id).is_some_and(Theme::is_bundled) {
            return Err(LibraryError::Bundled);
        }
        let Some(theme) = self.get(id) else {
            return Err(LibraryError::NotFound(id.to_owned()));
        };
        let folder = self.data.themes().join(&theme.id);
        let _guard = self.options.lock.acquire()?;
        fs::remove_dir_all(&folder).map_err(io_error(&folder))?;
        self.reload();
        Ok(())
    }
}
