use std::path::PathBuf;

use crate::limits::{MAX_COLOR_FILE_BYTES, MAX_COLOR_KEYS, MAX_NAME_CHARS, MAX_WALLPAPERS};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    #[error("No colors.toml found in {}", path.display())]
    NoColors { path: PathBuf },
    #[error("{file} needs background and foreground.")]
    NeedsBackgroundAndForeground { file: String },
    #[error("{file} is larger than {} KB.", MAX_COLOR_FILE_BYTES / 1000)]
    FileTooLarge { file: String },
    #[error("{file} has more than {MAX_COLOR_KEYS} keys.")]
    TooManyKeys { file: String },
    #[error("{file} is not valid TOML: {message}")]
    InvalidToml { file: String, message: String },
    #[error("{file}: {key} = {value} is not a #rgb or #rrggbb color.")]
    InvalidColor {
        file: String,
        key: String,
        value: String,
    },
    #[error("{file}: mode must be \"light\" or \"dark\", not {value}.")]
    InvalidMode { file: String, value: String },
    #[error("tintcast.toml: {key} {message}.")]
    InvalidSetting { key: String, message: String },
    #[error("The theme name is longer than {MAX_NAME_CHARS} characters.")]
    NameTooLong,
    #[error("The folder name {0} has no letters or digits to name the theme.")]
    Unnamable(String),
    #[error("The theme has {0} wallpapers, more than {MAX_WALLPAPERS}.")]
    TooManyWallpapers(usize),
    #[error("Tintcast can't read {}: {message}", path.display())]
    Io { path: PathBuf, message: String },
}
