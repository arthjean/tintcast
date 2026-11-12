use std::path::{Component, Path, PathBuf};

use tintcast_engine::DataDir;

use crate::keys::{ACCENT, BACKGROUND, COLORS, DWM, PERSONALIZE, RUN, RUN_VALUE};
use crate::terminal::{fragment_dir, settings_candidates};

#[derive(Debug, Clone)]
pub struct Allowlist {
    folders: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

fn same_text(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn normal(path: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::Normal(part) => parts.push(part.to_string_lossy().to_lowercase()),
        }
    }
    path.is_absolute().then_some(parts)
}

impl Allowlist {
    pub fn new(local_app_data: &Path) -> Self {
        Self {
            folders: vec![
                local_app_data.join(DataDir::FOLDER),
                fragment_dir(local_app_data),
            ],
            files: settings_candidates(local_app_data)
                .into_iter()
                .map(|(_, path)| path)
                .collect(),
        }
    }

    pub fn registry(&self, key: &str, name: &str) -> bool {
        [PERSONALIZE, ACCENT, DWM].iter().any(|k| same_text(k, key))
            || (same_text(key, COLORS) && same_text(name, BACKGROUND))
            || (same_text(key, RUN) && same_text(name, RUN_VALUE))
    }

    pub fn file(&self, path: &Path) -> bool {
        let Some(target) = normal(path) else {
            return false;
        };
        let inside = |folder: &PathBuf| {
            normal(folder).is_some_and(|folder| {
                target.len() > folder.len() && target[..folder.len()] == folder[..]
            })
        };
        self.folders.iter().any(inside)
            || self
                .files
                .iter()
                .any(|file| normal(file).is_some_and(|file| file == target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Allowlist {
        Allowlist::new(Path::new(r"C:\Users\u\AppData\Local"))
    }

    #[test]
    fn allows_only_the_listed_hkcu_locations() {
        let list = list();
        assert!(list.registry(PERSONALIZE, "AppsUseLightTheme"));
        assert!(list.registry(&PERSONALIZE.to_uppercase(), "AppsUseLightTheme"));
        assert!(list.registry(ACCENT, "AccentPalette"));
        assert!(list.registry(DWM, "ColorizationColor"));
        assert!(list.registry(COLORS, "Background"));
        assert!(list.registry(RUN, "Tintcast"));
        assert!(!list.registry(COLORS, "Window"));
        assert!(!list.registry(RUN, "OneDrive"));
        assert!(!list.registry(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers",
            "BackgroundType"
        ));
        assert!(!list.registry(r"Software\Policies", "Anything"));
    }

    #[test]
    fn allows_only_the_data_folder_the_fragment_folder_and_terminal_settings() {
        let list = list();
        let local = Path::new(r"C:\Users\u\AppData\Local");
        assert!(list.file(&local.join(r"Tintcast\ledger.json")));
        assert!(list.file(&local.join(r"TINTCAST\wallpapers\x.png")));
        assert!(
            list.file(&local.join(r"Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json"))
        );
        assert!(
            list.file(&local.join(
                r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"
            ))
        );
        assert!(list.file(&local.join(
            r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"
        )));
        assert!(list.file(&local.join(r"Microsoft\Windows Terminal\settings.json")));
        assert!(!list.file(&local.join("Tintcast")));
        assert!(!list.file(&local.join(r"Tintcast\..\evil.txt")));
        assert!(!list.file(&local.join(r"Tintcastle\x")));
        assert!(!list.file(&local.join(r"Microsoft\Windows Terminal\Fragments\Other\x.json")));
        assert!(!list.file(
            &local.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\state.json")
        ));
        assert!(!list.file(Path::new(r"C:\ProgramData\Tintcast\x")));
        assert!(!list.file(Path::new(r"Tintcast\ledger.json")));
    }
}
