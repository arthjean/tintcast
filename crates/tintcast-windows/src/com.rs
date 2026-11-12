use std::cell::Cell;
use std::ffi::c_void;

use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper};
use windows::core::{HSTRING, PWSTR};

thread_local! {
    static APARTMENT: Cell<bool> = const { Cell::new(false) };
}

pub struct Apartment;

impl Apartment {
    pub fn enter() -> Self {
        APARTMENT.with(|entered| {
            if !entered.get() {
                let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                entered.set(true);
            }
        });
        Self
    }
}

pub fn take_pwstr(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let value = unsafe { text.to_string().ok() };
    unsafe { CoTaskMemFree(Some(text.0 as *const c_void)) };
    value
}

pub fn wallpaper_api<T>(
    call: impl FnOnce(&IDesktopWallpaper) -> windows::core::Result<T>,
) -> windows::core::Result<T> {
    let _apartment = Apartment::enter();
    let api: IDesktopWallpaper = unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? };
    call(&api)
}

pub fn connected_monitors(api: &IDesktopWallpaper) -> windows::core::Result<Vec<String>> {
    let count = unsafe { api.GetMonitorDevicePathCount()? };
    let mut monitors = Vec::new();
    for index in 0..count {
        let Some(id) = take_pwstr(unsafe { api.GetMonitorDevicePathAt(index)? }) else {
            continue;
        };
        let attached = unsafe { api.GetMonitorRECT(&HSTRING::from(id.as_str())) }
            .is_ok_and(|rect| rect.right > rect.left && rect.bottom > rect.top);
        if attached && !id.is_empty() {
            monitors.push(id);
        }
    }
    Ok(monitors)
}

pub fn wallpaper_of(api: &IDesktopWallpaper, monitor: &str) -> windows::core::Result<String> {
    let path = unsafe { api.GetWallpaper(&HSTRING::from(monitor))? };
    Ok(take_pwstr(path).unwrap_or_default())
}

pub fn wallpaper_paths() -> windows::core::Result<Vec<String>> {
    wallpaper_api(|api| {
        connected_monitors(api)?
            .iter()
            .map(|monitor| wallpaper_of(api, monitor))
            .collect()
    })
}
