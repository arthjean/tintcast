use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use tintcast_engine::fake::FakeServices;
use tintcast_engine::{RegistryValue, Rgba};

use crate::appearance::from_abgr;
use crate::desktop::{BackgroundKind, Desktop};
use crate::keys::DWM;

#[derive(Debug)]
struct State {
    local_app_data: Option<PathBuf>,
    contrast: bool,
    policy: bool,
    kind: BackgroundKind,
    accent: Option<Result<Rgba, String>>,
}

#[derive(Debug)]
pub struct FakeDesktop {
    state: Mutex<State>,
    services: Option<Arc<FakeServices>>,
}

impl FakeDesktop {
    pub fn new(local_app_data: impl Into<PathBuf>) -> Self {
        Self {
            state: Mutex::new(State {
                local_app_data: Some(local_app_data.into()),
                contrast: false,
                policy: false,
                kind: BackgroundKind::Picture,
                accent: None,
            }),
            services: None,
        }
    }

    pub fn following(mut self, services: Arc<FakeServices>) -> Self {
        self.services = Some(services);
        self
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn set_local_app_data(&self, path: Option<PathBuf>) {
        self.state().local_app_data = path;
    }

    pub fn set_contrast_theme(&self, on: bool) {
        self.state().contrast = on;
    }

    pub fn set_wallpaper_policy(&self, on: bool) {
        self.state().policy = on;
    }

    pub fn set_background_kind(&self, kind: BackgroundKind) {
        self.state().kind = kind;
    }

    pub fn set_accent(&self, accent: Option<Result<Rgba, String>>) {
        self.state().accent = accent;
    }
}

impl Desktop for FakeDesktop {
    fn local_app_data(&self) -> Option<PathBuf> {
        self.state().local_app_data.clone()
    }

    fn contrast_theme_on(&self) -> bool {
        self.state().contrast
    }

    fn wallpaper_policy_blocks(&self) -> bool {
        self.state().policy
    }

    fn background_kind(&self) -> BackgroundKind {
        self.state().kind
    }

    fn accent_color(&self) -> Result<Rgba, String> {
        if let Some(accent) = self.state().accent.clone() {
            return accent;
        }
        match self
            .services
            .as_ref()
            .and_then(|services| services.registry(DWM, "AccentColor"))
        {
            Some(RegistryValue::Dword(value)) => Ok(from_abgr(value)),
            _ => Err("no accent color is set".to_owned()),
        }
    }

    fn ui_colors(&self) -> Result<(Rgba, Rgba), String> {
        Ok((Rgba::rgb(0, 0, 0), Rgba::rgb(255, 255, 255)))
    }

    fn dwm_colorization(&self) -> Option<u32> {
        None
    }

    fn windows_build(&self) -> String {
        "fake".to_owned()
    }
}
