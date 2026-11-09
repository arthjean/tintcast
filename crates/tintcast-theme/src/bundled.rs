use crate::error::ThemeError;
use crate::source::{COLORS_FILE, ParsedTheme, ThemeFiles, parse};

pub struct BundledTheme {
    pub slug: &'static str,
    pub colors: &'static str,
    pub settings: &'static str,
}

pub const BUNDLED: [BundledTheme; 4] = [
    BundledTheme {
        slug: "daylight",
        colors: include_str!("../bundled/daylight/colors.toml"),
        settings: include_str!("../bundled/daylight/tintcast.toml"),
    },
    BundledTheme {
        slug: "inkwell",
        colors: include_str!("../bundled/inkwell/colors.toml"),
        settings: include_str!("../bundled/inkwell/tintcast.toml"),
    },
    BundledTheme {
        slug: "ironwood",
        colors: include_str!("../bundled/ironwood/colors.toml"),
        settings: include_str!("../bundled/ironwood/tintcast.toml"),
    },
    BundledTheme {
        slug: "linen",
        colors: include_str!("../bundled/linen/colors.toml"),
        settings: include_str!("../bundled/linen/tintcast.toml"),
    },
];

pub fn is_bundled(slug: &str) -> bool {
    BUNDLED.iter().any(|theme| theme.slug == slug)
}

impl BundledTheme {
    pub fn parse(&self) -> Result<ParsedTheme, ThemeError> {
        parse(
            self.slug,
            &ThemeFiles {
                colors_name: COLORS_FILE,
                colors: self.colors.as_bytes().to_vec(),
                settings: Some(self.settings.as_bytes().to_vec()),
                light_marker: false,
            },
        )
    }
}
