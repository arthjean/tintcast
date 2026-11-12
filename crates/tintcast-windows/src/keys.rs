pub const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
pub const ACCENT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
pub const DWM: &str = r"Software\Microsoft\Windows\DWM";
pub const COLORS: &str = r"Control Panel\Colors";
pub const BACKGROUND: &str = "Background";
pub const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Tintcast";
pub const WALLPAPERS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Wallpapers";
pub const POLICY_ACTIVE_DESKTOP: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop";
pub const NO_CHANGING_WALLPAPER: &str = "NoChangingWallPaper";

pub const WATCHED: [(&str, &str); 11] = [
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
    (COLORS, BACKGROUND),
];
