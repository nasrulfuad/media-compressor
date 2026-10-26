use std::path::Path;

use crate::domain::media::MediaType;
use crate::domain::report::MediaDetails;
use crate::error::AppResult;
use crate::pdf::engine::compress_pdf_bytes;

use super::media_compressor::{CompressedMedia, CompressionSettings, MediaCompressor};

const DEFAULT_PDF_EXTENSION: &str = "pdf";

pub struct PdfCompressor;

impl MediaCompressor for PdfCompressor {
    fn name(&self) -> &'static str {
        "PDF"
    }

    fn supports(&self, media_type: &MediaType) -> bool {
        matches!(media_type, MediaType::Pdf)
    }

    fn output_extension(
        &self,
        input_path: &Path,
        _media_type: &MediaType,
        _settings: &CompressionSettings,
    ) -> String {
        input_path
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned())
            .unwrap_or_else(|| DEFAULT_PDF_EXTENSION.to_string())
    }

    fn compress(
        &self,
        input: &[u8],
        _media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> AppResult<CompressedMedia> {
        let compressed = compress_pdf_bytes(input, &settings.pdf)?;
        Ok(CompressedMedia {
            bytes: compressed.bytes,
            kept_original: compressed.kept_original,
            details: MediaDetails::Pdf {
                passes: compressed.passes,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_settings;
    use crate::domain::media::ImageFormat;
    use crate::test_support::PdfFixture;

    #[test]
    fn supports_only_pdf_media() {
        assert!(PdfCompressor.supports(&MediaType::Pdf));
        assert!(!PdfCompressor.supports(&MediaType::Image(ImageFormat::Jpeg)));
        assert!(!PdfCompressor.supports(&MediaType::Unknown {
            leading_bytes: vec![]
        }));
    }

    #[test]
    fn pdf_compressor_output_extension_keeps_input_extension() {
        let extension = PdfCompressor.output_extension(
            Path::new("a/Doc.PDF"),
            &MediaType::Pdf,
            &test_settings(),
        );
        assert_eq!(extension, "PDF");
    }

    #[test]
    fn pdf_compressor_output_extension_defaults_to_pdf_without_extension() {
        let extension =
            PdfCompressor.output_extension(Path::new("scan"), &MediaType::Pdf, &test_settings());
        assert_eq!(extension, "pdf");
    }

    #[test]
    fn compress_returns_pdf_details_and_valid_bytes() {
        let input = PdfFixture::new()
            .page_with_content(&b"BT   (Hello)   Tj   ET   ".repeat(50))
            .into_bytes();
        let result = PdfCompressor
            .compress(&input, &MediaType::Pdf, &test_settings())
            .unwrap();
        assert!(matches!(result.details, MediaDetails::Pdf { ref passes } if !passes.is_empty()));
        assert!(result.bytes.len() < input.len());
        assert!(!result.kept_original);
    }

    #[test]
    fn compress_propagates_invalid_pdf_error() {
        let error = PdfCompressor
            .compress(b"%PDF-1.4 broken", &MediaType::Pdf, &test_settings())
            .unwrap_err();
        assert!(matches!(
            error,
            crate::error::AppError::Lopdf(_) | crate::error::AppError::InvalidPdf(_)
        ));
    }

    #[test]
    fn name_is_pdf() {
        assert_eq!(PdfCompressor.name(), "PDF");
    }
}
