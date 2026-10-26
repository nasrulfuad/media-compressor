use std::io::Cursor;

use image::DynamicImage;

use crate::error::{AppError, AppResult};

pub const MAX_OXIPNG_PRESET: u8 = 6;

pub fn encode_png(image: &DynamicImage, effort: u8) -> AppResult<Vec<u8>> {
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| AppError::Encode(format!("PNG encode: {error}")))?;
    optimize_png(&encoded.into_inner(), effort)
}

pub fn optimize_png(png_bytes: &[u8], effort: u8) -> AppResult<Vec<u8>> {
    let preset = effort.min(MAX_OXIPNG_PRESET);
    oxipng::optimize_from_memory(png_bytes, &oxipng::Options::from_preset(preset))
        .map_err(|error| AppError::Encode(format!("oxipng optimization: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{gradient_rgb, gradient_rgba_with_transparency};

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn encode_png_produces_png_signature_and_round_trips_pixels() {
        let image = gradient_rgb(16, 16);
        let bytes = encode_png(&image, 2).unwrap();
        assert_eq!(&bytes[..8], &PNG_SIGNATURE);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.to_rgb8(), image.to_rgb8());
    }

    #[test]
    fn encode_png_keeps_grayscale_color_type() {
        let gray = DynamicImage::ImageLuma8(gradient_rgb(8, 8).to_luma8());
        let bytes = encode_png(&gray, 2).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.color().channel_count(), 1);
    }

    #[test]
    fn encode_png_keeps_alpha_channel() {
        let image = gradient_rgba_with_transparency(8, 8);
        let bytes = encode_png(&image, 2).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0[3], 0);
        assert_eq!(decoded.get_pixel(7, 0).0[3], 255);
    }

    #[test]
    fn encode_png_clamps_effort_above_maximum() {
        let bytes = encode_png(&gradient_rgb(8, 8), 255).unwrap();
        assert_eq!(&bytes[..8], &PNG_SIGNATURE);
    }

    #[test]
    fn optimize_png_never_returns_invalid_png_for_garbage() {
        let error = optimize_png(b"not a png", 2).unwrap_err();
        assert!(matches!(error, AppError::Encode(_)));
    }
}
