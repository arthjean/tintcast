use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::error::ThemeError;

pub const OMARCHY_KEYS: [&str; 26] = [
    "mode",
    "accent",
    "selection",
    "muted",
    "background",
    "dark_background",
    "darker_background",
    "lighter_background",
    "foreground",
    "dark_foreground",
    "light_foreground",
    "bright_foreground",
    "red",
    "yellow",
    "orange",
    "green",
    "cyan",
    "blue",
    "magenta",
    "brown",
    "bright_red",
    "bright_yellow",
    "bright_green",
    "bright_cyan",
    "bright_blue",
    "bright_magenta",
];

pub const LEGACY_KEYS: [&str; 20] = [
    "color0",
    "color1",
    "color2",
    "color3",
    "color4",
    "color5",
    "color6",
    "color7",
    "color8",
    "color9",
    "color10",
    "color11",
    "color12",
    "color13",
    "color14",
    "color15",
    "purple",
    "bright_purple",
    "selection_background",
    "selection_foreground",
];

const NORMAL: [&str; 6] = ["red", "green", "yellow", "blue", "magenta", "cyan"];

pub const LUMINANCE_SPLIT: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn of_background(background: Rgba) -> Self {
        if background.relative_luminance() > LUMINANCE_SPLIT {
            Self::Light
        } else {
            Self::Dark
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub mode: Mode,
    pub background: Rgba,
    pub foreground: Rgba,
    pub accent: Rgba,
    pub selection: Rgba,
    pub selection_foreground: Option<Rgba>,
    pub muted: Rgba,
    pub ansi: [Rgba; 16],
    pub named: BTreeMap<String, Rgba>,
}

pub fn is_known_key(key: &str) -> bool {
    OMARCHY_KEYS.contains(&key) || LEGACY_KEYS.contains(&key)
}

impl Palette {
    pub fn new(background: Rgba, foreground: Rgba) -> Self {
        let named = BTreeMap::from([
            ("background".to_owned(), background),
            ("foreground".to_owned(), foreground),
        ]);
        Self::build(&named, background, foreground, None, false)
    }

    pub(crate) fn resolve(
        named: BTreeMap<String, Rgba>,
        mode: Option<Mode>,
        light_marker: bool,
        file: &str,
    ) -> Result<Self, ThemeError> {
        let (Some(&background), Some(&foreground)) =
            (named.get("background"), named.get("foreground"))
        else {
            return Err(ThemeError::NeedsBackgroundAndForeground {
                file: file.to_owned(),
            });
        };
        Ok(Self::build(
            &named,
            background,
            foreground,
            mode,
            light_marker,
        ))
    }

    fn build(
        named: &BTreeMap<String, Rgba>,
        background: Rgba,
        foreground: Rgba,
        mode: Option<Mode>,
        light_marker: bool,
    ) -> Self {
        let get = |key: &str| named.get(key).copied();
        let alias = |key: &str, legacy: &str| get(key).or_else(|| get(legacy));
        let normal = |key: &str| {
            if key == "magenta" {
                alias("magenta", "purple")
            } else {
                get(key)
            }
        };
        let bright = |key: &str| {
            if key == "magenta" {
                alias("bright_magenta", "bright_purple")
            } else {
                get(&format!("bright_{key}"))
            }
        };
        let muted = get("muted").unwrap_or_else(|| foreground.mix(background, 0.5));
        let mut ansi = [foreground; 16];
        ansi[0] = background;
        ansi[7] = foreground;
        ansi[8] = muted;
        ansi[15] = get("bright_foreground").unwrap_or(foreground);
        for (i, key) in NORMAL.iter().enumerate() {
            let color = normal(key).unwrap_or(foreground);
            ansi[1 + i] = color;
            ansi[9 + i] = bright(key).unwrap_or(color);
        }
        for (slot, value) in ansi.iter_mut().enumerate() {
            if let Some(color) = get(&format!("color{slot}")) {
                *value = color;
            }
        }
        let mode = mode.unwrap_or(if light_marker {
            Mode::Light
        } else {
            Mode::of_background(background)
        });
        Self {
            mode,
            background,
            foreground,
            accent: get("accent").unwrap_or(ansi[4]),
            selection: alias("selection", "selection_background")
                .unwrap_or_else(|| background.mix(foreground, 0.25)),
            selection_foreground: get("selection_foreground"),
            muted,
            ansi,
            named: named.clone(),
        }
    }

    pub fn with_accent(mut self, accent: Rgba) -> Self {
        self.accent = accent;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(pairs: &[(&str, &str)]) -> BTreeMap<String, Rgba> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Rgba::parse(v).unwrap()))
            .collect()
    }

    fn c(text: &str) -> Rgba {
        Rgba::parse(text).unwrap()
    }

    #[test]
    fn maps_terminal_slots_from_omarchy_keys() {
        let palette = Palette::resolve(
            named(&[
                ("background", "#000001"),
                ("foreground", "#000007"),
                ("muted", "#000008"),
                ("red", "#000011"),
                ("green", "#000012"),
                ("yellow", "#000013"),
                ("blue", "#000014"),
                ("magenta", "#000015"),
                ("cyan", "#000016"),
                ("bright_red", "#000021"),
                ("bright_blue", "#000024"),
                ("bright_foreground", "#00000f"),
            ]),
            None,
            false,
            "colors.toml",
        )
        .unwrap();
        let expected = [
            "#000001", "#000011", "#000012", "#000013", "#000014", "#000015", "#000016", "#000007",
            "#000008", "#000021", "#000012", "#000013", "#000024", "#000015", "#000016", "#00000f",
        ];
        assert_eq!(palette.ansi.map(Rgba::hex), expected.map(str::to_owned));
    }

    #[test]
    fn bright_foreground_falls_back_to_foreground_and_legacy_slots_override() {
        let palette = Palette::resolve(
            named(&[
                ("background", "#101010"),
                ("foreground", "#e0e0e0"),
                ("red", "#aa0000"),
                ("color1", "#bb0000"),
                ("color15", "#ffffff"),
                ("purple", "#880088"),
            ]),
            None,
            false,
            "colors.toml",
        )
        .unwrap();
        assert_eq!(palette.ansi[1], c("#bb0000"));
        assert_eq!(palette.ansi[15], c("#ffffff"));
        assert_eq!(palette.ansi[5], c("#880088"));
        assert_eq!(palette.ansi[13], c("#880088"));
        let plain = Palette::new(c("#101010"), c("#e0e0e0"));
        assert_eq!(plain.ansi[15], c("#e0e0e0"));
    }

    #[test]
    fn mode_comes_from_the_key_then_the_marker_then_luminance() {
        let dark_bg = named(&[("background", "#202020"), ("foreground", "#eeeeee")]);
        let light_bg = named(&[("background", "#f0f0f0"), ("foreground", "#111111")]);
        let resolve = |n: &BTreeMap<String, Rgba>, mode, marker| {
            Palette::resolve(n.clone(), mode, marker, "colors.toml")
                .unwrap()
                .mode
        };
        assert_eq!(resolve(&dark_bg, Some(Mode::Light), false), Mode::Light);
        assert_eq!(resolve(&dark_bg, None, true), Mode::Light);
        assert_eq!(resolve(&dark_bg, None, false), Mode::Dark);
        assert_eq!(resolve(&light_bg, None, false), Mode::Light);
        assert_eq!(resolve(&light_bg, Some(Mode::Dark), true), Mode::Dark);
    }

    #[test]
    fn accent_falls_back_to_blue() {
        let palette = Palette::resolve(
            named(&[
                ("background", "#000000"),
                ("foreground", "#ffffff"),
                ("blue", "#3366ff"),
            ]),
            None,
            false,
            "colors.toml",
        )
        .unwrap();
        assert_eq!(palette.accent, c("#3366ff"));
    }

    #[test]
    fn background_and_foreground_are_required() {
        let error = Palette::resolve(
            named(&[("background", "#000000")]),
            None,
            false,
            "colors.toml",
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "colors.toml needs background and foreground."
        );
    }
}
