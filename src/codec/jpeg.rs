use std::panic::{catch_unwind, AssertUnwindSafe};

use image::DynamicImage;

use crate::domain::options::ColorMode;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    Progressive,
    Baseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegSettings {
    pub quality: u8,
    pub color_mode: ColorMode,
    pub scan_mode: ScanMode,
}

struct RawPixels {
    color_space: mozjpeg::ColorSpace,
    width: usize,
    height: usize,
    samples: Vec<u8>,
}

fn raw_pixels(image: &DynamicImage, color_mode: ColorMode) -> RawPixels {
    let width = image.width() as usize;
    let height = image.height() as usize;
    match color_mode {
        ColorMode::Grayscale => RawPixels {
            color_space: mozjpeg::ColorSpace::JCS_GRAYSCALE,
            width,
            height,
            samples: image.to_luma8().into_raw(),
        },
        ColorMode::Preserve => RawPixels {
            color_space: mozjpeg::ColorSpace::JCS_RGB,
            width,
            height,
            samples: image.to_rgb8().into_raw(),
        },
    }
}

fn compress_with_mozjpeg(pixels: &RawPixels, settings: JpegSettings) -> AppResult<Vec<u8>> {
    let mut compressor = mozjpeg::Compress::new(pixels.color_space);
    compressor.set_size(pixels.width, pixels.height);
    compressor.set_quality(f32::from(settings.quality));
    if settings.scan_mode == ScanMode::Progressive {
        compressor.set_progressive_mode();
    }
    let mut started = compressor
        .start_compress(Vec::new())
        .map_err(|error| AppError::Encode(format!("mozjpeg start_compress: {error}")))?;
    started
        .write_scanlines(&pixels.samples)
        .map_err(|error| AppError::Encode(format!("mozjpeg write_scanlines: {error}")))?;
    started
        .finish()
        .map_err(|error| AppError::Encode(format!("mozjpeg finish: {error}")))
}

pub fn encode_jpeg(image: &DynamicImage, settings: JpegSettings) -> AppResult<Vec<u8>> {
    if image.width() == 0 || image.height() == 0 {
        return Err(AppError::Encode("cannot encode an empty image".to_string()));
    }
    let pixels = raw_pixels(image, settings.color_mode);
    catch_unwind(AssertUnwindSafe(|| {
        compress_with_mozjpeg(&pixels, settings)
    }))
    .map_err(|_| AppError::Encode("mozjpeg panicked".to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::gradient_rgb;

    const JPEG_SOI: [u8; 2] = [0xFF, 0xD8];

    fn settings(color_mode: ColorMode, scan_mode: ScanMode) -> JpegSettings {
        JpegSettings {
            quality: 75,
            color_mode,
            scan_mode,
        }
    }

    #[test]
    fn encode_jpeg_baseline_and_progressive_both_start_with_soi() {
        let image = gradient_rgb(16, 16);
        for scan_mode in [ScanMode::Baseline, ScanMode::Progressive] {
            let bytes = encode_jpeg(&image, settings(ColorMode::Preserve, scan_mode)).unwrap();
            assert_eq!(&bytes[..2], &JPEG_SOI, "{scan_mode:?}");
        }
    }

    #[test]
    fn encode_jpeg_grayscale_decodes_as_single_channel() {
        let bytes = encode_jpeg(
            &gradient_rgb(16, 16),
            settings(ColorMode::Grayscale, ScanMode::Baseline),
        )
        .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.color().channel_count(), 1);
    }

    #[test]
    fn encode_jpeg_color_decodes_as_three_channels() {
        let bytes = encode_jpeg(
            &gradient_rgb(16, 16),
            settings(ColorMode::Preserve, ScanMode::Baseline),
        )
        .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.color().channel_count(), 3);
    }

    #[test]
    fn encode_jpeg_lower_quality_gives_smaller_output() {
        let image = gradient_rgb(64, 64);
        let high = JpegSettings {
            quality: 95,
            ..settings(ColorMode::Preserve, ScanMode::Baseline)
        };
        let low = JpegSettings {
            quality: 20,
            ..high
        };
        let high_len = encode_jpeg(&image, high).unwrap().len();
        let low_len = encode_jpeg(&image, low).unwrap().len();
        assert!(low_len < high_len, "{low_len} !< {high_len}");
    }

    #[test]
    fn encode_jpeg_returns_error_for_zero_sized_image() {
        let image = DynamicImage::new_rgb8(0, 0);
        let error =
            encode_jpeg(&image, settings(ColorMode::Preserve, ScanMode::Baseline)).unwrap_err();
        assert!(matches!(error, AppError::Encode(_)));
    }
}
