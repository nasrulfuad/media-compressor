use std::io::Cursor;

use image::{DynamicImage, ImageReader};
use tracing::{debug, warn};

use crate::codec::jpeg::{encode_jpeg, JpegSettings, ScanMode};
use crate::codec::png::{encode_png, optimize_png};
use crate::domain::media::{Dimensions, ImageFormat};
use crate::domain::options::{ColorMode, ImageOptions, MetadataPolicy};
use crate::error::{AppError, AppResult};

use super::format_resolution::resolve_output_format;
use super::transform::{flatten_onto_white, resize_to_max_dimension, to_grayscale};

pub const MAX_IMAGE_SIDE_PIXELS: u32 = 65_535;
pub const MAX_IMAGE_DECODE_ALLOC_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedImage {
    pub bytes: Vec<u8>,
    pub output_format: ImageFormat,
    pub original_dimensions: Dimensions,
    pub output_dimensions: Dimensions,
    pub kept_original: bool,
}

fn dimensions_of(image: &DynamicImage) -> Dimensions {
    Dimensions {
        width: image.width(),
        height: image.height(),
    }
}

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE_PIXELS);
    limits.max_image_height = Some(MAX_IMAGE_SIDE_PIXELS);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC_BYTES);
    limits
}

fn reader_for(bytes: &[u8]) -> AppResult<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| AppError::ImageDecode(error.to_string()))?;
    reader.limits(decode_limits());
    Ok(reader)
}

fn decode_image(bytes: &[u8], max_pixels: u64) -> AppResult<DynamicImage> {
    let (width, height) = reader_for(bytes)?
        .into_dimensions()
        .map_err(|error| AppError::ImageDecode(format!("failed to read image header: {error}")))?;
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(AppError::ImageTooLarge {
            width,
            height,
            limit: max_pixels,
        });
    }
    reader_for(bytes)?
        .decode()
        .map_err(|error| AppError::ImageDecode(format!("failed to decode image: {error}")))
}

fn apply_transforms(mut image: DynamicImage, options: &ImageOptions) -> DynamicImage {
    if let Some(max_dimension) = options.max_dimension {
        image = resize_to_max_dimension(image, max_dimension);
    }
    if options.color_mode == ColorMode::Grayscale {
        image = to_grayscale(image);
    }
    image
}

fn encode_as_jpeg(image: &DynamicImage, options: &ImageOptions) -> AppResult<Vec<u8>> {
    let flattened;
    let source = if image.color().has_alpha() {
        warn!(
            reason = "JPEG has no alpha channel",
            "Flattening transparency onto white"
        );
        flattened = DynamicImage::ImageRgb8(flatten_onto_white(image));
        &flattened
    } else {
        image
    };
    encode_jpeg(
        source,
        JpegSettings {
            quality: options.quality,
            color_mode: options.color_mode,
            scan_mode: ScanMode::Progressive,
        },
    )
}

fn encode_image(
    image: &DynamicImage,
    format: ImageFormat,
    options: &ImageOptions,
) -> AppResult<Vec<u8>> {
    match format {
        ImageFormat::Jpeg => encode_as_jpeg(image, options),
        ImageFormat::Png => encode_png(image, options.level.preset().png_effort),
        other => Err(AppError::UnsupportedFormat(format!(
            "{} output format is not supported",
            other.display_name()
        ))),
    }
}

fn is_lossless_png_passthrough(
    input_format: ImageFormat,
    output_format: ImageFormat,
    options: &ImageOptions,
) -> bool {
    input_format == ImageFormat::Png
        && output_format == ImageFormat::Png
        && options.max_dimension.is_none()
        && options.color_mode == ColorMode::Preserve
}

fn finish(input: &[u8], input_format: ImageFormat, encoded: EncodedImage) -> EncodedImage {
    let unchanged_format = encoded.output_format == input_format;
    if unchanged_format && encoded.bytes.len() >= input.len() {
        debug!("Re-encoded image is not smaller; keeping original bytes");
        return EncodedImage {
            bytes: input.to_vec(),
            kept_original: true,
            ..encoded
        };
    }
    encoded
}

pub fn compress_image_bytes(
    input: &[u8],
    input_format: ImageFormat,
    options: &ImageOptions,
) -> AppResult<EncodedImage> {
    if !input_format.is_decodable() {
        return Err(AppError::UnsupportedFormat(
            input_format.unsupported_input_message(),
        ));
    }
    if options.metadata == MetadataPolicy::Keep {
        warn!("Keeping metadata is not supported for images; metadata is stripped by re-encoding");
    }
    let output_format =
        resolve_output_format(options.output_format, input_format, options.encoding);
    let decoded = decode_image(input, options.limits.max_image_pixels)?;
    let original_dimensions = dimensions_of(&decoded);
    if is_lossless_png_passthrough(input_format, output_format, options) {
        let bytes = optimize_png(input, options.level.preset().png_effort)?;
        let encoded = EncodedImage {
            bytes,
            output_format,
            original_dimensions,
            output_dimensions: original_dimensions,
            kept_original: false,
        };
        return Ok(finish(input, input_format, encoded));
    }
    let transformed = apply_transforms(decoded, options);
    let bytes = encode_image(&transformed, output_format, options)?;
    let encoded = EncodedImage {
        bytes,
        output_format,
        original_dimensions,
        output_dimensions: dimensions_of(&transformed),
        kept_original: false,
    };
    Ok(finish(input, input_format, encoded))
}

#[cfg(test)]
mod tests {
    use image::ImageFormat as CrateFormat;

    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::domain::media::OutputFormat;
    use crate::domain::options::ImageEncodingMode;
    use crate::test_support::{
        encode_as, gradient_rgb, gradient_rgb16, gradient_rgba_with_transparency,
    };

    fn options() -> ImageOptions {
        ImageOptions::for_level(CompressionLevel::Medium)
    }

    fn jpeg_input(width: u32, height: u32) -> Vec<u8> {
        encode_as(&gradient_rgb(width, height), CrateFormat::Jpeg)
    }

    fn png_input(width: u32, height: u32) -> Vec<u8> {
        encode_as(&gradient_rgb(width, height), CrateFormat::Png)
    }

    #[test]
    fn compress_jpeg_returns_valid_jpeg_with_same_dimensions() {
        let input = jpeg_input(64, 64);
        let result = compress_image_bytes(&input, ImageFormat::Jpeg, &options()).unwrap();
        assert_eq!(result.output_format, ImageFormat::Jpeg);
        assert_eq!(&result.bytes[..2], &[0xFF, 0xD8]);
        assert_eq!(
            result.original_dimensions,
            Dimensions {
                width: 64,
                height: 64
            }
        );
        assert_eq!(result.output_dimensions, result.original_dimensions);
    }

    #[test]
    fn compress_jpeg_with_lower_quality_is_not_larger_than_original() {
        let input = encode_as(&gradient_rgb(128, 128), CrateFormat::Jpeg);
        let mut settings = options();
        settings.quality = 30;
        let result = compress_image_bytes(&input, ImageFormat::Jpeg, &settings).unwrap();
        assert!(result.bytes.len() <= input.len());
    }

    #[test]
    fn compress_png_without_transforms_keeps_pixels_and_alpha() {
        let image = gradient_rgba_with_transparency(16, 16);
        let input = encode_as(&image, CrateFormat::Png);
        let result = compress_image_bytes(&input, ImageFormat::Png, &options()).unwrap();
        assert_eq!(result.output_format, ImageFormat::Png);
        let decoded = image::load_from_memory(&result.bytes).unwrap().to_rgba8();
        assert_eq!(decoded, image.to_rgba8());
    }

    #[test]
    fn compress_png_preserves_sixteen_bit_depth_without_transforms() {
        let image = gradient_rgb16(16, 16);
        let input = encode_as(&image, CrateFormat::Png);
        let result = compress_image_bytes(&input, ImageFormat::Png, &options()).unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap();
        assert!(matches!(decoded, DynamicImage::ImageRgb16(_)));
        assert_eq!(decoded.to_rgb16(), image.to_rgb16());
    }

    #[test]
    fn compress_png_to_jpeg_converts_format() {
        let mut settings = options();
        settings.output_format = OutputFormat::Jpeg;
        let result = compress_image_bytes(&png_input(32, 32), ImageFormat::Png, &settings).unwrap();
        assert_eq!(result.output_format, ImageFormat::Jpeg);
        assert_eq!(&result.bytes[..2], &[0xFF, 0xD8]);
        assert!(!result.kept_original);
    }

    #[test]
    fn compress_transparent_png_to_jpeg_flattens_onto_white() {
        let image = gradient_rgba_with_transparency(16, 16);
        let input = encode_as(&image, CrateFormat::Png);
        let mut settings = options();
        settings.output_format = OutputFormat::Jpeg;
        settings.quality = 95;
        let result = compress_image_bytes(&input, ImageFormat::Png, &settings).unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap().to_rgb8();
        let corner = decoded.get_pixel(0, 0).0;
        assert!(corner.iter().all(|channel| *channel >= 250), "{corner:?}");
    }

    #[test]
    fn compress_jpeg_to_png_converts_format_and_round_trips() {
        let mut settings = options();
        settings.output_format = OutputFormat::Png;
        let result =
            compress_image_bytes(&jpeg_input(32, 32), ImageFormat::Jpeg, &settings).unwrap();
        assert_eq!(result.output_format, ImageFormat::Png);
        assert_eq!(&result.bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn compress_lossless_mode_converts_jpeg_to_png() {
        let mut settings = options();
        settings.encoding = ImageEncodingMode::Lossless;
        let result =
            compress_image_bytes(&jpeg_input(32, 32), ImageFormat::Jpeg, &settings).unwrap();
        assert_eq!(result.output_format, ImageFormat::Png);
    }

    #[test]
    fn compress_with_max_dimension_resizes_and_reports_new_dimensions() {
        let mut settings = options();
        settings.max_dimension = Some(100);
        let result =
            compress_image_bytes(&jpeg_input(200, 200), ImageFormat::Jpeg, &settings).unwrap();
        assert_eq!(
            result.original_dimensions,
            Dimensions {
                width: 200,
                height: 200
            }
        );
        assert_eq!(
            result.output_dimensions,
            Dimensions {
                width: 100,
                height: 100
            }
        );
        let decoded = image::load_from_memory(&result.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 100));
    }

    #[test]
    fn compress_grayscale_outputs_single_channel_image() {
        let mut settings = options();
        settings.color_mode = ColorMode::Grayscale;
        let result = compress_image_bytes(&png_input(32, 32), ImageFormat::Png, &settings).unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap();
        assert_eq!(decoded.color().channel_count(), 1);
    }

    #[test]
    fn compress_keeps_original_bytes_when_reencoding_is_not_smaller() {
        let tiny = encode_as(&gradient_rgb(2, 2), CrateFormat::Jpeg);
        let mut settings = options();
        settings.quality = 100;
        let result = compress_image_bytes(&tiny, ImageFormat::Jpeg, &settings).unwrap();
        if result.kept_original {
            assert_eq!(result.bytes, tiny);
        } else {
            assert!(result.bytes.len() < tiny.len());
        }
    }

    #[test]
    fn compress_rejects_undecodable_input_formats() {
        for format in [
            ImageFormat::Gif,
            ImageFormat::Tiff,
            ImageFormat::Bmp,
            ImageFormat::WebP,
        ] {
            let error = compress_image_bytes(b"data", format, &options()).unwrap_err();
            assert!(
                matches!(&error, AppError::UnsupportedFormat(message) if message.contains(format.display_name())),
                "{error}"
            );
        }
    }

    #[test]
    fn compress_rejects_corrupt_jpeg() {
        let mut corrupt = vec![0xFF, 0xD8, 0xFF, 0xE0];
        corrupt.extend_from_slice(&[0u8; 32]);
        let error = compress_image_bytes(&corrupt, ImageFormat::Jpeg, &options()).unwrap_err();
        assert!(matches!(error, AppError::ImageDecode(_)));
    }

    #[test]
    fn compress_image_rejects_dimensions_above_pixel_limit() {
        let mut settings = options();
        settings.limits.max_image_pixels = 100;
        let error =
            compress_image_bytes(&png_input(32, 32), ImageFormat::Png, &settings).unwrap_err();
        assert!(matches!(
            error,
            AppError::ImageTooLarge {
                width: 32,
                height: 32,
                limit: 100
            }
        ));
    }

    #[test]
    fn compress_image_rejects_png_header_declaring_huge_dimensions() {
        let mut png = png_input(8, 8);
        png[16..20].copy_from_slice(&70_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&70_000u32.to_be_bytes());
        let error = compress_image_bytes(&png, ImageFormat::Png, &options()).unwrap_err();
        assert!(matches!(
            error,
            AppError::ImageDecode(_) | AppError::ImageTooLarge { .. }
        ));
    }
}
