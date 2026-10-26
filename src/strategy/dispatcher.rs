use std::path::Path;

use crate::compressor::engine::CompressorEngine;
use crate::compressor::level::CompressionLevel;
use crate::error::{AppError, AppResult};
use crate::image_compressor::ImageCompressorEngine;
use crate::stats::report::{CompressionReport, ImageCompressionReport};

use super::{FileTypeResolver, ImageOutputFormat, MediaType};

/// All configuration options for a single compression operation.
///
/// Built from CLI args in `main.rs` and passed to [`CompressorDispatcher::compress`].
#[derive(Debug, Clone)]
pub struct DispatchConfig {
    // ── Shared ──────────────────────────────────────────────────────────────
    pub level: CompressionLevel,
    pub dry_run: bool,
    pub keep_metadata: bool,

    // ── Image-specific ───────────────────────────────────────────────────────
    /// General quality override (1-100).  Takes precedence over the level preset.
    pub quality: Option<u8>,
    /// Target output format for images.
    pub format: ImageOutputFormat,
    /// Resize so neither dimension exceeds this value.
    pub max_dimension: Option<u32>,
    /// Convert output to grayscale.
    pub grayscale: bool,
    /// Explicitly request metadata stripping (images always strip via re-encode).
    pub strip_metadata: bool,
    /// Lossless mode (for images: JPEG→PNG; for PDF: skip lossy recompress).
    pub lossless: bool,

    // ── PDF-specific ─────────────────────────────────────────────────────────
    /// Override JPEG quality for images embedded inside PDF pages.
    pub pdf_image_quality: Option<u8>,
    /// Override target DPI for image downsampling inside PDFs.
    pub pdf_target_dpi: Option<u32>,
    /// When `true`, run font deduplication and subsetting on PDF files.
    pub pdf_compress_fonts: bool,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        Self {
            level: CompressionLevel::default(),
            dry_run: false,
            keep_metadata: false,
            quality: None,
            format: ImageOutputFormat::Original,
            max_dimension: None,
            grayscale: false,
            strip_metadata: false,
            lossless: false,
            pdf_image_quality: None,
            pdf_target_dpi: None,
            pdf_compress_fonts: false,
        }
    }
}

/// Result of a compression operation, holding the report for either file type.
#[derive(Debug)]
pub enum CompressResult {
    /// The input was a PDF and was handled by `CompressorEngine`.
    Pdf(CompressionReport),
    /// The input was a raster image and was handled by `ImageCompressorEngine`.
    Image(ImageCompressionReport),
}

impl CompressResult {
    /// Print a coloured human-readable summary to stdout.
    pub fn print_pretty(&self) {
        match self {
            CompressResult::Pdf(r) => r.print_pretty(),
            CompressResult::Image(r) => r.print_pretty(),
        }
    }

    /// Serialise the report to a JSON string.
    pub fn to_json(&self) -> AppResult<String> {
        match self {
            CompressResult::Pdf(r) => r.to_json(),
            CompressResult::Image(r) => r.to_json(),
        }
    }
}

/// Route a file to the correct compression strategy based on its magic bytes.
pub struct CompressorDispatcher;

impl CompressorDispatcher {
    /// Detect the file type of `input` and compress it to `output`.
    ///
    /// Builds the appropriate engine from `config`, dispatches based on the
    /// detected [`MediaType`], and returns a [`CompressResult`].
    pub fn compress(
        &self,
        config: &DispatchConfig,
        input: &Path,
        output: &Path,
    ) -> AppResult<CompressResult> {
        match FileTypeResolver::detect(input)? {
            MediaType::Pdf => {
                let mut engine = CompressorEngine::new(config.level);
                engine.keep_metadata = config.keep_metadata;
                engine.image_quality = config.pdf_image_quality;
                engine.target_dpi = config.pdf_target_dpi;
                engine.dry_run = config.dry_run;
                engine.compress_fonts = config.pdf_compress_fonts;
                let report = engine.compress(input, output)?;
                Ok(CompressResult::Pdf(report))
            }
            MediaType::Image(_) => {
                let mut engine = ImageCompressorEngine::new(config.level);
                engine.output_format = config.format.clone();
                engine.max_dimension = config.max_dimension;
                engine.grayscale = config.grayscale;
                engine.lossless = config.lossless;
                engine.dry_run = config.dry_run;
                // Quality: explicit override takes priority over level preset.
                if let Some(q) = config.quality {
                    engine.quality = q;
                }
                let report = engine.compress(input, output)?;
                Ok(CompressResult::Image(report))
            }
            MediaType::Unknown(bytes) => Err(AppError::UnsupportedFormat(format!(
                "Unrecognised file type (magic bytes: {})",
                bytes
                    .iter()
                    .take(8)
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn dispatcher() -> CompressorDispatcher {
        CompressorDispatcher
    }

    fn config() -> DispatchConfig {
        DispatchConfig::default()
    }

    #[test]
    fn dispatcher_rejects_unknown_file() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        let out = NamedTempFile::new().unwrap();
        let err = dispatcher()
            .compress(&config(), f.path(), out.path())
            .unwrap_err();
        assert!(matches!(err, AppError::UnsupportedFormat(_)));
    }

    #[test]
    fn dispatcher_compresses_jpeg_file() {
        use std::io::BufWriter;
        let img = image::DynamicImage::new_rgb8(16, 16);
        let mut f = NamedTempFile::with_suffix(".jpg").unwrap();
        img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Jpeg)
            .unwrap();
        let out = NamedTempFile::with_suffix(".jpg").unwrap();

        let result = dispatcher()
            .compress(&config(), f.path(), out.path())
            .unwrap();
        assert!(matches!(result, CompressResult::Image(_)));
    }

    #[test]
    fn dispatcher_compresses_png_file() {
        use std::io::BufWriter;
        let img = image::DynamicImage::new_rgb8(16, 16);
        let mut f = NamedTempFile::with_suffix(".png").unwrap();
        img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Png)
            .unwrap();
        let out = NamedTempFile::with_suffix(".png").unwrap();

        let result = dispatcher()
            .compress(&config(), f.path(), out.path())
            .unwrap();
        assert!(matches!(result, CompressResult::Image(_)));
    }
}
