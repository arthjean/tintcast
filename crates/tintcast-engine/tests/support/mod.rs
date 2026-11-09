#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use tintcast_engine::fake::FakeServices;
use tintcast_engine::{
    ApplyRequest, DataDir, Detection, Engine, EngineLock, Integration, Operation, Palette, Plan,
    RegistryValue, ReloadResult, Rgba, SystemResult, SystemServices, Theme, WallpaperPosition,
};

pub const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
pub const ACCENT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
pub const DWM: &str = r"Software\Microsoft\Windows\DWM";

fn palette(background: u8, foreground: u8) -> Palette {
    Palette::new(
        Rgba::rgb(background, background, background),
        Rgba::rgb(foreground, foreground, foreground),
    )
}

pub fn moondust() -> Theme {
    Theme::new("moondust", "Moondust", palette(0x17, 0xd9))
}

pub fn daylight() -> Theme {
    Theme::new("daylight", "Daylight", palette(0xf4, 0x2b))
}

pub fn ember() -> Theme {
    Theme::new("ember", "Ember", palette(0x20, 0xe0))
}

pub fn library(id: &str) -> Option<Theme> {
    [moondust(), daylight(), ember()]
        .into_iter()
        .find(|t| t.id == id)
}

pub fn shade(theme: &Theme) -> u32 {
    match theme.id.as_str() {
        "moondust" => 0x00_30_20_10,
        "daylight" => 0x00_f0_e0_d0,
        _ => 0x00_80_40_20,
    }
}

pub fn light(theme: &Theme) -> u32 {
    u32::from(theme.id == "daylight")
}

pub fn dword(key: &str, name: &str, value: u32) -> Operation {
    Operation::SetRegistryValue {
        key: key.into(),
        name: name.into(),
        value: Some(RegistryValue::Dword(value)),
    }
}

pub type PlanFn = dyn Fn(&ApplyRequest, &dyn SystemServices) -> Result<Plan, String> + Send + Sync;

pub struct Scripted {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub plan: Box<PlanFn>,
    pub reloads: Arc<AtomicUsize>,
}

impl Scripted {
    pub fn new(
        id: &str,
        name: &str,
        plan: impl Fn(&ApplyRequest, &dyn SystemServices) -> Result<Plan, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            installed: true,
            plan: Box::new(plan),
            reloads: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn not_installed(mut self) -> Self {
        self.installed = false;
        self
    }
}

impl Integration for Scripted {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn detect(&self, _services: &dyn SystemServices) -> Detection {
        if self.installed {
            Detection::installed(format!("{} is ready", self.name))
        } else {
            Detection::not_installed(format!("{} is not installed.", self.name))
        }
    }

    fn plan(
        &self,
        request: &ApplyRequest,
        _detection: &Detection,
        services: &dyn SystemServices,
    ) -> Result<Plan, String> {
        (self.plan)(request, services)
    }

    fn reload(&self, _services: &dyn SystemServices) -> ReloadResult {
        self.reloads.fetch_add(1, Ordering::SeqCst);
        ReloadResult::Reloaded
    }
}

pub fn system_integration() -> Scripted {
    Scripted::new("system", "Mode and accent", |request, _| {
        let theme = &request.theme;
        Ok(Plan::changes(vec![
            dword(PERSONALIZE, "AppsUseLightTheme", light(theme)),
            dword(PERSONALIZE, "SystemUsesLightTheme", light(theme)),
            dword(ACCENT, "AccentColorMenu", shade(theme)),
            dword(DWM, "AccentColor", shade(theme)),
        ]))
    })
}

pub fn wallpaper_integration(store: PathBuf) -> Scripted {
    Scripted::new("wallpaper", "Wallpaper", move |request, services| {
        let monitors = services.monitors().map_err(|e| e.to_string())?;
        let image = store.join(format!("{}-{}.png", request.theme.id, request.wallpaper));
        let mut ops: Vec<_> = monitors
            .into_iter()
            .map(|monitor| Operation::SetWallpaper {
                monitor,
                image: image.clone(),
            })
            .collect();
        ops.push(Operation::SetWallpaperPosition {
            position: WallpaperPosition::Fill,
        });
        Ok(Plan::changes(ops))
    })
}

pub fn terminal_paths() -> (PathBuf, PathBuf) {
    (
        PathBuf::from(
            r"C:\Users\u\AppData\Local\Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json",
        ),
        PathBuf::from(
            r"C:\Users\u\AppData\Local\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json",
        ),
    )
}

pub fn terminal_integration() -> Scripted {
    Scripted::new("terminal", "Windows Terminal", |request, _| {
        let (fragment, settings) = terminal_paths();
        let scheme = format!("Tintcast {}", request.theme.name);
        Ok(Plan::changes(vec![
            Operation::WriteManagedFile {
                path: fragment,
                contents: format!("{{\"schemes\":[{{\"name\":\"{scheme}\"}}]}}\n").into_bytes(),
            },
            Operation::SetJsonValue {
                path: settings,
                pointer: "/profiles/defaults/colorScheme".into(),
                value: Some(serde_json::Value::String(scheme)),
            },
        ]))
    })
}

pub const SETTINGS: &str = "{\n    // My Terminal settings\n    \"defaultProfile\": \"{61c54bbd-c2c6-5271-96e7-009a87ff44bf}\",\n    \"profiles\": {\n        \"defaults\": {\n            \"font\": { \"face\": \"Cascadia Mono\" } /* keep */\n        },\n        \"list\": []\n    }\n}\n";

pub type Hook = dyn Fn(&str) + Send + Sync;

pub struct Hooked {
    pub inner: Arc<FakeServices>,
    pub before_write: Box<Hook>,
    pub after_write: Option<Box<Hook>>,
}

impl Hooked {
    pub fn before(inner: Arc<FakeServices>, hook: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            inner,
            before_write: Box::new(hook),
            after_write: None,
        }
    }

    pub fn after(inner: Arc<FakeServices>, hook: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self {
            inner,
            before_write: Box::new(|_| {}),
            after_write: Some(Box::new(hook)),
        }
    }

    fn write<T>(&self, resource: String, call: impl FnOnce() -> T) -> T {
        (self.before_write)(&resource);
        let result = call();
        if let Some(after) = &self.after_write {
            after(&resource);
        }
        result
    }
}

impl SystemServices for Hooked {
    fn read_registry(&self, key: &str, name: &str) -> SystemResult<Option<RegistryValue>> {
        self.inner.read_registry(key, name)
    }

    fn write_registry(&self, key: &str, name: &str, value: &RegistryValue) -> SystemResult<()> {
        self.write(format!(r"reg:HKCU\{key}\{name}"), || {
            self.inner.write_registry(key, name, value)
        })
    }

    fn delete_registry(&self, key: &str, name: &str) -> SystemResult<()> {
        self.write(format!(r"reg:HKCU\{key}\{name}"), || {
            self.inner.delete_registry(key, name)
        })
    }

    fn monitors(&self) -> SystemResult<Vec<String>> {
        self.inner.monitors()
    }

    fn wallpaper(&self, monitor: &str) -> SystemResult<Option<PathBuf>> {
        self.inner.wallpaper(monitor)
    }

    fn set_wallpaper(&self, monitor: &str, image: Option<&Path>) -> SystemResult<()> {
        self.write(format!("wallpaper:{monitor}"), || {
            self.inner.set_wallpaper(monitor, image)
        })
    }

    fn wallpaper_position(&self) -> SystemResult<WallpaperPosition> {
        self.inner.wallpaper_position()
    }

    fn set_wallpaper_position(&self, position: WallpaperPosition) -> SystemResult<()> {
        self.write("wallpaper-position".to_owned(), || {
            self.inner.set_wallpaper_position(position)
        })
    }

    fn read_file(&self, path: &Path) -> SystemResult<Option<Vec<u8>>> {
        self.inner.read_file(path)
    }

    fn write_file(&self, path: &Path, bytes: &[u8]) -> SystemResult<()> {
        self.write(format!("file:{}", path.display()), || {
            self.inner.write_file(path, bytes)
        })
    }

    fn delete_file(&self, path: &Path) -> SystemResult<()> {
        self.write(format!("file:{}", path.display()), || {
            self.inner.delete_file(path)
        })
    }

    fn set_file_modified(&self, path: &Path, time: SystemTime) -> SystemResult<()> {
        self.inner.set_file_modified(path, time)
    }

    fn broadcast_color_change(&self) -> SystemResult<()> {
        self.inner.broadcast_color_change()
    }
}

pub fn unique_lock() -> EngineLock {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    EngineLock::new(
        format!(
            r"Local\Tintcast.Engine.it-{}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ),
        Duration::from_millis(500),
    )
}

pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub data: DataDir,
    pub fake: Arc<FakeServices>,
    pub lock: EngineLock,
    pub engine: Engine,
}

pub fn data_dir() -> (tempfile::TempDir, DataDir) {
    let temp = tempfile::tempdir().unwrap();
    let data = DataDir::open_in(Some(temp.path().into())).unwrap();
    (temp, data)
}

pub fn fixture(integrations: Vec<Box<dyn Integration>>) -> Fixture {
    let (temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&[r"\\?\DISPLAY#ONE"]));
    let lock = unique_lock();
    let engine = Engine::new(data.clone(), fake.clone(), integrations).with_lock(lock.clone());
    Fixture {
        temp,
        data,
        fake,
        lock,
        engine,
    }
}

pub fn standard(data: &DataDir) -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(system_integration()),
        Box::new(wallpaper_integration(data.wallpapers())),
        Box::new(terminal_integration()),
    ]
}

pub fn standard_fixture() -> Fixture {
    let (temp, data) = data_dir();
    let fake = Arc::new(FakeServices::with_monitors(&[r"\\?\DISPLAY#ONE"]));
    let lock = unique_lock();
    let engine = Engine::new(data.clone(), fake.clone(), standard(&data)).with_lock(lock.clone());
    Fixture {
        temp,
        data,
        fake,
        lock,
        engine,
    }
}

pub fn seed_user_setup(fake: &FakeServices) {
    fake.set_registry(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    fake.set_registry(
        PERSONALIZE,
        "SystemUsesLightTheme",
        Some(RegistryValue::Dword(1)),
    );
    fake.set_registry(
        ACCENT,
        "AccentColorMenu",
        Some(RegistryValue::Dword(0x00_d7_78_00)),
    );
    fake.set_monitor_wallpaper(
        r"\\?\DISPLAY#ONE",
        Some(Path::new(r"C:\Users\u\Pictures\beach.jpg")),
    );
    fake.set_position(WallpaperPosition::Fit);
    let (_, settings) = terminal_paths();
    fake.put_file(&settings, SETTINGS.as_bytes());
}

pub fn journal_files(data: &DataDir) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(data.journal())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
