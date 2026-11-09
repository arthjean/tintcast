use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("\"{0}\" is not a #rgb or #rrggbb color")]
pub struct ColorParseError(pub String);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub fn parse(text: &str) -> Result<Self, ColorParseError> {
        let error = || ColorParseError(text.to_owned());
        let digits = text.strip_prefix('#').ok_or_else(error)?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error());
        }
        let nibble = |i: usize| {
            digits
                .get(i..=i)
                .and_then(|d| u8::from_str_radix(d, 16).ok())
                .ok_or_else(error)
        };
        let byte = |i: usize| {
            digits
                .get(i..i + 2)
                .and_then(|d| u8::from_str_radix(d, 16).ok())
                .ok_or_else(error)
        };
        match digits.len() {
            3 => Ok(Self::rgb(nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17)),
            6 => Ok(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            _ => Err(error()),
        }
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn relative_luminance(self) -> f64 {
        fn channel(value: u8) -> f64 {
            let c = f64::from(value) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    pub fn contrast(self, other: Self) -> f64 {
        let a = self.relative_luminance();
        let b = other.relative_luminance();
        let (light, dark) = if a >= b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

    pub fn mix(self, other: Self, amount: f32) -> Self {
        let t = amount.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| {
            let value = f32::from(a) + (f32::from(b) - f32::from(a)) * t;
            value.round().clamp(0.0, 255.0) as u8
        };
        Self {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

impl FromStr for Rgba {
    type Err = ColorParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_and_long_hex_into_rgba() {
        assert_eq!(Rgba::parse("#1e1e2e").unwrap(), Rgba::rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(Rgba::parse("#FFF").unwrap(), Rgba::rgb(255, 255, 255));
        assert_eq!(Rgba::parse("#a0b").unwrap(), Rgba::rgb(0xaa, 0x00, 0xbb));
        assert_eq!(Rgba::parse("#a0b").unwrap().a, 255);
    }

    #[test]
    fn rejects_everything_else() {
        for text in [
            "",
            "#",
            "1e1e2e",
            "#1e1e2",
            "#1e1e2e80",
            "#ggg",
            "rgb(1,2,3)",
            "#+1f",
        ] {
            assert!(Rgba::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn contrast_matches_wcag_reference_values() {
        let black = Rgba::rgb(0, 0, 0);
        let white = Rgba::rgb(255, 255, 255);
        assert!((black.contrast(white) - 21.0).abs() < 1e-9);
        assert!((white.contrast(white) - 1.0).abs() < 1e-9);
        assert!((Rgba::rgb(0x77, 0x77, 0x77).contrast(white) - 4.48).abs() < 0.01);
    }
}
