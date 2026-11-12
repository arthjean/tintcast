mod allowlist;
pub mod appearance;
mod com;
mod desktop;
pub mod fake;
pub mod keys;
mod report;
mod services;
pub mod terminal;
pub mod wallpaper;

use std::sync::Arc;

pub use tintcast_engine as engine;
use tintcast_engine::{DataDir, Engine, Integration, SystemServices};

pub use allowlist::Allowlist;
pub use desktop::{BackgroundKind, Desktop, WindowsDesktop, local_app_data_from_env};
pub use report::{Report, report};
pub use services::{WindowsServices, registry_write_error};

pub const LOCAL_APP_DATA_UNSET: &str =
    "%LOCALAPPDATA% is not set to a full path, so Tintcast can't reach Windows settings.";

pub fn integrations(desktop: &Arc<dyn Desktop>) -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(appearance::Appearance::new(Arc::clone(desktop))),
        Box::new(wallpaper::Wallpapers::new(Arc::clone(desktop))),
        Box::new(terminal::Terminal::new(Arc::clone(desktop))),
    ]
}

#[derive(Clone)]
pub struct System {
    pub services: Arc<dyn SystemServices>,
    pub desktop: Arc<dyn Desktop>,
}

impl System {
    pub fn windows() -> Result<Self, String> {
        let local = local_app_data_from_env().ok_or_else(|| LOCAL_APP_DATA_UNSET.to_owned())?;
        Ok(Self {
            services: Arc::new(WindowsServices::new(&local)),
            desktop: Arc::new(WindowsDesktop),
        })
    }

    pub fn engine(&self, data: DataDir) -> Engine {
        Engine::new(
            data,
            Arc::clone(&self.services),
            integrations(&self.desktop),
        )
    }

    pub fn report(&self) -> Report {
        report(self.services.as_ref(), self.desktop.as_ref())
    }
}

pub fn describe(resource: &str) -> String {
    if let Some(rest) = resource.strip_prefix(r"reg:HKCU\") {
        let (key, name) = rest.rsplit_once('\\').unwrap_or(("", rest));
        return match name {
            "AppsUseLightTheme" => "app light or dark mode".to_owned(),
            "SystemUsesLightTheme" => "Windows light or dark mode".to_owned(),
            keys::BACKGROUND if key.eq_ignore_ascii_case(keys::COLORS) => {
                "solid background color".to_owned()
            }
            _ if key.eq_ignore_ascii_case(keys::ACCENT) || key.eq_ignore_ascii_case(keys::DWM) => {
                format!("accent color ({name})")
            }
            _ => format!(r"HKCU\{rest}"),
        };
    }
    if let Some(monitor) = resource.strip_prefix("wallpaper:") {
        return format!("wallpaper on display {monitor}");
    }
    if resource == "wallpaper-position" {
        return "wallpaper position".to_owned();
    }
    if let Some(path) = resource.strip_prefix("file:") {
        if path.to_ascii_lowercase().contains(r"\fragments\tintcast\") {
            return "Windows Terminal color schemes".to_owned();
        }
        return path.to_owned();
    }
    if let Some(rest) = resource.strip_prefix("json:") {
        let (path, pointer) = rest.rsplit_once('#').unwrap_or((rest, ""));
        if pointer == terminal::DEFAULT_SCHEME {
            return format!("Windows Terminal default color scheme in {path}");
        }
        return format!("{pointer} in {path}");
    }
    resource.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_resources_in_plain_words() {
        assert_eq!(
            describe(r"reg:HKCU\Software\Microsoft\Windows\DWM\AccentColor"),
            "accent color (AccentColor)"
        );
        assert_eq!(
            describe(
                r"reg:HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\SystemUsesLightTheme"
            ),
            "Windows light or dark mode"
        );
        assert_eq!(
            describe(r"reg:HKCU\Control Panel\Colors\Background"),
            "solid background color"
        );
        assert_eq!(describe("wallpaper:ONE"), "wallpaper on display ONE");
        assert_eq!(
            describe(r"file:C:\L\Microsoft\Windows Terminal\Fragments\Tintcast\tintcast.json"),
            "Windows Terminal color schemes"
        );
        assert_eq!(
            describe(r"json:C:\s\settings.json#/profiles/defaults/colorScheme"),
            r"Windows Terminal default color scheme in C:\s\settings.json"
        );
    }
}
