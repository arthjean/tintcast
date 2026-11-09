use image::{Rgb, RgbImage};

use crate::color::Rgba;
use crate::palette::{Mode, Palette};

pub const GENERATOR_VERSION: u32 = 1;
pub const VARIANTS: usize = 3;
pub const WIDTH: u32 = 3840;
pub const HEIGHT: u32 = 2160;

type Color = [f32; 3];

fn rgb(color: Rgba) -> Color {
    [f32::from(color.r), f32::from(color.g), f32::from(color.b)]
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn noise(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9e37_79b1) ^ y.wrapping_mul(0x85eb_ca77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    f32::from((h & 0xff) as u8) / 255.0 - 0.5
}

fn finish(color: Color, x: u32, y: u32) -> Rgb<u8> {
    let n = noise(x, y) * 1.5;
    Rgb(color.map(|c| (c + n).round().clamp(0.0, 255.0) as u8))
}

struct Scene {
    background: Color,
    deep: Color,
    accent: Color,
    tints: [Color; 4],
    strength: f32,
}

impl Scene {
    fn new(palette: &Palette) -> Self {
        let background = rgb(palette.background);
        let (deep, strength) = match palette.mode {
            Mode::Dark => (mix(background, [0.0; 3], 0.45), 0.42),
            Mode::Light => (mix(background, rgb(palette.foreground), 0.08), 0.28),
        };
        Self {
            background,
            deep,
            accent: rgb(palette.accent),
            tints: [
                rgb(palette.ansi[4]),
                rgb(palette.ansi[5]),
                rgb(palette.ansi[6]),
                rgb(palette.ansi[2]),
            ],
            strength,
        }
    }

    fn sky(&self, v: f32) -> Color {
        mix(self.background, self.deep, v)
    }
}

fn glow(scene: &Scene, u: f32, v: f32, aspect: f32) -> Color {
    let mut color = scene.sky(v);
    let lights = [
        (0.78 * aspect, 0.30, 0.55, scene.accent, 1.0),
        (0.16 * aspect, 0.78, 0.62, scene.tints[0], 0.8),
        (0.52 * aspect, 1.08, 0.48, scene.tints[1], 0.7),
    ];
    for (cx, cy, radius, tint, weight) in lights {
        let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() / radius;
        color = mix(color, tint, scene.strength * weight * (-d * d).exp());
    }
    color
}

fn ridges(scene: &Scene, u: f32, v: f32, aspect: f32, pixel: f32) -> Color {
    let sun = ((u - 0.70 * aspect).powi(2) + (v - 0.30).powi(2)).sqrt();
    let mut color = mix(
        scene.sky(v * 0.8),
        scene.accent,
        scene.strength * (1.0 - smoothstep(0.10, 0.11 + pixel, sun)),
    );
    color = mix(
        color,
        scene.accent,
        scene.strength * 0.35 * (-(sun / 0.35).powi(2)).exp(),
    );
    for layer in 0..5u8 {
        let i = f32::from(layer);
        let base = 0.50 + i * 0.105;
        let curve = base
            + 0.045 * (u * (2.1 + i * 0.7) + i * 1.9).sin()
            + 0.020 * (u * (5.3 + i * 1.3) + i * 0.7).sin();
        let cover = smoothstep(curve - pixel, curve + pixel, v);
        let tint = scene.tints[usize::from(layer) % scene.tints.len()];
        let fill = mix(
            mix(scene.deep, tint, scene.strength * (0.30 + i * 0.12)),
            scene.deep,
            i * 0.12,
        );
        color = mix(color, fill, cover);
    }
    color
}

fn rings(scene: &Scene, u: f32, v: f32, aspect: f32, pixel: f32) -> Color {
    let mut color = mix(scene.background, scene.deep, (u / aspect) * 0.6 + v * 0.4);
    let d = ((u - 0.12 * aspect).powi(2) + (v - 0.88).powi(2)).sqrt();
    let spacing = 0.085;
    let band = d / spacing;
    let index = band.floor();
    let within = (band - index) * spacing;
    let edge = smoothstep(0.0, pixel * 2.0, within)
        * (1.0 - smoothstep(spacing - pixel * 2.0, spacing, within));
    if (1.0..18.0).contains(&index) {
        let k = index as usize;
        let fade = 1.0 - index / 18.0;
        let (tint, amount) = if k == 7 {
            (scene.accent, scene.strength * 1.4)
        } else {
            (scene.tints[k % scene.tints.len()], scene.strength * 0.45)
        };
        color = mix(
            color,
            tint,
            amount * fade * edge * if k.is_multiple_of(2) { 1.0 } else { 0.55 },
        );
    }
    let vignette = ((u / aspect - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt();
    mix(color, scene.deep, smoothstep(0.35, 0.95, vignette) * 0.5)
}

pub fn wallpaper(palette: &Palette, variant: usize, width: u32, height: u32) -> RgbImage {
    let scene = Scene::new(palette);
    let h = height.max(1) as f32;
    let aspect = width as f32 / h;
    let pixel = 1.5 / h;
    RgbImage::from_fn(width, height, |x, y| {
        let u = x as f32 / h;
        let v = y as f32 / h;
        let color = match variant % VARIANTS {
            0 => glow(&scene, u, v, aspect),
            1 => ridges(&scene, u, v, aspect, pixel),
            _ => rings(&scene, u, v, aspect, pixel),
        };
        finish(color, x, y)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> Palette {
        Palette::new(Rgba::rgb(0x14, 0x16, 0x1c), Rgba::rgb(0xe0, 0xe0, 0xe0))
            .with_accent(Rgba::rgb(0x3f, 0xb8, 0xaf))
    }

    #[test]
    fn the_same_palette_gives_the_same_pixels() {
        for variant in 0..VARIANTS {
            let a = wallpaper(&palette(), variant, 192, 108);
            let b = wallpaper(&palette(), variant, 192, 108);
            assert_eq!(a.as_raw(), b.as_raw());
        }
    }

    #[test]
    fn variants_and_palettes_differ() {
        let images: Vec<_> = (0..VARIANTS)
            .map(|v| wallpaper(&palette(), v, 96, 54))
            .collect();
        assert_ne!(images[0].as_raw(), images[1].as_raw());
        assert_ne!(images[1].as_raw(), images[2].as_raw());
        let other = palette().with_accent(Rgba::rgb(0xff, 0x00, 0x00));
        assert_ne!(wallpaper(&other, 0, 96, 54).as_raw(), images[0].as_raw());
    }
}
