use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tintcast_engine::{
    CommitError, DISPLAY_NOT_CONNECTED, RegistryChange, RegistryValue, SystemError, SystemResult,
    SystemServices, WallpaperPosition, write_atomic,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    DESKTOP_WALLPAPER_POSITION, DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH,
    DWPOS_TILE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
    SendMessageTimeoutW, SystemParametersInfoW, WM_SETTINGCHANGE,
};
use windows::core::HSTRING;
use windows_registry::{CURRENT_USER, Transaction, Type};

use crate::allowlist::Allowlist;
use crate::com;
use crate::terminal::fragment_dir;

const NOT_FOUND: u32 = 0x8007_0002;
const PATH_NOT_FOUND: u32 = 0x8007_0003;
const ACCESS_DENIED: u32 = 0x8007_0005;
const BROADCAST_TIMEOUT_MS: u32 = 5000;

fn code(error: &windows::core::Error) -> u32 {
    error.code().0 as u32
}

fn missing(error: &windows::core::Error) -> bool {
    matches!(code(error), NOT_FOUND | PATH_NOT_FOUND)
}

pub fn registry_write_error(name: &str, code: u32, message: &str) -> SystemError {
    if code == ACCESS_DENIED {
        SystemError::new(format!("Windows blocked the change to {name}."))
    } else {
        SystemError::new(format!("Windows could not change {name}: {message}"))
    }
}

fn transaction_error(action: &str, error: &windows::core::Error) -> CommitError {
    CommitError {
        index: None,
        error: SystemError::new(format!(
            "Windows could not {action} the registry changes: {}",
            error.message()
        )),
    }
}

pub fn position_to_windows(position: WallpaperPosition) -> DESKTOP_WALLPAPER_POSITION {
    match position {
        WallpaperPosition::Fill => DWPOS_FILL,
        WallpaperPosition::Fit => DWPOS_FIT,
        WallpaperPosition::Stretch => DWPOS_STRETCH,
        WallpaperPosition::Center => DWPOS_CENTER,
        WallpaperPosition::Tile => DWPOS_TILE,
        WallpaperPosition::Span => DWPOS_SPAN,
    }
}

pub fn position_from_windows(position: DESKTOP_WALLPAPER_POSITION) -> Option<WallpaperPosition> {
    WallpaperPosition::ALL
        .into_iter()
        .find(|p| position_to_windows(*p) == position)
}

fn remove_wallpaper() -> windows::core::Result<()> {
    let mut empty = [0u16; 1];
    unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            Some(empty.as_mut_ptr().cast()),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    }
}

#[derive(Debug, Clone)]
pub struct WindowsServices {
    allowlist: Allowlist,
    fragment_dir: PathBuf,
}

fn system(error: windows::core::Error) -> SystemError {
    SystemError::new(error.message())
}

impl WindowsServices {
    pub fn new(local_app_data: &Path) -> Self {
        Self {
            allowlist: Allowlist::new(local_app_data),
            fragment_dir: fragment_dir(local_app_data),
        }
    }

    fn allow_registry(&self, key: &str, name: &str) -> SystemResult<()> {
        if self.allowlist.registry(key, name) {
            Ok(())
        } else {
            Err(SystemError::new(format!(
                r"Tintcast does not write to HKCU\{key}\{name}."
            )))
        }
    }

    fn allow_file(&self, path: &Path) -> SystemResult<()> {
        if self.allowlist.file(path) {
            Ok(())
        } else {
            Err(SystemError::new(format!(
                "Tintcast does not write to {}.",
                path.display()
            )))
        }
    }

    fn connected(&self, monitor: &str) -> SystemResult<()> {
        if self.monitors()?.iter().any(|m| m == monitor) {
            Ok(())
        } else {
            Err(SystemError::new(DISPLAY_NOT_CONNECTED))
        }
    }
}

impl SystemServices for WindowsServices {
    fn read_registry(&self, key: &str, name: &str) -> SystemResult<Option<RegistryValue>> {
        let read_error = |error: windows::core::Error| {
            SystemError::new(format!(
                r"Tintcast can't read HKCU\{key}\{name}: {}",
                error.message()
            ))
        };
        let opened = match CURRENT_USER.open(key) {
            Ok(opened) => opened,
            Err(error) if missing(&error) => return Ok(None),
            Err(error) => return Err(read_error(error)),
        };
        let value = match opened.get_value(name) {
            Ok(value) => value,
            Err(error) if missing(&error) => return Ok(None),
            Err(error) => return Err(read_error(error)),
        };
        match value.ty() {
            Type::U32 => {
                let raw: [u8; 4] =
                    value
                        .get(..4)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| {
                            SystemError::new(format!(r"HKCU\{key}\{name} is a short DWORD"))
                        })?;
                Ok(Some(RegistryValue::Dword(u32::from_le_bytes(raw))))
            }
            Type::Bytes => Ok(Some(RegistryValue::Binary(value.to_vec()))),
            Type::String => String::try_from(value)
                .map(|text| Some(RegistryValue::String(text)))
                .map_err(read_error),
            other => Err(SystemError::new(format!(
                r"HKCU\{key}\{name} has the registry type {other:?}, which Tintcast does not change"
            ))),
        }
    }

    fn commit_registry(&self, changes: &[RegistryChange]) -> Result<(), CommitError> {
        if changes.is_empty() {
            return Ok(());
        }
        for (index, change) in changes.iter().enumerate() {
            self.allow_registry(&change.key, &change.name)
                .map_err(|error| CommitError::at(index, error))?;
        }
        let transaction = Transaction::new().map_err(|error| transaction_error("start", &error))?;
        for (index, change) in changes.iter().enumerate() {
            let fail = |error: windows::core::Error| {
                CommitError::at(
                    index,
                    registry_write_error(&change.name, code(&error), &error.message()),
                )
            };
            let mut options = CURRENT_USER.options();
            options.read().write().transaction(&transaction);
            match &change.value {
                Some(value) => {
                    let opened = options.create().open(&change.key).map_err(fail)?;
                    match value {
                        RegistryValue::Dword(n) => opened.set_u32(&change.name, *n),
                        RegistryValue::Binary(bytes) => {
                            opened.set_bytes(&change.name, Type::Bytes, bytes)
                        }
                        RegistryValue::String(text) => opened.set_string(&change.name, text),
                    }
                    .map_err(fail)?;
                }
                None => {
                    let opened = match options.open(&change.key) {
                        Ok(opened) => opened,
                        Err(error) if missing(&error) => continue,
                        Err(error) => return Err(fail(error)),
                    };
                    if let Err(error) = opened.remove_value(&change.name)
                        && !missing(&error)
                    {
                        return Err(fail(error));
                    }
                }
            }
        }
        transaction
            .commit()
            .map_err(|error| transaction_error("save", &error))
    }

    fn monitors(&self) -> SystemResult<Vec<String>> {
        com::wallpaper_api(com::connected_monitors).map_err(system)
    }

    fn wallpaper(&self, monitor: &str) -> SystemResult<Option<PathBuf>> {
        self.connected(monitor)?;
        let path = com::wallpaper_api(|api| com::wallpaper_of(api, monitor)).map_err(system)?;
        Ok((!path.is_empty()).then(|| PathBuf::from(path)))
    }

    fn set_wallpaper(&self, monitor: &str, image: Option<&Path>) -> SystemResult<()> {
        self.connected(monitor)?;
        let changed = match image {
            Some(image) => com::wallpaper_api(|api| unsafe {
                api.SetWallpaper(&HSTRING::from(monitor), &HSTRING::from(image.as_os_str()))
            }),
            None => remove_wallpaper(),
        };
        changed.map_err(|error| {
            SystemError::new(format!(
                "Windows could not change the wallpaper: {}",
                error.message()
            ))
        })
    }

    fn wallpaper_position(&self) -> SystemResult<WallpaperPosition> {
        let raw = com::wallpaper_api(|api| unsafe { api.GetPosition() }).map_err(system)?;
        position_from_windows(raw).ok_or_else(|| {
            SystemError::new(format!(
                "Windows reports an unknown wallpaper position {}",
                raw.0
            ))
        })
    }

    fn set_wallpaper_position(&self, position: WallpaperPosition) -> SystemResult<()> {
        com::wallpaper_api(|api| unsafe { api.SetPosition(position_to_windows(position)) })
            .map_err(system)
    }

    fn read_file(&self, path: &Path) -> SystemResult<Option<Vec<u8>>> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(SystemError::new(format!(
                "Tintcast can't read {}: {error}",
                path.display()
            ))),
        }
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> SystemResult<()> {
        self.allow_file(path)?;
        write_atomic(path, bytes).map_err(|error| {
            SystemError::new(format!("Tintcast can't write {}: {error}", path.display()))
        })
    }

    fn delete_file(&self, path: &Path) -> SystemResult<()> {
        self.allow_file(path)?;
        match fs::remove_file(path) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(SystemError::new(format!(
                    "Tintcast can't delete {}: {error}",
                    path.display()
                )));
            }
            _ => {}
        }
        if path.parent() == Some(self.fragment_dir.as_path()) {
            let _ = fs::remove_dir(&self.fragment_dir);
        }
        Ok(())
    }

    fn set_file_modified(&self, path: &Path, time: SystemTime) -> SystemResult<()> {
        self.allow_file(path)?;
        fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(time))
            .map_err(|error| {
                SystemError::new(format!("Tintcast can't touch {}: {error}", path.display()))
            })
    }

    fn broadcast_color_change(&self) -> SystemResult<()> {
        let area = HSTRING::from("ImmersiveColorSet");
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(area.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                BROADCAST_TIMEOUT_MS,
                None,
            )
        };
        if sent.0 == 0 {
            Err(SystemError::new(format!(
                "Windows did not deliver the color change: {}",
                windows::core::Error::from_thread().message()
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denied_registry_write_names_the_value() {
        assert_eq!(
            registry_write_error("AccentColor", ACCESS_DENIED, "Access is denied.").to_string(),
            "Windows blocked the change to AccentColor."
        );
        assert_eq!(
            registry_write_error("AccentColor", 0x8007_03f3, "Bad").to_string(),
            "Windows could not change AccentColor: Bad"
        );
    }

    #[test]
    fn positions_map_both_ways() {
        for position in WallpaperPosition::ALL {
            assert_eq!(
                position_from_windows(position_to_windows(position)),
                Some(position)
            );
        }
        assert_eq!(position_to_windows(WallpaperPosition::Fill), DWPOS_FILL);
        assert_eq!(position_from_windows(DESKTOP_WALLPAPER_POSITION(42)), None);
    }

    #[test]
    fn writes_outside_the_allowlist_are_refused_before_any_system_call() {
        let temp = tempfile::tempdir().unwrap();
        let services = WindowsServices::new(temp.path());
        let outside = temp.path().join("elsewhere.txt");
        let error = services.write_file(&outside, b"x").unwrap_err();
        assert!(error.to_string().starts_with("Tintcast does not write to"));
        assert!(!outside.exists());
        assert!(services.delete_file(&outside).is_err());
        assert!(
            services
                .set_file_modified(&outside, SystemTime::now())
                .is_err()
        );
        let error = services
            .commit_registry(&[
                RegistryChange::set(crate::keys::DWM, "AccentColor", RegistryValue::Dword(0)),
                RegistryChange::set(
                    r"Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers",
                    "BackgroundType",
                    RegistryValue::Dword(0),
                ),
            ])
            .unwrap_err();
        assert_eq!(error.index, Some(1));
        assert!(
            error
                .to_string()
                .starts_with("Tintcast does not write to HKCU")
        );
        assert_eq!(
            services
                .commit_registry(&[RegistryChange::delete(r"Software\Classes", "Anything")])
                .unwrap_err()
                .index,
            Some(0)
        );
        assert_eq!(services.commit_registry(&[]), Ok(()));
    }

    #[test]
    fn files_in_the_data_folder_are_written_atomically_and_touched() {
        let temp = tempfile::tempdir().unwrap();
        let services = WindowsServices::new(temp.path());
        let path = temp.path().join(r"Tintcast\probe.json");
        assert_eq!(services.read_file(&path).unwrap(), None);
        services.write_file(&path, b"{}").unwrap();
        assert_eq!(services.read_file(&path).unwrap().unwrap(), b"{}");
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        services.set_file_modified(&path, old).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), old);
        assert_eq!(fs::read(&path).unwrap(), b"{}");
        services.delete_file(&path).unwrap();
        services.delete_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn deleting_the_fragment_removes_its_empty_folder() {
        let temp = tempfile::tempdir().unwrap();
        let services = WindowsServices::new(temp.path());
        let fragment = fragment_dir(temp.path()).join("tintcast.json");
        services.write_file(&fragment, b"{}").unwrap();
        services.delete_file(&fragment).unwrap();
        assert!(!fragment_dir(temp.path()).exists());
        assert!(fragment_dir(temp.path()).parent().unwrap().exists());
    }

    #[test]
    fn reading_registry_values_is_safe_on_the_host() {
        let services = WindowsServices::new(Path::new(r"C:\nowhere"));
        assert_eq!(
            services
                .read_registry(r"Software\Tintcast-does-not-exist", "Nothing")
                .unwrap(),
            None
        );
        let value = services
            .read_registry(crate::keys::PERSONALIZE, "AppsUseLightTheme")
            .unwrap();
        assert!(matches!(value, None | Some(RegistryValue::Dword(0 | 1))));
    }
}
