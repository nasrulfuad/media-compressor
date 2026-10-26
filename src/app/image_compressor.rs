use std::path::Path;

use crate::domain::media::{ImageFormat, MediaType};
use crate::domain::report::MediaDetails;
use crate::error::{AppError, AppResult};
use crate::raster::engine::compress_image_bytes;
use crate::raster::format_resolution::{output_extension, resolve_output_format};

use super::media_compressor::{CompressedMedia, CompressionSettings, MediaCompressor};

pub struct ImageCompressor;

fn input_format_of(media_type: &MediaType) -> AppResult<ImageFormat> {
    match media_type {
        MediaType::Image(format) => Ok(*format),
        other => Err(AppError::UnsupportedFormat(format!(
            "image compressor cannot handle {other:?}"
        ))),
    }
}

impl MediaCompressor for ImageCompressor {
    fn name(&self) -> &'static str {
        "Image"
    }

    fn supports(&self, media_type: &MediaType) -> bool {
        matches!(media_type, MediaType::Image(format) if format.is_decodable())
    }

    fn output_extension(
        &self,
        input_path: &Path,
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> String {
        let MediaType::Image(input_format) = media_type else {
            return String::new();
        };
        let output_format = resolve_output_format(
            settings.image.output_format,
            *input_format,
            settings.image.encoding,
        );
        output_extension(input_path, *input_format, output_format)
    }

    fn compress(
        &self,
        input: &[u8],
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> AppResult<CompressedMedia> {
        let input_format = input_format_of(media_type)?;
        let encoded = compress_image_bytes(input, input_format, &settings.image)?;
        Ok(CompressedMedia {
            bytes: encoded.bytes,
            kept_original: encoded.kept_original,
            details: MediaDetails::Image {
                input_format,
                output_format: encoded.output_format,
                original_dimensions: encoded.original_dimensions,
                output_dimensions: encoded.output_dimensions,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_settings;
    use crate::domain::media::{Dimensions, OutputFormat};
    use crate::test_support::{encode_as, gradient_rgb};

    #[test]
    fn supports_only_decodable_images() {
        assert!(ImageCompressor.supports(&MediaType::Image(ImageFormat::Jpeg)));
        assert!(ImageCompressor.supports(&MediaType::Image(ImageFormat::Png)));
        assert!(!ImageCompressor.supports(&MediaType::Image(ImageFormat::Gif)));
        assert!(!ImageCompressor.supports(&MediaType::Pdf));
    }

    #[test]
    fn image_compressor_output_extension_follows_resolved_format() {
        let mut settings = test_settings();
        let media = MediaType::Image(ImageFormat::Png);
        assert_eq!(
            ImageCompressor.output_extension(Path::new("a/photo.PNG"), &media, &settings),
            "PNG"
        );
        settings.image.output_format = OutputFormat::Jpeg;
        assert_eq!(
            ImageCompressor.output_extension(Path::new("a/photo.png"), &media, &settings),
            "jpg"
        );
    }

    #[test]
    fn compress_returns_image_details_with_dimensions() {
        let input = encode_as(&gradient_rgb(32, 24), image::ImageFormat::Png);
        let result = ImageCompressor
            .compress(
                &input,
                &MediaType::Image(ImageFormat::Png),
                &test_settings(),
            )
            .unwrap();
        match result.details {
            MediaDetails::Image {
                input_format,
                output_format,
                original_dimensions,
                output_dimensions,
            } => {
                assert_eq!(input_format, ImageFormat::Png);
                assert_eq!(output_format, ImageFormat::Png);
                assert_eq!(
                    original_dimensions,
                    Dimensions {
                        width: 32,
                        height: 24
                    }
                );
                assert_eq!(output_dimensions, original_dimensions);
            }
            other => panic!("expected image details, got {other:?}"),
        }
    }

    #[test]
    fn compress_rejects_non_image_media_type() {
        let error = ImageCompressor
            .compress(b"x", &MediaType::Pdf, &test_settings())
            .unwrap_err();
        assert!(matches!(error, AppError::UnsupportedFormat(_)));
    }

    #[test]
    fn name_is_image() {
        assert_eq!(ImageCompressor.name(), "Image");
    }
}
