use lopdf::Document;
use tracing::{debug, info};

use crate::domain::options::PdfOptions;
use crate::domain::report::PassStat;
use crate::error::{AppError, AppResult};

use super::pipeline::standard_pipeline;

const PDF_MAGIC: &[u8] = b"%PDF-";

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedPdf {
    pub bytes: Vec<u8>,
    pub passes: Vec<PassStat>,
    pub kept_original: bool,
}

fn map_load_error(error: lopdf::Error) -> AppError {
    match &error {
        lopdf::Error::Type => {
            AppError::InvalidPdf("unexpected object type while parsing".to_string())
        }
        lopdf::Error::ContentDecode => {
            AppError::InvalidPdf("content stream could not be decoded".to_string())
        }
        other if other.to_string().to_lowercase().contains("encrypt") => AppError::EncryptedPdf,
        _ => AppError::Lopdf(error),
    }
}

pub fn parse_pdf(bytes: &[u8]) -> AppResult<Document> {
    if !bytes.starts_with(PDF_MAGIC) {
        return Err(AppError::InvalidPdf(
            "file does not appear to be a valid PDF (missing %PDF- header)".to_string(),
        ));
    }
    let document = Document::load_mem(bytes).map_err(map_load_error)?;
    if document.is_encrypted() {
        return Err(AppError::EncryptedPdf);
    }
    Ok(document)
}

fn serialize(document: &mut Document) -> AppResult<Vec<u8>> {
    let mut output = Vec::new();
    document.save_to(&mut output)?;
    Ok(output)
}

fn run_passes(document: &mut Document, options: &PdfOptions) -> AppResult<Vec<PassStat>> {
    let mut stats = Vec::new();
    for pass in standard_pipeline() {
        if !pass.is_enabled(options) {
            debug!(pass = pass.name(), "Pass disabled");
            continue;
        }
        let bytes_saved = pass.run(document, options)?;
        debug!(pass = pass.name(), bytes_saved, "Pass finished");
        stats.push(PassStat {
            pass: pass.name(),
            bytes_saved,
        });
    }
    Ok(stats)
}

pub fn compress_pdf_bytes(input: &[u8], options: &PdfOptions) -> AppResult<CompressedPdf> {
    let mut document = parse_pdf(input)?;
    let passes = run_passes(&mut document, options)?;
    let pruned = document.prune_objects();
    debug!(count = pruned.len(), "Pruned unreferenced objects");
    document.renumber_objects();
    let output = serialize(&mut document)?;
    if output.len() >= input.len() {
        info!(
            input_bytes = input.len(),
            output_bytes = output.len(),
            "Compressed PDF is not smaller; keeping original bytes"
        );
        return Ok(CompressedPdf {
            bytes: input.to_vec(),
            passes,
            kept_original: true,
        });
    }
    Ok(CompressedPdf {
        bytes: output,
        passes,
        kept_original: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::{decoded_page_content, FixtureImage, PdfFixture};

    fn options(level: CompressionLevel) -> PdfOptions {
        PdfOptions::for_level(level)
    }

    fn verbose_content() -> Vec<u8> {
        b"BT   /F1   12   Tf   (Discount 50%)   Tj   ET   % trailing comment\n".repeat(40)
    }

    fn redundant_pdf() -> Vec<u8> {
        PdfFixture::new()
            .page_with_content(&verbose_content())
            .with_image(FixtureImage::FlateRgb {
                width: 128,
                height: 128,
            })
            .with_info_and_xmp_metadata()
            .into_bytes()
    }

    #[test]
    fn compress_pdf_bytes_shrinks_redundant_pdf_and_preserves_pages() {
        let input = redundant_pdf();
        let result = compress_pdf_bytes(&input, &options(CompressionLevel::High)).unwrap();
        assert!(!result.kept_original);
        assert!(result.bytes.len() < input.len());
        let reloaded = Document::load_mem(&result.bytes).unwrap();
        assert_eq!(reloaded.get_pages().len(), 1);
        assert!(decoded_page_content(&reloaded, 1).starts_with(b"BT/F1 12 Tf(Discount 50%)Tj ET"));
    }

    #[test]
    fn compress_pdf_bytes_never_returns_larger_output() {
        let input = PdfFixture::new().page_with_content(b"q Q").into_bytes();
        let result = compress_pdf_bytes(&input, &options(CompressionLevel::Low)).unwrap();
        assert!(result.kept_original);
        assert_eq!(result.bytes, input);
    }

    #[test]
    fn compress_pdf_bytes_runs_passes_in_standard_order() {
        let result =
            compress_pdf_bytes(&redundant_pdf(), &options(CompressionLevel::High)).unwrap();
        let names: Vec<&str> = result.passes.iter().map(|stat| stat.pass).collect();
        assert_eq!(
            names,
            vec![
                "cleanup",
                "inline_images",
                "dedup",
                "streams",
                "images",
                "merge_content",
                "metadata"
            ]
        );
    }

    #[test]
    fn compress_pdf_bytes_at_high_level_strips_info_dictionary() {
        let result =
            compress_pdf_bytes(&redundant_pdf(), &options(CompressionLevel::High)).unwrap();
        let reloaded = Document::load_mem(&result.bytes).unwrap();
        assert!(!reloaded.trailer.has(b"Info"));
    }

    #[test]
    fn compress_pdf_bytes_at_medium_level_keeps_info_dictionary() {
        let result =
            compress_pdf_bytes(&redundant_pdf(), &options(CompressionLevel::Medium)).unwrap();
        let reloaded = Document::load_mem(&result.bytes).unwrap();
        assert!(reloaded.trailer.has(b"Info"));
    }

    #[test]
    fn compress_pdf_bytes_converts_inline_images_and_compresses_them() {
        let content = b"q 128 0 0 128 0 0 cm BI /W 128 /H 128 /CS /RGB /BPC 8 ID ";
        let mut stream = content.to_vec();
        stream.extend((0..128 * 128 * 3).map(|index| ((index / 3) % 256) as u8));
        stream.extend_from_slice(b"\nEI Q");
        let input = PdfFixture::new().page_with_content(&stream).into_bytes();
        let result = compress_pdf_bytes(&input, &options(CompressionLevel::High)).unwrap();
        assert!(!result.kept_original);
        let reloaded = Document::load_mem(&result.bytes).unwrap();
        let image_count = reloaded
            .objects
            .values()
            .filter(|object| matches!(object, lopdf::Object::Stream(s) if crate::pdf::object_graph::is_image_stream_dict(&s.dict)))
            .count();
        assert_eq!(image_count, 1);
        assert!(result.bytes.len() < input.len() / 2);
        assert!(decoded_page_content(&reloaded, 1).ends_with(b"/ImInl1 Do Q"));
    }

    #[test]
    fn parse_pdf_rejects_missing_magic() {
        let error = parse_pdf(b"This is not a PDF file at all").unwrap_err();
        assert!(matches!(error, AppError::InvalidPdf(_)));
    }

    #[test]
    fn parse_pdf_rejects_truncated_magic() {
        assert!(matches!(
            parse_pdf(b"%PDF").unwrap_err(),
            AppError::InvalidPdf(_)
        ));
    }

    #[test]
    fn parse_pdf_rejects_corrupt_body_after_valid_magic() {
        assert!(parse_pdf(b"%PDF-1.7\nnot really a pdf").is_err());
    }

    #[test]
    fn compress_pdf_bytes_is_deterministic() {
        let input = redundant_pdf();
        let first = compress_pdf_bytes(&input, &options(CompressionLevel::Medium)).unwrap();
        let second = compress_pdf_bytes(&input, &options(CompressionLevel::Medium)).unwrap();
        assert_eq!(first.bytes.len(), second.bytes.len());
    }
}
