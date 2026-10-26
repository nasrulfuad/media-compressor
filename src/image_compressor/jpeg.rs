use image::DynamicImage;

use crate::error::{AppError, AppResult};

/// Encode `img` as JPEG using mozjpeg.
///
/// - `quality`: 1–100 (higher = better quality, larger file).
/// - `grayscale`: when `true`, encodes as single-channel grayscale.
/// - `progressive`: when `true`, produces a progressive JPEG scan.
pub fn encode_jpeg(
    img: &DynamicImage,
    quality: u8,
    grayscale: bool,
    progressive: bool,
) -> AppResult<Vec<u8>> {
    if grayscale {
        encode_gray(img, quality, progressive)
    } else {
        encode_rgb(img, quality, progressive)
    }
}

fn encode_rgb(img: &DynamicImage, quality: u8, progressive: bool) -> AppResult<Vec<u8>> {
    let rgb = img.to_rgb8();
    let width = rgb.width() as usize;
    let height = rgb.height() as usize;
    let pixels: &[u8] = rgb.as_raw();

    // mozjpeg uses longjmp internally; catch_unwind keeps the Rust stack safe.
    std::panic::catch_unwind(|| {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(width, height);
        comp.set_quality(quality as f32);
        if progressive {
            comp.set_progressive_mode();
        }

        let mut started = comp
            .start_compress(Vec::new())
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg start_compress: {e}")))?;

        started
            .write_scanlines(pixels)
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg write_scanlines: {e}")))?;

        started
            .finish()
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg finish: {e}")))
    })
    .map_err(|_| AppError::ImageEncode("mozjpeg panicked during RGB encode".to_string()))?
}

fn encode_gray(img: &DynamicImage, quality: u8, progressive: bool) -> AppResult<Vec<u8>> {
    let gray = img.to_luma8();
    let width = gray.width() as usize;
    let height = gray.height() as usize;
    let pixels: &[u8] = gray.as_raw();

    std::panic::catch_unwind(|| {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_GRAYSCALE);
        comp.set_size(width, height);
        comp.set_quality(quality as f32);
        if progressive {
            comp.set_progressive_mode();
        }

        let mut started = comp
            .start_compress(Vec::new())
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg start_compress (gray): {e}")))?;

        started
            .write_scanlines(pixels)
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg write_scanlines (gray): {e}")))?;

        started
            .finish()
            .map_err(|e| AppError::ImageEncode(format!("mozjpeg finish (gray): {e}")))
    })
    .map_err(|_| AppError::ImageEncode("mozjpeg panicked during grayscale encode".to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_jpeg_rgb_produces_valid_jpeg() {
        let img = DynamicImage::new_rgb8(16, 16);
        let bytes = encode_jpeg(&img, 75, false, true).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(
            &bytes[..2],
            &[0xFF, 0xD8],
            "Must start with JPEG SOI marker"
        );
    }

    #[test]
    fn encode_jpeg_grayscale_produces_valid_jpeg() {
        let img = DynamicImage::new_rgb8(16, 16);
        let bytes = encode_jpeg(&img, 75, true, true).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(
            &bytes[..2],
            &[0xFF, 0xD8],
            "Must start with JPEG SOI marker"
        );
    }

    #[test]
    fn encode_jpeg_non_progressive_produces_valid_jpeg() {
        let img = DynamicImage::new_rgb8(16, 16);
        let bytes = encode_jpeg(&img, 75, false, false).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }
}
