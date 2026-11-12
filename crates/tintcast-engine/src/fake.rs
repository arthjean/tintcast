use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

use tintcast_theme::WallpaperPosition;

pub use crate::services::DISPLAY_NOT_CONNECTED;
use crate::services::{
    CommitError, RegistryChange, RegistryValue, SystemError, SystemResult, SystemServices,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CallKind {
    ReadRegistry,
    WriteRegistry,
    DeleteRegistry,
    Monitors,
    Wallpaper,
    SetWallpaper,
    WallpaperPosition,
    SetWallpaperPosition,
    ReadFile,
    WriteFile,
    DeleteFile,
    SetFileModified,
    BroadcastColorChange,
}

impl CallKind {
    pub fn is_write(self) -> bool {
        matches!(
            self,
            Self::WriteRegistry
                | Self::DeleteRegistry
                | Self::SetWallpaper
                | Self::SetWallpaperPosition
                | Self::WriteFile
                | Self::DeleteFile
                | Self::SetFileModified
                | Self::BroadcastColorChange
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub kind: CallKind,
    pub target: String,
}

#[derive(Debug, Clone)]
struct Failure {
    kind: CallKind,
    target: Option<String>,
    message: String,
}

#[derive(Debug, Clone)]
pub struct FakeFile {
    pub bytes: Vec<u8>,
    pub modified: SystemTime,
}

pub type FakeRegistry = BTreeMap<(String, String), RegistryValue>;

type CommitHook = dyn Fn(&mut FakeRegistry) + Send + Sync;

#[derive(Default)]
struct Hooks(Vec<Box<CommitHook>>);

impl fmt::Debug for Hooks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} commit hooks", self.0.len())
    }
}

#[derive(Debug, Default)]
struct State {
    registry: FakeRegistry,
    connected: Vec<String>,
    wallpapers: BTreeMap<String, PathBuf>,
    position: WallpaperPosition,
    files: BTreeMap<PathBuf, FakeFile>,
    broadcasts: usize,
    calls: Vec<Call>,
    failures: Vec<Failure>,
    hooks: Hooks,
}

impl State {
    fn record(&mut self, kind: CallKind, target: String) -> Option<String> {
        let failure = self
            .failures
            .iter()
            .find(|f| f.kind == kind && f.target.as_deref().is_none_or(|t| target.contains(t)))
            .map(|f| f.message.clone());
        self.calls.push(Call { kind, target });
        failure
    }
}

#[derive(Debug, Default)]
pub struct FakeServices {
    state: Mutex<State>,
}

impl FakeServices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_monitors(monitors: &[&str]) -> Self {
        let fake = Self::new();
        for monitor in monitors {
            fake.connect_monitor(monitor);
        }
        fake
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn call(
        &self,
        kind: CallKind,
        target: impl Into<String>,
    ) -> SystemResult<MutexGuard<'_, State>> {
        let mut state = self.state();
        match state.record(kind, target.into()) {
            Some(message) => Err(SystemError(message)),
            None => Ok(state),
        }
    }

    pub fn fail(&self, kind: CallKind, target: Option<&str>, message: &str) {
        self.state().failures.push(Failure {
            kind,
            target: target.map(str::to_owned),
            message: message.to_owned(),
        });
    }

    pub fn on_commit(&self, hook: impl Fn(&mut FakeRegistry) + Send + Sync + 'static) {
        self.state().hooks.0.push(Box::new(hook));
    }

    pub fn clear_failures(&self) {
        self.state().failures.clear();
    }

    pub fn calls(&self) -> Vec<Call> {
        self.state().calls.clone()
    }

    pub fn writes(&self) -> Vec<Call> {
        self.state()
            .calls
            .iter()
            .filter(|c| c.kind.is_write())
            .cloned()
            .collect()
    }

    pub fn clear_calls(&self) {
        self.state().calls.clear();
    }

    pub fn registry(&self, key: &str, name: &str) -> Option<RegistryValue> {
        self.state()
            .registry
            .get(&(key.to_owned(), name.to_owned()))
            .cloned()
    }

    pub fn set_registry(&self, key: &str, name: &str, value: Option<RegistryValue>) {
        let id = (key.to_owned(), name.to_owned());
        let mut state = self.state();
        match value {
            Some(value) => state.registry.insert(id, value),
            None => state.registry.remove(&id),
        };
    }

    pub fn connect_monitor(&self, monitor: &str) {
        let mut state = self.state();
        if !state.connected.iter().any(|m| m == monitor) {
            state.connected.push(monitor.to_owned());
        }
    }

    pub fn disconnect_monitor(&self, monitor: &str) {
        self.state().connected.retain(|m| m != monitor);
    }

    pub fn monitor_wallpaper(&self, monitor: &str) -> Option<PathBuf> {
        self.state().wallpapers.get(monitor).cloned()
    }

    pub fn set_monitor_wallpaper(&self, monitor: &str, image: Option<&Path>) {
        let mut state = self.state();
        match image {
            Some(image) => state
                .wallpapers
                .insert(monitor.to_owned(), image.to_path_buf()),
            None => state.wallpapers.remove(monitor),
        };
    }

    pub fn position(&self) -> WallpaperPosition {
        self.state().position
    }

    pub fn set_position(&self, position: WallpaperPosition) {
        self.state().position = position;
    }

    pub fn file(&self, path: &Path) -> Option<Vec<u8>> {
        self.state().files.get(path).map(|f| f.bytes.clone())
    }

    pub fn file_modified(&self, path: &Path) -> Option<SystemTime> {
        self.state().files.get(path).map(|f| f.modified)
    }

    pub fn put_file(&self, path: &Path, bytes: &[u8]) {
        self.state().files.insert(
            path.to_path_buf(),
            FakeFile {
                bytes: bytes.to_vec(),
                modified: SystemTime::now(),
            },
        );
    }

    pub fn remove_file(&self, path: &Path) {
        self.state().files.remove(path);
    }

    pub fn broadcasts(&self) -> usize {
        self.state().broadcasts
    }
}

fn registry_target(key: &str, name: &str) -> String {
    format!(r"{key}\{name}")
}

fn path_target(path: &Path) -> String {
    path.display().to_string()
}

impl SystemServices for FakeServices {
    fn read_registry(&self, key: &str, name: &str) -> SystemResult<Option<RegistryValue>> {
        let state = self.call(CallKind::ReadRegistry, registry_target(key, name))?;
        Ok(state
            .registry
            .get(&(key.to_owned(), name.to_owned()))
            .cloned())
    }

    fn commit_registry(&self, changes: &[RegistryChange]) -> Result<(), CommitError> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut state = self.state();
        for (index, change) in changes.iter().enumerate() {
            let kind = match change.value {
                Some(_) => CallKind::WriteRegistry,
                None => CallKind::DeleteRegistry,
            };
            if let Some(message) = state.record(kind, registry_target(&change.key, &change.name)) {
                return Err(CommitError::at(index, SystemError(message)));
            }
        }
        for change in changes {
            let id = (change.key.clone(), change.name.clone());
            match &change.value {
                Some(value) => state.registry.insert(id, value.clone()),
                None => state.registry.remove(&id),
            };
        }
        let hooks = std::mem::take(&mut state.hooks);
        for hook in &hooks.0 {
            hook(&mut state.registry);
        }
        state.hooks = hooks;
        Ok(())
    }

    fn monitors(&self) -> SystemResult<Vec<String>> {
        let state = self.call(CallKind::Monitors, "monitors")?;
        Ok(state.connected.clone())
    }

    fn wallpaper(&self, monitor: &str) -> SystemResult<Option<PathBuf>> {
        let state = self.call(CallKind::Wallpaper, monitor)?;
        if !state.connected.iter().any(|m| m == monitor) {
            return Err(SystemError::new(DISPLAY_NOT_CONNECTED));
        }
        Ok(state.wallpapers.get(monitor).cloned())
    }

    fn set_wallpaper(&self, monitor: &str, image: Option<&Path>) -> SystemResult<()> {
        let mut state = self.call(CallKind::SetWallpaper, monitor)?;
        if !state.connected.iter().any(|m| m == monitor) {
            return Err(SystemError::new(DISPLAY_NOT_CONNECTED));
        }
        match image {
            Some(image) => state
                .wallpapers
                .insert(monitor.to_owned(), image.to_path_buf()),
            None => state.wallpapers.remove(monitor),
        };
        Ok(())
    }

    fn wallpaper_position(&self) -> SystemResult<WallpaperPosition> {
        let state = self.call(CallKind::WallpaperPosition, "position")?;
        Ok(state.position)
    }

    fn set_wallpaper_position(&self, position: WallpaperPosition) -> SystemResult<()> {
        let mut state = self.call(CallKind::SetWallpaperPosition, "position")?;
        state.position = position;
        Ok(())
    }

    fn read_file(&self, path: &Path) -> SystemResult<Option<Vec<u8>>> {
        let state = self.call(CallKind::ReadFile, path_target(path))?;
        Ok(state.files.get(path).map(|f| f.bytes.clone()))
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> SystemResult<()> {
        let mut state = self.call(CallKind::WriteFile, path_target(path))?;
        state.files.insert(
            path.to_path_buf(),
            FakeFile {
                bytes: bytes.to_vec(),
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }

    fn delete_file(&self, path: &Path) -> SystemResult<()> {
        let mut state = self.call(CallKind::DeleteFile, path_target(path))?;
        state.files.remove(path);
        Ok(())
    }

    fn set_file_modified(&self, path: &Path, time: SystemTime) -> SystemResult<()> {
        let mut state = self.call(CallKind::SetFileModified, path_target(path))?;
        let file = state
            .files
            .get_mut(path)
            .ok_or_else(|| SystemError::new(format!("{} does not exist", path.display())))?;
        file.modified = time;
        Ok(())
    }

    fn broadcast_color_change(&self) -> SystemResult<()> {
        let mut state = self.call(CallKind::BroadcastColorChange, "ImmersiveColorSet")?;
        state.broadcasts += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = r"Software\Microsoft\Windows\DWM";

    #[test]
    fn keeps_state_in_memory_and_records_every_call() {
        let fake = FakeServices::with_monitors(&["one"]);
        fake.commit_registry(&[RegistryChange::set(
            KEY,
            "AccentColor",
            RegistryValue::Dword(7),
        )])
        .unwrap();
        assert_eq!(
            fake.read_registry(KEY, "AccentColor").unwrap(),
            Some(RegistryValue::Dword(7))
        );
        fake.set_wallpaper("one", Some(Path::new("a.png"))).unwrap();
        fake.write_file(Path::new("x.json"), b"{}").unwrap();
        fake.broadcast_color_change().unwrap();
        let kinds: Vec<_> = fake.calls().iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                CallKind::WriteRegistry,
                CallKind::ReadRegistry,
                CallKind::SetWallpaper,
                CallKind::WriteFile,
                CallKind::BroadcastColorChange
            ]
        );
        assert_eq!(fake.writes().len(), 4);
        assert_eq!(fake.monitor_wallpaper("one"), Some(PathBuf::from("a.png")));
        assert_eq!(fake.broadcasts(), 1);
    }

    #[test]
    fn a_named_call_fails_on_request_with_an_error_value() {
        let fake = FakeServices::new();
        fake.fail(CallKind::WriteRegistry, Some("AccentColor"), "denied");
        let error = fake
            .commit_registry(&[RegistryChange::set(
                KEY,
                "AccentColor",
                RegistryValue::Dword(1),
            )])
            .unwrap_err();
        assert_eq!(error.to_string(), "denied");
        fake.commit_registry(&[RegistryChange::set(
            KEY,
            "ColorizationColor",
            RegistryValue::Dword(1),
        )])
        .unwrap();
        assert_eq!(fake.registry(KEY, "AccentColor"), None);
        assert_eq!(fake.calls().len(), 2);
    }

    #[test]
    fn a_commit_lands_whole_or_not_at_all_and_hooks_see_it_whole() {
        let fake = FakeServices::new();
        let seen: std::sync::Arc<Mutex<Vec<[Option<RegistryValue>; 2]>>> =
            std::sync::Arc::default();
        let observer = seen.clone();
        fake.on_commit(move |registry| {
            let get = |name: &str| registry.get(&(KEY.to_owned(), name.to_owned())).cloned();
            observer
                .lock()
                .unwrap()
                .push([get("AccentColor"), get("ColorizationColor")]);
        });
        fake.fail(CallKind::WriteRegistry, Some("ColorizationColor"), "denied");
        let both = [
            RegistryChange::set(KEY, "AccentColor", RegistryValue::Dword(1)),
            RegistryChange::set(KEY, "ColorizationColor", RegistryValue::Dword(2)),
        ];
        let error = fake.commit_registry(&both).unwrap_err();
        assert_eq!(error.index, Some(1));
        assert_eq!(fake.registry(KEY, "AccentColor"), None);
        assert!(seen.lock().unwrap().is_empty());

        fake.clear_failures();
        fake.commit_registry(&both).unwrap();
        fake.commit_registry(&[RegistryChange::delete(KEY, "AccentColor")])
            .unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                [Some(RegistryValue::Dword(1)), Some(RegistryValue::Dword(2))],
                [None, Some(RegistryValue::Dword(2))],
            ]
        );
    }

    #[test]
    fn a_disconnected_display_reports_display_not_connected() {
        let fake = FakeServices::with_monitors(&["one", "two"]);
        fake.disconnect_monitor("two");
        assert_eq!(fake.monitors().unwrap(), ["one"]);
        assert_eq!(
            fake.wallpaper("two").unwrap_err().to_string(),
            DISPLAY_NOT_CONNECTED
        );
        assert!(fake.set_wallpaper("two", None).is_err());
    }
}
