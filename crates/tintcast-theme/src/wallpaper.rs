use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageEncoder, ImageError, ImageFormat, ImageReader, Limits, RgbImage};

use crate::limits::{MAX_IMAGE_ALLOC, MAX_IMAGE_FILE_BYTES, MAX_IMAGE_SIDE};

pub const IMAGE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
pub const THUMBNAIL_WIDTH: u32 = 480;
pub const THUMBNAIL_HEIGHT: u32 = 270;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WallpaperError {
    #[error("larger than {} MB", MAX_IMAGE_FILE_BYTES / 1_000_000)]
    FileTooLarge,
    #[error("larger than {MAX_IMAGE_SIDE} px")]
    TooLarge,
    #[error("it needs more than {} MiB of memory to decode", MAX_IMAGE_ALLOC / (1024 * 1024))]
    TooMuchMemory,
    #[error("it is not a PNG, JPEG, or WebP image")]
    Unsupported,
    #[error("it is corrupt or unreadable ({0})")]
    Corrupt(String),
    #[error("Tintcast can't store it ({0})")]
    Store(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file} was skipped: {reason}.")]
pub struct RejectedWallpaper {
    pub file: String,
    pub reason: WallpaperError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub png: Vec<u8>,
    pub thumbnail: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub fn is_image_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(ext))
    })
}

pub fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    limits
}

fn classify(error: ImageError) -> WallpaperError {
    match error {
        ImageError::Limits(limit) => match limit.kind() {
            image::error::LimitErrorKind::DimensionError => WallpaperError::TooLarge,
            _ => WallpaperError::TooMuchMemory,
        },
        ImageError::Unsupported(_) => WallpaperError::Unsupported,
        other => WallpaperError::Corrupt(other.to_string()),
    }
}

fn reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>, WallpaperError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| WallpaperError::Corrupt(e.to_string()))?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return Err(WallpaperError::Unsupported);
    }
    reader.limits(limits());
    Ok(reader)
}

pub fn decode(bytes: &[u8]) -> Result<RgbImage, WallpaperError> {
    if bytes.len() as u64 > MAX_IMAGE_FILE_BYTES {
        return Err(WallpaperError::FileTooLarge);
    }
    let (width, height) = reader(bytes)?.into_dimensions().map_err(classify)?;
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(WallpaperError::TooLarge);
    }
    if width == 0 || height == 0 {
        return Err(WallpaperError::Corrupt("it has no pixels".into()));
    }
    let image = reader(bytes)?.decode().map_err(classify)?;
    Ok(match image {
        DynamicImage::ImageRgb8(rgb) => rgb,
        other => other.to_rgb8(),
    })
}

pub fn convert(bytes: &[u8]) -> Result<Converted, WallpaperError> {
    encode(&decode(bytes)?)
}

fn png(image: &RgbImage) -> Result<Vec<u8>, WallpaperError> {
    let mut out = Vec::with_capacity(image.as_raw().len() / 2);
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| WallpaperError::Store(e.to_string()))?;
    Ok(out)
}

pub fn thumbnail(image: &RgbImage) -> RgbImage {
    let (width, height) = image.dimensions();
    let target = u64::from(THUMBNAIL_WIDTH) * u64::from(height);
    let actual = u64::from(THUMBNAIL_HEIGHT) * u64::from(width);
    let (crop_w, crop_h) = if actual > target {
        let w =
            (u64::from(height) * u64::from(THUMBNAIL_WIDTH) / u64::from(THUMBNAIL_HEIGHT)).max(1);
        (u32::try_from(w).unwrap_or(width).min(width), height)
    } else {
        let h =
            (u64::from(width) * u64::from(THUMBNAIL_HEIGHT) / u64::from(THUMBNAIL_WIDTH)).max(1);
        (width, u32::try_from(h).unwrap_or(height).min(height))
    };
    let x = (width - crop_w) / 2;
    let y = (height - crop_h) / 2;
    let cropped = image::imageops::crop_imm(image, x, y, crop_w, crop_h).to_image();
    image::imageops::thumbnail(&cropped, THUMBNAIL_WIDTH, THUMBNAIL_HEIGHT)
}

pub fn encode(image: &RgbImage) -> Result<Converted, WallpaperError> {
    Ok(Converted {
        png: png(image)?,
        thumbnail: png(&thumbnail(image))?,
        width: image.width(),
        height: image.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;
    use image::codecs::webp::WebPEncoder;

    fn sample(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        })
    }

    fn webp(image: &RgbImage) -> Vec<u8> {
        let mut out = Vec::new();
        WebPEncoder::new_lossless(&mut out)
            .encode(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    fn png_bytes(image: &RgbImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn png_jpeg_and_webp_convert_to_png_with_a_sixteen_by_nine_thumbnail() {
        let image = sample(64, 48);
        let mut jpeg = Cursor::new(Vec::new());
        image.write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
        for bytes in [png_bytes(&image), jpeg.into_inner(), webp(&image)] {
            let converted = convert(&bytes).unwrap();
            assert_eq!((converted.width, converted.height), (64, 48));
            let stored =
                image::load_from_memory_with_format(&converted.png, ImageFormat::Png).unwrap();
            assert_eq!((stored.width(), stored.height()), (64, 48));
            let thumb = image::load_from_memory_with_format(&converted.thumbnail, ImageFormat::Png)
                .unwrap();
            assert_eq!((thumb.width(), thumb.height()), (480, 270));
        }
    }

    #[test]
    fn images_over_the_side_limit_are_rejected_from_their_header() {
        let mut header = Vec::new();
        header.extend_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
        header.extend_from_slice(&20_000u32.to_be_bytes());
        header.extend_from_slice(&100u32.to_be_bytes());
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        let crc = crc(&header[12..]);
        header.extend_from_slice(&crc.to_be_bytes());
        assert_eq!(decode(&header).unwrap_err(), WallpaperError::TooLarge);
        let rejected = RejectedWallpaper {
            file: "huge.png".into(),
            reason: WallpaperError::TooLarge,
        };
        assert_eq!(
            rejected.to_string(),
            "huge.png was skipped: larger than 16384 px."
        );
    }

    fn crc(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn corrupt_unknown_and_oversized_files_are_rejected() {
        let mut truncated = png_bytes(&sample(32, 32));
        truncated.truncate(truncated.len() / 2);
        assert!(matches!(
            decode(&truncated),
            Err(WallpaperError::Corrupt(_))
        ));
        assert_eq!(
            decode(b"GIF89a....").unwrap_err(),
            WallpaperError::Unsupported
        );
        assert_eq!(
            decode(b"not an image").unwrap_err(),
            WallpaperError::Unsupported
        );
        let big = vec![0u8; 64_000_001];
        assert_eq!(decode(&big).unwrap_err(), WallpaperError::FileTooLarge);
    }

    fn vp8l_chunk(image: &RgbImage) -> Vec<u8> {
        let file = webp(image);
        let start = file.windows(4).position(|w| w == b"VP8L").unwrap();
        file[start..].to_vec()
    }

    fn u24(value: u32) -> [u8; 3] {
        let bytes = value.to_le_bytes();
        [bytes[0], bytes[1], bytes[2]]
    }

    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        out.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn animated(frames: &[RgbImage]) -> Vec<u8> {
        let (w, h) = frames[0].dimensions();
        let mut vp8x = vec![0x02, 0, 0, 0];
        vp8x.extend_from_slice(&u24(w - 1));
        vp8x.extend_from_slice(&u24(h - 1));
        let mut body = b"WEBP".to_vec();
        body.extend(chunk(b"VP8X", &vp8x));
        body.extend(chunk(b"ANIM", &[0, 0, 0, 0, 0, 0]));
        for frame in frames {
            let mut anmf = Vec::new();
            anmf.extend_from_slice(&u24(0));
            anmf.extend_from_slice(&u24(0));
            anmf.extend_from_slice(&u24(w - 1));
            anmf.extend_from_slice(&u24(h - 1));
            anmf.extend_from_slice(&u24(100));
            anmf.push(0);
            anmf.extend(vp8l_chunk(frame));
            body.extend(chunk(b"ANMF", &anmf));
        }
        chunk(b"RIFF", &body)
    }

    #[test]
    fn an_animated_webp_gives_its_first_frame() {
        let red = RgbImage::from_pixel(16, 9, Rgb([220, 30, 30]));
        let blue = RgbImage::from_pixel(16, 9, Rgb([30, 30, 220]));
        let file = animated(&[red, blue]);
        let decoded = decode(&file).unwrap();
        assert_eq!(decoded.dimensions(), (16, 9));
        let [r, g, b] = decoded.get_pixel(8, 4).0;
        assert!(r >= 210 && g <= 40 && b <= 40, "{:?}", [r, g, b]);
    }

    #[test]
    fn other_formats_are_unsupported_even_when_another_crate_enables_their_decoder() {
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&58u32.to_le_bytes());
        bmp.extend_from_slice(&[0, 0, 0, 0]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[10, 20, 30, 0]);
        let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";
        for bytes in [&bmp[..], &gif[..]] {
            assert_eq!(decode(bytes).unwrap_err(), WallpaperError::Unsupported);
        }
    }

    #[test]
    fn image_names_need_a_known_extension() {
        assert!(is_image_name("1-totoro.webp"));
        assert!(is_image_name("A.JPEG"));
        assert!(!is_image_name("intro.mp4"));
        assert!(!is_image_name(".png"));
        assert!(!is_image_name("png"));
    }
}
