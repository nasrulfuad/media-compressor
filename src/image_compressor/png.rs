use std::io::Cursor;

use image::DynamicImage;

use crate::error::{AppError, AppResult};

/// Encode `img` as an optimized PNG using oxipng.
///
/// Returns full PNG file bytes (not raw pixel data).
///
/// - `grayscale`: when `true`, encodes as single-channel grayscale.
/// - `effort`: oxipng preset 0–6; higher = more optimization passes, slower.
pub fn encode_png(img: &DynamicImage, grayscale: bool, effort: u8) -> AppResult<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();

    // First pass: encode as PNG with the `image` crate.
    {
        let cursor = Cursor::new(&mut buf);
        if grayscale {
            let gray = img.to_luma8();
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                gray.as_raw(),
                gray.width(),
                gray.height(),
                image::ExtendedColorType::L8,
            )
            .map_err(|e| AppError::ImageEncode(format!("PNG encode (gray): {e}")))?;
        } else {
            let rgb = img.to_rgb8();
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| AppError::ImageEncode(format!("PNG encode (rgb): {e}")))?;
        }
    }

    // Second pass: optimize with oxipng for better lossless compression.
    let preset = effort.min(6);
    oxipng::optimize_from_memory(&buf, &oxipng::Options::from_preset(preset))
        .map_err(|e| AppError::ImageEncode(format!("oxipng optimization: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_HEADER: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn encode_png_produces_valid_png_header() {
        let img = DynamicImage::new_rgb8(16, 16);
        let bytes = encode_png(&img, false, 2).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..8], &PNG_HEADER, "Must start with PNG signature");
    }

    #[test]
    fn encode_png_grayscale_produces_valid_png() {
        let img = DynamicImage::new_rgb8(16, 16);
        let bytes = encode_png(&img, true, 2).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..8], &PNG_HEADER);
    }

    #[test]
    fn encode_png_effort_clamped_to_valid_range() {
        // effort=255 should not panic; clamped to 6 internally.
        let img = DynamicImage::new_rgb8(8, 8);
        let result = encode_png(&img, false, 255);
        assert!(result.is_ok());
    }
}
