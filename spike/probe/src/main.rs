use std::{
    collections::BTreeMap,
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    ptr::null_mut,
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use windows::{
    UI::ViewManagement::{UIColorType, UISettings},
    Win32::{
        Foundation::{LPARAM, WPARAM},
        Graphics::Dwm::DwmGetColorizationColor,
        System::{
            Com::{
                CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree,
            },
            LibraryLoader::{GetProcAddress, LoadLibraryW},
        },
        UI::{
            Shell::{DESKTOP_WALLPAPER_POSITION, DWPOS_FILL, DesktopWallpaper, IDesktopWallpaper},
            WindowsAndMessaging::{
                HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
            },
        },
    },
};
use windows_core::{BOOL, BSTR, GUID, HRESULT, HSTRING, PCWSTR, PWSTR, s, w};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE, Type};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const ACCENT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
const DWM: &str = r"Software\Microsoft\Windows\DWM";

const WATCHED: &[(&str, &str)] = &[
    (PERSONALIZE, "AppsUseLightTheme"),
    (PERSONALIZE, "SystemUsesLightTheme"),
    (PERSONALIZE, "ColorPrevalence"),
    (ACCENT, "AccentColorMenu"),
    (ACCENT, "StartColorMenu"),
    (ACCENT, "AccentPalette"),
    (DWM, "AccentColor"),
    (DWM, "ColorizationColor"),
    (DWM, "ColorizationAfterglow"),
    (DWM, "ColorPrevalence"),
];

const CLSID_THEME_MANAGER2: GUID = GUID::from_u128(0x9324da94_50ec_4a14_a770_e90ca03e7c8f);
const PROBE_THEME_NAME: &str = "Tintcast Probe";
const APPLY_IGNORE_BACKGROUND: u32 = 1 << 0;
const APPLY_IGNORE_CURSOR: u32 = 1 << 1;
const APPLY_IGNORE_DESKTOP_ICONS: u32 = 1 << 2;
const APPLY_IGNORE_SOUND: u32 = 1 << 4;
const APPLY_NO_HOURGLASS: u32 = 1 << 8;
const APPLY_FLAGS: u32 = APPLY_IGNORE_BACKGROUND
    | APPLY_IGNORE_CURSOR
    | APPLY_IGNORE_DESKTOP_ICONS
    | APPLY_IGNORE_SOUND
    | APPLY_NO_HOURGLASS;

mod com {
    #![allow(non_snake_case)]

    use std::ffi::c_void;

    use windows_core::{BSTR, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, PWSTR, interface};

    #[interface("26e4185f-0528-475f-acaf-abe89ba6017d")]
    pub unsafe trait ITheme: IUnknown {
        pub unsafe fn get_DisplayName(&self, name: *mut BSTR) -> HRESULT;
    }

    #[interface("c1e8c83e-845d-4d95-81db-e283fdffc000")]
    pub unsafe trait IThemeManager2: IUnknown {
        pub unsafe fn Init(&self, flags: u32) -> HRESULT;
        pub unsafe fn InitAsync(&self, hwnd: *mut c_void, unknown: i32) -> HRESULT;
        pub unsafe fn Refresh(&self) -> HRESULT;
        pub unsafe fn RefreshAsync(&self, hwnd: *mut c_void, unknown: i32) -> HRESULT;
        pub unsafe fn RefreshComplete(&self) -> HRESULT;
        pub unsafe fn GetThemeCount(&self, count: *mut i32) -> HRESULT;
        pub unsafe fn GetTheme(&self, index: i32, theme: *mut Option<ITheme>) -> HRESULT;
        pub unsafe fn IsThemeDisabled(&self, index: i32, disabled: *mut i32) -> HRESULT;
        pub unsafe fn GetCurrentTheme(&self, index: *mut i32) -> HRESULT;
        pub unsafe fn SetCurrentTheme(
            &self,
            parent: *mut c_void,
            index: i32,
            apply_now: i32,
            apply_flags: u32,
            pack_flags: u32,
        ) -> HRESULT;
        pub unsafe fn GetCustomTheme(&self, index: *mut i32) -> HRESULT;
        pub unsafe fn GetDefaultTheme(&self, index: *mut i32) -> HRESULT;
        pub unsafe fn CreateThemePack(
            &self,
            hwnd: *mut c_void,
            path: PCWSTR,
            pack_flags: u32,
        ) -> HRESULT;
        pub unsafe fn CloneAndSetCurrentTheme(
            &self,
            hwnd: *mut c_void,
            path: PCWSTR,
            cloned: *mut PWSTR,
        ) -> HRESULT;
        pub unsafe fn InstallThemePack(
            &self,
            hwnd: *mut c_void,
            path: PCWSTR,
            unknown: i32,
            pack_flags: u32,
            name: *mut PWSTR,
            theme: *mut Option<ITheme>,
        ) -> HRESULT;
        pub unsafe fn DeleteTheme(&self, path: PCWSTR) -> HRESULT;
        pub unsafe fn OpenTheme(&self, hwnd: *mut c_void, path: PCWSTR, pack_flags: u32)
        -> HRESULT;
        pub unsafe fn AddAndSelectTheme(
            &self,
            hwnd: *mut c_void,
            path: PCWSTR,
            apply_flags: u32,
            pack_flags: u32,
        ) -> HRESULT;
    }
}

use com::IThemeManager2;

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ImmersiveColorPreference {
    start: u32,
    accent: u32,
}

type GetUserColorPreference =
    unsafe extern "system" fn(*mut ImmersiveColorPreference, BOOL) -> HRESULT;
type SetUserColorPreference =
    unsafe extern "system" fn(*const ImmersiveColorPreference, BOOL) -> HRESULT;
type RawProc = unsafe extern "system" fn() -> isize;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
enum RegValue {
    U32(u32),
    Bytes(String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Monitor {
    id: String,
    path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct WallpaperState {
    position: Option<i32>,
    monitors: Vec<Monitor>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct UiColors {
    accent: String,
    background: String,
    foreground: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct UxTheme {
    get_export: bool,
    set_export: bool,
    start: Option<String>,
    accent: Option<String>,
    start_raw: Option<u32>,
    accent_raw: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ThemeList {
    count: i32,
    current: i32,
    custom: i32,
    current_name: Option<String>,
    names: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TerminalInfo {
    settings_found: Vec<String>,
    fragments_dir_exists: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Report {
    build: String,
    registry: BTreeMap<String, Option<RegValue>>,
    wallpaper: Option<WallpaperState>,
    wallpaper_error: Option<String>,
    ui_settings: Option<UiColors>,
    ui_settings_error: Option<String>,
    dwm_colorization: Option<String>,
    uxtheme: UxTheme,
    themes: Option<ThemeList>,
    themes_error: Option<String>,
    terminal: TerminalInfo,
}

fn main() -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let started = Instant::now();
    match args.as_slice() {
        ["report"] => println!("{}", serde_json::to_string_pretty(&report())?),
        ["report", out] => fs::write(out, serde_json::to_string_pretty(&report())?)?,
        ["image", out, top, bottom] => make_image(Path::new(out), top, bottom)?,
        ["wallpaper-set", image] => wallpaper_set(Path::new(image))?,
        ["wallpaper-restore", state] => wallpaper_restore(&load(state)?)?,
        ["appearance-set", mode] => appearance_set(mode)?,
        ["registry-restore", state] => registry_restore(&load(state)?)?,
        ["accent-registry", hex] => accent_registry(hex)?,
        ["accent-uxtheme", hex] => accent_uxtheme(hex)?,
        ["accent-uxtheme-restore", state] => accent_uxtheme_restore(&load(state)?)?,
        ["theme-apply", hex, mode] => theme_apply(hex, mode)?,
        ["theme-restore", state] => theme_restore(&load(state)?)?,
        ["hotkeys"] => hotkeys(),
        _ => bail!(
            "usage: tintcast-probe report [out.json] | image <out.bmp> <#top> <#bottom> | wallpaper-set <image> | \
             wallpaper-restore <state.json> | appearance-set dark|light | registry-restore <state.json> | \
             accent-registry <#rrggbb> | accent-uxtheme <#rrggbb> | accent-uxtheme-restore <state.json> | \
             theme-apply <#rrggbb> dark|light | theme-restore <state.json> | hotkeys"
        ),
    }
    if args.first().is_some_and(|a| *a != "report") {
        println!("{}: {} ms", args.join(" "), started.elapsed().as_millis());
    }
    Ok(())
}

fn load(path: &str) -> Result<Report> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}

fn report() -> Report {
    let registry = WATCHED
        .iter()
        .map(|(key, name)| (format!(r"{key}\{name}"), read_reg(key, name)))
        .collect();
    let (wallpaper, wallpaper_error) = split(read_wallpaper());
    let (ui_settings, ui_settings_error) = split(ui_colors());
    let (themes, themes_error) = split(theme_manager().and_then(|tm| theme_list(&tm)));
    Report {
        build: windows_build(),
        registry,
        wallpaper,
        wallpaper_error,
        ui_settings,
        ui_settings_error,
        dwm_colorization: dwm_colorization(),
        uxtheme: uxtheme_state(),
        themes,
        themes_error,
        terminal: terminal_info(),
    }
}

fn split<T>(result: Result<T>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    }
}

fn windows_build() -> String {
    let Ok(key) = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
        return "unknown".into();
    };
    let build = key.get_string("CurrentBuild").unwrap_or_default();
    let ubr = key
        .get_u32("UBR")
        .map(|n| n.to_string())
        .unwrap_or_default();
    let version = key.get_string("DisplayVersion").unwrap_or_default();
    format!("{build}.{ubr} ({version})")
}

fn read_reg(key: &str, name: &str) -> Option<RegValue> {
    let value = CURRENT_USER.open(key).ok()?.get_value(name).ok()?;
    let bytes: &[u8] = &value;
    if matches!(value.ty(), Type::U32) {
        let raw: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
        Some(RegValue::U32(u32::from_le_bytes(raw)))
    } else {
        Some(RegValue::Bytes(to_hex(bytes)))
    }
}

fn write_reg(key: &str, name: &str, value: Option<&RegValue>) -> Result<()> {
    let key = CURRENT_USER.create(key)?;
    match value {
        Some(RegValue::U32(n)) => key.set_u32(name, *n)?,
        Some(RegValue::Bytes(hex)) => key.set_bytes(name, Type::Bytes, &from_hex(hex)?)?,
        None => {
            let _ = key.remove_value(name);
        }
    }
    Ok(())
}

fn broadcast(area: &str) {
    let area = HSTRING::from(area);
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(area.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }
}

fn wallpaper_api() -> Result<IDesktopWallpaper> {
    Ok(unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? })
}

fn take_pwstr(text: PWSTR) -> Option<String> {
    if text.is_null() {
        return None;
    }
    let value = unsafe { text.to_string().ok() };
    unsafe { CoTaskMemFree(Some(text.0 as *const c_void)) };
    value
}

fn read_wallpaper() -> Result<WallpaperState> {
    let api = wallpaper_api()?;
    let count = unsafe { api.GetMonitorDevicePathCount()? };
    let mut monitors = Vec::new();
    for index in 0..count {
        let id = take_pwstr(unsafe { api.GetMonitorDevicePathAt(index)? }).unwrap_or_default();
        let path = unsafe { api.GetWallpaper(&HSTRING::from(id.as_str())) }
            .ok()
            .and_then(take_pwstr);
        monitors.push(Monitor { id, path });
    }
    let position = unsafe { api.GetPosition() }.ok().map(|p| p.0);
    Ok(WallpaperState { position, monitors })
}

fn wallpaper_set(image: &Path) -> Result<()> {
    let image = std::path::absolute(image)?;
    let image = HSTRING::from(image.to_string_lossy().as_ref());
    let api = wallpaper_api()?;
    unsafe { api.SetPosition(DWPOS_FILL)? };
    for monitor in read_wallpaper()?.monitors {
        unsafe { api.SetWallpaper(&HSTRING::from(monitor.id.as_str()), &image)? };
    }
    Ok(())
}

fn wallpaper_restore(state: &Report) -> Result<()> {
    let saved = state
        .wallpaper
        .clone()
        .ok_or_else(|| anyhow!("the state has no wallpaper"))?;
    let api = wallpaper_api()?;
    if let Some(position) = saved.position {
        unsafe { api.SetPosition(DESKTOP_WALLPAPER_POSITION(position))? };
    }
    for monitor in saved.monitors {
        let path = monitor.path.unwrap_or_default();
        let result = unsafe {
            api.SetWallpaper(
                &HSTRING::from(monitor.id.as_str()),
                &HSTRING::from(path.as_str()),
            )
        };
        println!("restore {} -> {path:?}: {result:?}", monitor.id);
    }
    Ok(())
}

fn ui_colors() -> Result<UiColors> {
    let settings = UISettings::new()?;
    let color = |kind| -> Result<String> {
        let c = settings.GetColorValue(kind)?;
        Ok(format!("#{:02x}{:02x}{:02x}", c.R, c.G, c.B))
    };
    Ok(UiColors {
        accent: color(UIColorType::Accent)?,
        background: color(UIColorType::Background)?,
        foreground: color(UIColorType::Foreground)?,
    })
}

fn dwm_colorization() -> Option<String> {
    let mut color = 0u32;
    let mut opaque = BOOL::default();
    unsafe { DwmGetColorizationColor(&mut color, &mut opaque) }.ok()?;
    Some(format!("0x{color:08x}"))
}

fn uxtheme_functions() -> (
    Option<GetUserColorPreference>,
    Option<SetUserColorPreference>,
) {
    let Ok(module) = (unsafe { LoadLibraryW(w!("uxtheme.dll")) }) else {
        return (None, None);
    };
    let get = unsafe { GetProcAddress(module, s!("GetUserColorPreference")) }
        .map(|f| unsafe { std::mem::transmute::<RawProc, GetUserColorPreference>(f) });
    let set = unsafe { GetProcAddress(module, s!("SetUserColorPreference")) }
        .map(|f| unsafe { std::mem::transmute::<RawProc, SetUserColorPreference>(f) });
    (get, set)
}

fn uxtheme_state() -> UxTheme {
    let (get, set) = uxtheme_functions();
    let mut state = UxTheme {
        get_export: get.is_some(),
        set_export: set.is_some(),
        ..UxTheme::default()
    };
    if let Some(get) = get {
        let mut pref = ImmersiveColorPreference::default();
        if unsafe { get(&mut pref, BOOL(0)) }.is_ok() {
            state.start = Some(colorref_hex(pref.start));
            state.accent = Some(colorref_hex(pref.accent));
            state.start_raw = Some(pref.start);
            state.accent_raw = Some(pref.accent);
        }
    }
    state
}

fn accent_uxtheme(hex: &str) -> Result<()> {
    let (_, Some(set)) = uxtheme_functions() else {
        bail!("uxtheme has no SetUserColorPreference export")
    };
    let accent = parse_hex(hex)?;
    let pref = ImmersiveColorPreference {
        start: colorref(shade(accent, -0.5)),
        accent: colorref(accent),
    };
    unsafe { set(&pref, BOOL(1)) }.ok()?;
    Ok(())
}

fn accent_uxtheme_restore(state: &Report) -> Result<()> {
    let (_, Some(set)) = uxtheme_functions() else {
        bail!("uxtheme has no SetUserColorPreference export")
    };
    let (Some(start), Some(accent)) = (state.uxtheme.start_raw, state.uxtheme.accent_raw) else {
        bail!("the state has no uxtheme colors")
    };
    unsafe { set(&ImmersiveColorPreference { start, accent }, BOOL(1)) }.ok()?;
    Ok(())
}

fn appearance_set(mode: &str) -> Result<()> {
    let light = match mode {
        "light" => 1,
        "dark" => 0,
        _ => bail!("mode must be dark or light"),
    };
    write_reg(
        PERSONALIZE,
        "AppsUseLightTheme",
        Some(&RegValue::U32(light)),
    )?;
    write_reg(
        PERSONALIZE,
        "SystemUsesLightTheme",
        Some(&RegValue::U32(light)),
    )?;
    broadcast("ImmersiveColorSet");
    Ok(())
}

fn registry_restore(state: &Report) -> Result<()> {
    for (key, name) in WATCHED {
        let saved = state
            .registry
            .get(&format!(r"{key}\{name}"))
            .cloned()
            .flatten();
        if read_reg(key, name) != saved {
            write_reg(key, name, saved.as_ref())?;
            println!(r"restored {key}\{name}");
        }
    }
    broadcast("ImmersiveColorSet");
    Ok(())
}

fn accent_registry(hex: &str) -> Result<()> {
    let accent = parse_hex(hex)?;
    let palette =
        [0.75, 0.5, 0.25, 0.0, -0.25, -0.5, -0.75, -0.9].map(|amount| shade(accent, amount));
    let bytes: Vec<u8> = palette
        .iter()
        .flat_map(|(r, g, b)| [*r, *g, *b, 0])
        .collect();
    let (r, g, b) = (
        u32::from(accent.0),
        u32::from(accent.1),
        u32::from(accent.2),
    );
    write_reg(
        ACCENT,
        "AccentPalette",
        Some(&RegValue::Bytes(to_hex(&bytes))),
    )?;
    write_reg(
        ACCENT,
        "AccentColorMenu",
        Some(&RegValue::U32(abgr(accent))),
    )?;
    write_reg(
        ACCENT,
        "StartColorMenu",
        Some(&RegValue::U32(abgr(palette[4]))),
    )?;
    write_reg(DWM, "AccentColor", Some(&RegValue::U32(abgr(accent))))?;
    write_reg(
        DWM,
        "ColorizationColor",
        Some(&RegValue::U32(0xc400_0000 | (r << 16) | (g << 8) | b)),
    )?;
    write_reg(
        DWM,
        "ColorizationAfterglow",
        Some(&RegValue::U32(0xc400_0000 | (r << 16) | (g << 8) | b)),
    )?;
    broadcast("ImmersiveColorSet");
    Ok(())
}

fn theme_manager() -> Result<IThemeManager2> {
    let manager: IThemeManager2 =
        unsafe { CoCreateInstance(&CLSID_THEME_MANAGER2, None, CLSCTX_ALL)? };
    unsafe { manager.Init(0) }.ok()?;
    Ok(manager)
}

fn theme_name(manager: &IThemeManager2, index: i32) -> Result<String> {
    let mut theme = None;
    unsafe { manager.GetTheme(index, &mut theme) }.ok()?;
    let theme = theme.ok_or_else(|| anyhow!("no theme at {index}"))?;
    let mut name = BSTR::new();
    unsafe { theme.get_DisplayName(&mut name) }.ok()?;
    Ok(name.to_string())
}

fn theme_list(manager: &IThemeManager2) -> Result<ThemeList> {
    let (mut count, mut current, mut custom) = (0, -1, -1);
    unsafe { manager.GetThemeCount(&mut count) }.ok()?;
    unsafe { manager.GetCurrentTheme(&mut current) }.ok()?;
    unsafe { manager.GetCustomTheme(&mut custom) }.ok()?;
    let names = (0..count)
        .map(|i| theme_name(manager, i).unwrap_or_else(|e| format!("<{e}>")))
        .collect();
    let current_name = theme_name(manager, current).ok();
    Ok(ThemeList {
        count,
        current,
        custom,
        current_name,
        names,
    })
}

fn probe_theme_path() -> Result<PathBuf> {
    let base = std::env::var("LOCALAPPDATA")?;
    let dir = Path::new(&base).join(r"Microsoft\Windows\Themes");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("TintcastProbe.theme"))
}

fn theme_apply(hex: &str, mode: &str) -> Result<()> {
    let (r, g, b) = parse_hex(hex)?;
    let mode = match mode {
        "dark" => "Dark",
        "light" => "Light",
        _ => bail!("mode must be dark or light"),
    };
    let text = [
        "[Theme]".to_string(),
        format!("DisplayName={PROBE_THEME_NAME}"),
        "SetLogonBackground=0".into(),
        String::new(),
        r"[Control Panel\Desktop]".into(),
        "TileWallpaper=0".into(),
        "WallpaperStyle=10".into(),
        "Pattern=".into(),
        String::new(),
        "[VisualStyles]".into(),
        r"Path=%ResourceDir%\Themes\Aero\Aero.msstyles".into(),
        "ColorStyle=NormalColor".into(),
        "Size=NormalSize".into(),
        "AutoColorization=0".into(),
        format!("ColorizationColor=0XC4{r:02X}{g:02X}{b:02X}"),
        format!("SystemMode={mode}"),
        format!("AppMode={mode}"),
        String::new(),
        "[boot]".into(),
        "SCRNSAVE.EXE=".into(),
        String::new(),
        "[MasterThemeSelector]".into(),
        "MTSM=RJSPBS".into(),
        String::new(),
    ]
    .join("\r\n");
    let path = probe_theme_path()?;
    fs::write(&path, text)?;
    let manager = theme_manager()?;
    let wide = HSTRING::from(path.to_string_lossy().as_ref());
    let added =
        unsafe { manager.AddAndSelectTheme(null_mut(), PCWSTR(wide.as_ptr()), APPLY_FLAGS, 0) };
    println!("AddAndSelectTheme: {added:?}");
    if added.is_ok() {
        return Ok(());
    }
    unsafe { manager.Refresh() }.ok()?;
    let index = find_theme(&manager, PROBE_THEME_NAME)?;
    let set = unsafe { manager.SetCurrentTheme(null_mut(), index, 1, APPLY_FLAGS, 0) };
    println!("SetCurrentTheme({index}): {set:?}");
    Ok(set.ok()?)
}

fn find_theme(manager: &IThemeManager2, name: &str) -> Result<i32> {
    let list = theme_list(manager)?;
    let index = list
        .names
        .iter()
        .position(|n| n == name)
        .ok_or_else(|| anyhow!("no theme named {name}"))?;
    Ok(i32::try_from(index)?)
}

fn theme_restore(state: &Report) -> Result<()> {
    let name = state
        .themes
        .as_ref()
        .and_then(|t| t.current_name.clone())
        .ok_or_else(|| anyhow!("the state has no current theme"))?;
    let manager = theme_manager()?;
    unsafe { manager.Refresh() }.ok()?;
    let index = find_theme(&manager, &name)?;
    let set = unsafe { manager.SetCurrentTheme(null_mut(), index, 1, APPLY_FLAGS, 0) };
    println!("SetCurrentTheme({index}, {name}): {set:?}");
    if let Ok(path) = probe_theme_path() {
        let _ = fs::remove_file(path);
    }
    Ok(set.ok()?)
}

fn terminal_info() -> TerminalInfo {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let base = Path::new(&base);
    let candidates = [
        r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json",
        r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json",
        r"Microsoft\Windows Terminal\settings.json",
    ];
    TerminalInfo {
        settings_found: candidates
            .iter()
            .map(|c| base.join(c))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        fragments_dir_exists: base.join(r"Microsoft\Windows Terminal\Fragments").exists(),
    }
}

fn make_image(out: &Path, top: &str, bottom: &str) -> Result<()> {
    let (width, height) = (1920u32, 1080u32);
    let (top, bottom) = (parse_hex(top)?, parse_hex(bottom)?);
    let row = width * 3;
    let size = 54 + row * height;
    let mut data = Vec::with_capacity(size as usize);
    data.extend(b"BM");
    data.extend(size.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    data.extend(54u32.to_le_bytes());
    data.extend(40u32.to_le_bytes());
    data.extend(width.to_le_bytes());
    data.extend(height.to_le_bytes());
    data.extend(1u16.to_le_bytes());
    data.extend(24u16.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    data.extend((row * height).to_le_bytes());
    data.extend(2835u32.to_le_bytes());
    data.extend(2835u32.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    data.extend(0u32.to_le_bytes());
    for y in 0..height {
        let t = y as f32 / (height - 1) as f32;
        let mix = |a: u8, b: u8| (f32::from(a) * t + f32::from(b) * (1.0 - t)).round() as u8;
        let pixel = [
            mix(top.2, bottom.2),
            mix(top.1, bottom.1),
            mix(top.0, bottom.0),
        ];
        for _ in 0..width {
            data.extend(pixel);
        }
    }
    fs::write(out, data)?;
    Ok(())
}

fn parse_hex(hex: &str) -> Result<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        bail!("expected #rrggbb, got {hex}");
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
    Ok((channel(0)?, channel(2)?, channel(4)?))
}

fn shade((r, g, b): (u8, u8, u8), amount: f32) -> (u8, u8, u8) {
    let toward = if amount >= 0.0 { 255.0 } else { 0.0 };
    let amount = amount.abs();
    let mix = |c: u8| (f32::from(c) + (toward - f32::from(c)) * amount).round() as u8;
    (mix(r), mix(g), mix(b))
}

fn abgr((r, g, b): (u8, u8, u8)) -> u32 {
    0xff00_0000 | (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r)
}

fn colorref((r, g, b): (u8, u8, u8)) -> u32 {
    (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r)
}

fn colorref_hex(c: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        c & 0xff,
        (c >> 8) & 0xff,
        (c >> 16) & 0xff
    )
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(hex: &str) -> Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        bail!("odd hex string");
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&hex[i..i + 2], 16)?))
        .collect()
}

fn hotkeys() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
        UnregisterHotKey,
    };
    let space = 0x20;
    let t = 0x54;
    let b = 0x42;
    let n = 0x4e;
    let combos: &[(&str, HOT_KEY_MODIFIERS, u32)] = &[
        (
            "Win+Ctrl+Shift+Space",
            MOD_WIN | MOD_CONTROL | MOD_SHIFT,
            space,
        ),
        ("Win+Ctrl+Alt+Space", MOD_WIN | MOD_CONTROL | MOD_ALT, space),
        ("Win+Alt+Space", MOD_WIN | MOD_ALT, space),
        ("Win+Shift+Space", MOD_WIN | MOD_SHIFT, space),
        ("Ctrl+Alt+Space", MOD_CONTROL | MOD_ALT, space),
        (
            "Ctrl+Shift+Alt+Space",
            MOD_CONTROL | MOD_SHIFT | MOD_ALT,
            space,
        ),
        ("Win+Ctrl+Shift+T", MOD_WIN | MOD_CONTROL | MOD_SHIFT, t),
        ("Win+Alt+T", MOD_WIN | MOD_ALT, t),
        ("Win+Ctrl+Space", MOD_WIN | MOD_CONTROL, space),
        (
            "Win+Ctrl+Alt+Shift+Space",
            MOD_WIN | MOD_CONTROL | MOD_ALT | MOD_SHIFT,
            space,
        ),
        ("Win+Ctrl+Alt+B", MOD_WIN | MOD_CONTROL | MOD_ALT, b),
        ("Win+Ctrl+Alt+N", MOD_WIN | MOD_CONTROL | MOD_ALT, n),
        ("Win+Alt+B", MOD_WIN | MOD_ALT, b),
    ];
    for (index, &(name, modifiers, key)) in combos.iter().enumerate() {
        let id = 0x7000 + i32::try_from(index).unwrap_or_default();
        match unsafe { RegisterHotKey(None, id, modifiers | MOD_NOREPEAT, key) } {
            Ok(()) => {
                let _ = unsafe { UnregisterHotKey(None, id) };
                println!("{name}: free");
            }
            Err(error) => println!("{name}: taken ({})", error.message()),
        }
    }
}
