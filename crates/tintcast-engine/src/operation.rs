use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tintcast_theme::WallpaperPosition;

use crate::services::RegistryValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Operation {
    SetRegistryValue {
        key: String,
        name: String,
        value: Option<RegistryValue>,
    },
    SetWallpaper {
        monitor: String,
        image: PathBuf,
    },
    SetWallpaperPosition {
        position: WallpaperPosition,
    },
    WriteManagedFile {
        path: PathBuf,
        #[serde(skip)]
        contents: Vec<u8>,
    },
    SetJsonValue {
        path: PathBuf,
        pointer: String,
        value: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceState {
    Absent,
    Registry { value: RegistryValue },
    Wallpaper { image: PathBuf },
    WallpaperPosition { position: WallpaperPosition },
    File { sha256: String },
    Json { value: Value },
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Operation {
    pub fn resource(&self) -> String {
        match self {
            Self::SetRegistryValue { key, name, .. } => format!(r"reg:HKCU\{key}\{name}"),
            Self::SetWallpaper { monitor, .. } => format!("wallpaper:{monitor}"),
            Self::SetWallpaperPosition { .. } => "wallpaper-position".to_owned(),
            Self::WriteManagedFile { path, .. } => format!("file:{}", path.display()),
            Self::SetJsonValue { path, pointer, .. } => {
                format!("json:{}#{pointer}", path.display())
            }
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::SetRegistryValue {
                key,
                name,
                value: Some(RegistryValue::Dword(n)),
            } => format!(r"Set {name} to {n} in HKCU\{key}"),
            Self::SetRegistryValue {
                key,
                name,
                value: Some(RegistryValue::Binary(bytes)),
            } => format!(r"Set {name} to {} bytes in HKCU\{key}", bytes.len()),
            Self::SetRegistryValue {
                key,
                name,
                value: None,
            } => format!(r"Delete {name} from HKCU\{key}"),
            Self::SetWallpaper { monitor, image } => {
                format!("Show {} on display {monitor}", file_name(image))
            }
            Self::SetWallpaperPosition { position } => {
                format!("Set the wallpaper position to {}", position.label())
            }
            Self::WriteManagedFile { path, .. } => format!("Write {}", path.display()),
            Self::SetJsonValue {
                path,
                pointer,
                value: Some(_),
            } => format!("Set {pointer} in {}", path.display()),
            Self::SetJsonValue {
                path,
                pointer,
                value: None,
            } => format!("Remove {pointer} from {}", path.display()),
        }
    }

    pub fn file_path(&self) -> Option<&Path> {
        match self {
            Self::WriteManagedFile { path, .. } | Self::SetJsonValue { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn target_state(&self) -> ResourceState {
        match self {
            Self::SetRegistryValue {
                value: Some(value), ..
            } => ResourceState::Registry {
                value: value.clone(),
            },
            Self::SetWallpaper { image, .. } => ResourceState::Wallpaper {
                image: image.clone(),
            },
            Self::SetWallpaperPosition { position } => ResourceState::WallpaperPosition {
                position: *position,
            },
            Self::WriteManagedFile { contents, .. } => ResourceState::File {
                sha256: sha256_hex(contents),
            },
            Self::SetJsonValue {
                value: Some(value), ..
            } => ResourceState::Json {
                value: value.clone(),
            },
            Self::SetRegistryValue { value: None, .. } | Self::SetJsonValue { value: None, .. } => {
                ResourceState::Absent
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

    fn every_case() -> [Operation; 5] {
        [
            Operation::SetRegistryValue {
                key: PERSONALIZE.into(),
                name: "AppsUseLightTheme".into(),
                value: Some(RegistryValue::Dword(0)),
            },
            Operation::SetWallpaper {
                monitor: r"\\?\DISPLAY#ONE".into(),
                image: PathBuf::from(r"C:\w\moondust-1.png"),
            },
            Operation::SetWallpaperPosition {
                position: WallpaperPosition::Fill,
            },
            Operation::WriteManagedFile {
                path: PathBuf::from(r"C:\f\tintcast.json"),
                contents: b"{}".to_vec(),
            },
            Operation::SetJsonValue {
                path: PathBuf::from(r"C:\t\settings.json"),
                pointer: "/profiles/defaults/colorScheme".into(),
                value: Some(json!("Tintcast Moondust")),
            },
        ]
    }

    #[test]
    fn resource_keys_are_stable() {
        let keys: Vec<_> = every_case().iter().map(Operation::resource).collect();
        assert_eq!(
            keys,
            [
                r"reg:HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme",
                r"wallpaper:\\?\DISPLAY#ONE",
                "wallpaper-position",
                r"file:C:\f\tintcast.json",
                r"json:C:\t\settings.json#/profiles/defaults/colorScheme",
            ]
        );
    }

    #[test]
    fn every_case_has_a_one_line_summary() {
        let summaries: Vec<_> = every_case().iter().map(Operation::summary).collect();
        assert_eq!(
            summaries,
            [
                r"Set AppsUseLightTheme to 0 in HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                r"Show moondust-1.png on display \\?\DISPLAY#ONE",
                "Set the wallpaper position to Fill",
                r"Write C:\f\tintcast.json",
                r"Set /profiles/defaults/colorScheme in C:\t\settings.json",
            ]
        );
        for summary in summaries {
            assert!(!summary.contains('\n'));
        }
        let removal = Operation::SetRegistryValue {
            key: "K".into(),
            name: "N".into(),
            value: None,
        };
        assert_eq!(removal.summary(), r"Delete N from HKCU\K");
        let bytes = Operation::SetRegistryValue {
            key: "K".into(),
            name: "AccentPalette".into(),
            value: Some(RegistryValue::Binary(vec![0; 32])),
        };
        assert_eq!(bytes.summary(), r"Set AccentPalette to 32 bytes in HKCU\K");
    }

    #[test]
    fn the_ledger_form_of_a_managed_file_carries_no_contents() {
        let op = Operation::WriteManagedFile {
            path: PathBuf::from("a.json"),
            contents: b"secret bytes".to_vec(),
        };
        let text = serde_json::to_string(&op).unwrap();
        assert!(!text.contains("contents"));
        let back: Operation = serde_json::from_str(&text).unwrap();
        assert_eq!(back.resource(), op.resource());
    }

    #[test]
    fn sha256_matches_a_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
