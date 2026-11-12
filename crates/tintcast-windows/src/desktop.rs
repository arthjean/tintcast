use std::path::PathBuf;

use serde::Serialize;
use tintcast_engine::Rgba;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Shell::DSS_SLIDESHOW;
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

use crate::com::{self, Apartment};
use crate::keys::{NO_CHANGING_WALLPAPER, POLICY_ACTIVE_DESKTOP, WALLPAPERS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundKind {
    Picture,
    SolidColor,
    Slideshow,
    Spotlight,
}

impl BackgroundKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::SolidColor => "solid color",
            Self::Slideshow => "slideshow",
            Self::Spotlight => "Windows Spotlight background",
        }
    }
}

pub trait Desktop: Send + Sync {
    fn local_app_data(&self) -> Option<PathBuf>;
    fn contrast_theme_on(&self) -> bool;
    fn wallpaper_policy_blocks(&self) -> bool;
    fn background_kind(&self) -> BackgroundKind;
    fn accent_color(&self) -> Result<Rgba, String>;
    fn ui_colors(&self) -> Result<(Rgba, Rgba), String>;
    fn dwm_colorization(&self) -> Option<u32>;
    fn windows_build(&self) -> String;
}

pub fn local_app_data_from_env() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDesktop;

fn ui_color(settings: &UISettings, kind: UIColorType) -> Result<Rgba, String> {
    let color = settings.GetColorValue(kind).map_err(|e| e.message())?;
    Ok(Rgba::rgb(color.R, color.G, color.B))
}

fn ui_settings() -> Result<UISettings, String> {
    UISettings::new().map_err(|e| format!("Windows did not answer for its colors: {}", e.message()))
}

fn policy_set(key: &windows_registry::Key) -> bool {
    key.open(POLICY_ACTIVE_DESKTOP)
        .and_then(|k| k.get_u32(NO_CHANGING_WALLPAPER))
        .is_ok_and(|value| value != 0)
}

impl Desktop for WindowsDesktop {
    fn local_app_data(&self) -> Option<PathBuf> {
        local_app_data_from_env()
    }

    fn contrast_theme_on(&self) -> bool {
        let mut contrast = HIGHCONTRASTW {
            cbSize: u32::try_from(std::mem::size_of::<HIGHCONTRASTW>()).unwrap_or(0),
            ..Default::default()
        };
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&raw mut contrast).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        read.is_ok() && contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
    }

    fn wallpaper_policy_blocks(&self) -> bool {
        policy_set(CURRENT_USER) || policy_set(LOCAL_MACHINE)
    }

    fn background_kind(&self) -> BackgroundKind {
        let slideshow = com::wallpaper_api(|api| unsafe { api.GetStatus() })
            .is_ok_and(|state| state.0 & DSS_SLIDESHOW.0 != 0);
        if slideshow {
            return BackgroundKind::Slideshow;
        }
        let spotlight_image = com::wallpaper_paths()
            .unwrap_or_default()
            .iter()
            .any(|path| path.to_ascii_lowercase().contains(r"\desktopspotlight\"));
        let kind = CURRENT_USER
            .open(WALLPAPERS)
            .and_then(|key| key.get_u32("BackgroundType"))
            .ok();
        match kind {
            Some(1) => BackgroundKind::SolidColor,
            Some(2) => BackgroundKind::Slideshow,
            Some(3) => BackgroundKind::Spotlight,
            _ if spotlight_image => BackgroundKind::Spotlight,
            _ => BackgroundKind::Picture,
        }
    }

    fn accent_color(&self) -> Result<Rgba, String> {
        let _apartment = Apartment::enter();
        ui_color(&ui_settings()?, UIColorType::Accent)
    }

    fn ui_colors(&self) -> Result<(Rgba, Rgba), String> {
        let _apartment = Apartment::enter();
        let settings = ui_settings()?;
        Ok((
            ui_color(&settings, UIColorType::Background)?,
            ui_color(&settings, UIColorType::Foreground)?,
        ))
    }

    fn dwm_colorization(&self) -> Option<u32> {
        let mut color = 0u32;
        let mut opaque = BOOL::default();
        unsafe { DwmGetColorizationColor(&mut color, &mut opaque) }.ok()?;
        Some(color)
    }

    fn windows_build(&self) -> String {
        let Ok(key) = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
            return "unknown".to_owned();
        };
        let build = key.get_string("CurrentBuild").unwrap_or_default();
        let revision = key
            .get_u32("UBR")
            .map(|n| n.to_string())
            .unwrap_or_default();
        let version = key.get_string("DisplayVersion").unwrap_or_default();
        format!("{build}.{revision} ({version})")
    }
}
