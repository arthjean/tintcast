use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tintcast_engine::{
    ApplyRequest, Detection, Integration, Mode, Operation, Plan, RegistryValue, ReloadResult, Rgba,
    SystemServices, Verification,
};

use crate::desktop::Desktop;
use crate::keys::{ACCENT, DWM, PERSONALIZE};

pub const ID: &str = "appearance";
pub const CONTRAST_SKIP: &str = "A contrast theme is on, so Tintcast leaves mode and accent alone.";
const SHADES: [f32; 8] = [0.75, 0.5, 0.25, 0.0, -0.25, -0.5, -0.75, -0.9];
const VERIFY_ATTEMPTS: usize = 20;
const VERIFY_PAUSE: Duration = Duration::from_millis(50);

pub fn shade(color: Rgba, amount: f32) -> Rgba {
    let toward = if amount >= 0.0 { 255.0 } else { 0.0 };
    let amount = amount.abs();
    let mix = |c: u8| {
        let value = f32::from(c) + (toward - f32::from(c)) * amount;
        value.round().clamp(0.0, 255.0) as u8
    };
    Rgba::rgb(mix(color.r), mix(color.g), mix(color.b))
}

pub fn abgr(color: Rgba) -> u32 {
    0xff00_0000 | (u32::from(color.b) << 16) | (u32::from(color.g) << 8) | u32::from(color.r)
}

pub fn from_abgr(value: u32) -> Rgba {
    let [r, g, b, _] = value.to_le_bytes();
    Rgba::rgb(r, g, b)
}

pub fn colorization(color: Rgba) -> u32 {
    0xc400_0000 | (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

pub fn accent_palette(accent: Rgba) -> [Rgba; 8] {
    SHADES.map(|amount| shade(accent, amount))
}

pub fn palette_bytes(accent: Rgba) -> Vec<u8> {
    accent_palette(accent)
        .iter()
        .flat_map(|c| [c.r, c.g, c.b, 0])
        .collect()
}

pub fn targets(mode: Mode, accent: Rgba) -> Vec<(&'static str, &'static str, RegistryValue)> {
    let light = RegistryValue::Dword(u32::from(mode == Mode::Light));
    let palette = accent_palette(accent);
    vec![
        (PERSONALIZE, "AppsUseLightTheme", light.clone()),
        (PERSONALIZE, "SystemUsesLightTheme", light),
        (
            ACCENT,
            "AccentPalette",
            RegistryValue::Binary(palette_bytes(accent)),
        ),
        (
            ACCENT,
            "AccentColorMenu",
            RegistryValue::Dword(abgr(accent)),
        ),
        (
            ACCENT,
            "StartColorMenu",
            RegistryValue::Dword(abgr(palette[4])),
        ),
        (DWM, "AccentColor", RegistryValue::Dword(abgr(accent))),
        (
            DWM,
            "ColorizationColor",
            RegistryValue::Dword(colorization(accent)),
        ),
        (
            DWM,
            "ColorizationAfterglow",
            RegistryValue::Dword(colorization(accent)),
        ),
    ]
}

fn planned_accent(operations: &[Operation]) -> Option<Rgba> {
    operations.iter().find_map(|op| match op {
        Operation::SetRegistryValue {
            key,
            name,
            value: Some(RegistryValue::Dword(value)),
        } if key == DWM && name == "AccentColor" => Some(from_abgr(*value)),
        _ => None,
    })
}

pub struct Appearance {
    desktop: Arc<dyn Desktop>,
}

impl Appearance {
    pub fn new(desktop: Arc<dyn Desktop>) -> Self {
        Self { desktop }
    }
}

impl Integration for Appearance {
    fn id(&self) -> &str {
        ID
    }

    fn name(&self) -> &str {
        "Mode and accent"
    }

    fn detect(&self, _services: &dyn SystemServices) -> Detection {
        Detection::installed("Windows personalization")
    }

    fn plan(
        &self,
        request: &ApplyRequest,
        _detection: &Detection,
        services: &dyn SystemServices,
    ) -> Result<Plan, String> {
        if self.desktop.contrast_theme_on() {
            return Ok(Plan::skip(CONTRAST_SKIP));
        }
        let palette = &request.theme.palette;
        let mut operations = Vec::new();
        for (key, name, value) in targets(palette.mode, palette.accent) {
            let current = services
                .read_registry(key, name)
                .map_err(|e| e.to_string())?;
            if current.as_ref() != Some(&value) {
                operations.push(Operation::SetRegistryValue {
                    key: key.to_owned(),
                    name: name.to_owned(),
                    value: Some(value),
                });
            }
        }
        Ok(Plan::changes(operations))
    }

    fn reload(&self, services: &dyn SystemServices) -> ReloadResult {
        match services.broadcast_color_change() {
            Ok(()) => ReloadResult::Reloaded,
            Err(error) => ReloadResult::Failed(error.to_string()),
        }
    }

    fn verify(&self, operations: &[Operation], _services: &dyn SystemServices) -> Verification {
        let Some(expected) = planned_accent(operations) else {
            return Verification::Matches;
        };
        let mut last = Err(String::new());
        for attempt in 0..VERIFY_ATTEMPTS {
            if attempt > 0 {
                thread::sleep(VERIFY_PAUSE);
            }
            last = self.desktop.accent_color();
            if last.as_ref() == Ok(&expected) {
                return Verification::Matches;
            }
        }
        Verification::Mismatch(match last {
            Ok(shown) => format!(
                "Windows shows the accent as {}, not {}",
                shown.hex(),
                expected.hex()
            ),
            Err(error) => format!("Tintcast could not read the accent back: {error}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_probe_palette() {
        let accent = Rgba::rgb(0xd9, 0x4f, 0x70);
        let palette = accent_palette(accent);
        assert_eq!(palette[3], accent);
        assert_eq!(palette[0], Rgba::rgb(0xf6, 0xd3, 0xdb));
        assert_eq!(palette[7], Rgba::rgb(0x16, 0x08, 0x0b));
        assert_eq!(palette_bytes(accent).len(), 32);
        assert_eq!(&palette_bytes(accent)[12..16], &[0xd9, 0x4f, 0x70, 0]);
    }

    #[test]
    fn encodes_accent_values_as_windows_stores_them() {
        let accent = Rgba::rgb(0x00, 0x78, 0xd4);
        assert_eq!(abgr(accent), 4_292_114_432);
        assert_eq!(from_abgr(abgr(accent)), accent);
        assert_eq!(colorization(accent), 0xc400_78d4);
    }

    #[test]
    fn never_touches_color_prevalence() {
        for (_, name, _) in targets(Mode::Dark, Rgba::rgb(1, 2, 3)) {
            assert_ne!(name, "ColorPrevalence");
        }
    }
}
