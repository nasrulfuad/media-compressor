use crate::domain::media::MediaType;
use crate::error::{AppError, AppResult};

use super::image_compressor::ImageCompressor;
use super::media_compressor::MediaCompressor;
use super::pdf_compressor::PdfCompressor;

const HEX_PREVIEW_BYTES: usize = 8;

pub struct CompressorRegistry {
    compressors: Vec<Box<dyn MediaCompressor>>,
}

fn hex_preview(leading_bytes: &[u8]) -> String {
    leading_bytes
        .iter()
        .take(HEX_PREVIEW_BYTES)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn unsupported_media_error(media_type: &MediaType) -> AppError {
    match media_type {
        MediaType::Unknown { leading_bytes } => AppError::UnsupportedFormat(format!(
            "unrecognised file type (magic bytes: {})",
            hex_preview(leading_bytes)
        )),
        MediaType::Image(format) => AppError::UnsupportedFormat(format.unsupported_input_message()),
        MediaType::Pdf => {
            AppError::UnsupportedFormat("no compressor registered for PDF".to_string())
        }
    }
}

impl CompressorRegistry {
    pub fn new(compressors: Vec<Box<dyn MediaCompressor>>) -> Self {
        Self { compressors }
    }

    pub fn standard() -> Self {
        Self::new(vec![Box::new(PdfCompressor), Box::new(ImageCompressor)])
    }

    pub fn find(&self, media_type: &MediaType) -> AppResult<&dyn MediaCompressor> {
        self.compressors
            .iter()
            .map(|compressor| compressor.as_ref())
            .find(|compressor| compressor.supports(media_type))
            .ok_or_else(|| unsupported_media_error(media_type))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::app::media_compressor::{CompressedMedia, CompressionSettings};
    use crate::domain::media::ImageFormat;
    use crate::domain::report::MediaDetails;

    struct FakeCompressor {
        label: &'static str,
    }

    impl MediaCompressor for FakeCompressor {
        fn name(&self) -> &'static str {
            self.label
        }

        fn supports(&self, _media_type: &MediaType) -> bool {
            true
        }

        fn output_extension(&self, _: &Path, _: &MediaType, _: &CompressionSettings) -> String {
            "fake".to_string()
        }

        fn compress(
            &self,
            input: &[u8],
            _: &MediaType,
            _: &CompressionSettings,
        ) -> AppResult<CompressedMedia> {
            Ok(CompressedMedia {
                bytes: input.to_vec(),
                kept_original: true,
                details: MediaDetails::Pdf { passes: Vec::new() },
            })
        }
    }

    fn name_for(media_type: &MediaType) -> &'static str {
        CompressorRegistry::standard()
            .find(media_type)
            .unwrap()
            .name()
    }

    #[test]
    fn registry_finds_pdf_compressor_for_pdf() {
        assert_eq!(name_for(&MediaType::Pdf), "PDF");
    }

    #[test]
    fn registry_finds_image_compressor_for_jpeg_and_png() {
        assert_eq!(name_for(&MediaType::Image(ImageFormat::Jpeg)), "Image");
        assert_eq!(name_for(&MediaType::Image(ImageFormat::Png)), "Image");
    }

    #[test]
    fn registry_rejects_unknown_media_with_hex_bytes_in_message() {
        let media = MediaType::Unknown {
            leading_bytes: vec![0xDE, 0xAD, 0xBE, 0xEF],
        };
        let error = CompressorRegistry::standard().find(&media).err().unwrap();
        assert_eq!(
            error.to_string(),
            "unsupported file type: unrecognised file type (magic bytes: de ad be ef)"
        );
    }

    #[test]
    fn registry_truncates_hex_preview_to_eight_bytes() {
        let media = MediaType::Unknown {
            leading_bytes: (0u8..16).collect(),
        };
        let message = CompressorRegistry::standard()
            .find(&media)
            .err()
            .unwrap()
            .to_string();
        assert!(
            message.ends_with("(magic bytes: 00 01 02 03 04 05 06 07)"),
            "{message}"
        );
    }

    #[test]
    fn registry_rejects_undecodable_image_formats() {
        for format in [
            ImageFormat::Gif,
            ImageFormat::Tiff,
            ImageFormat::Bmp,
            ImageFormat::WebP,
        ] {
            let error = CompressorRegistry::standard()
                .find(&MediaType::Image(format))
                .err()
                .unwrap();
            assert_eq!(
                error.to_string(),
                format!(
                    "unsupported file type: {}",
                    format.unsupported_input_message()
                )
            );
        }
    }

    #[test]
    fn registry_uses_first_matching_compressor() {
        let registry = CompressorRegistry::new(vec![
            Box::new(FakeCompressor { label: "first" }),
            Box::new(FakeCompressor { label: "second" }),
        ]);
        assert_eq!(registry.find(&MediaType::Pdf).unwrap().name(), "first");
    }

    #[test]
    fn empty_registry_rejects_everything() {
        let registry = CompressorRegistry::new(Vec::new());
        assert!(registry.find(&MediaType::Pdf).is_err());
    }
}
