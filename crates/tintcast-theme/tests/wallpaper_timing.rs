#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use image::codecs::webp::WebPEncoder;
use image::{ExtendedColorType, Rgb};
use tintcast_theme::{BUNDLED, generate, wallpaper};

#[test]
fn a_4k_webp_converts_within_the_budget() {
    let palette = BUNDLED[2].parse().unwrap().palette;
    let mut source = generate::wallpaper(&palette, 1, 3840, 2160);
    for (x, y, pixel) in source.enumerate_pixels_mut() {
        let grain = ((x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)) % 23) as u8;
        *pixel = Rgb(pixel.0.map(|c| c.saturating_add(grain)));
    }
    let mut webp = Vec::new();
    WebPEncoder::new_lossless(&mut webp)
        .encode(source.as_raw(), 3840, 2160, ExtendedColorType::Rgb8)
        .unwrap();
    let started = Instant::now();
    let converted = wallpaper::convert(&webp).unwrap();
    let elapsed = started.elapsed();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "3840x2160 WebP ({} KB) converted to PNG ({} KB) and thumbnail in {} ms ({profile} build)",
        webp.len() / 1000,
        converted.png.len() / 1000,
        elapsed.as_millis()
    );
    assert_eq!((converted.width, converted.height), (3840, 2160));
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_millis() <= 800,
            "took {} ms",
            elapsed.as_millis()
        );
    }
}
