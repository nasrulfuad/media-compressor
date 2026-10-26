use std::path::Path;
use std::time::Instant;

use tracing::{debug, info};

use crate::compressor::level::CompressionLevel;
use crate::error::{AppError, AppResult};
use crate::stats::report::ImageCompressionReport;
use crate::strategy::{ImageFormat, ImageOutputFormat};

use super::format::{detect_format_from_bytes, png_effort_for_level, resolve_output_format};
use super::{jpeg, png, transform};

/// Image compression engine.
///
/// Decodes an image, applies optional transforms (resize, grayscale), re-encodes
/// to the target format, and writes the output only when the result is smaller than
/// the input (same-format) or always when a format conversion is requested.
#[derive(Debug, Clone)]
pub struct ImageCompressorEngine {
    /// Encode quality 1–100.  Defaults from the `CompressionLevel` preset.
    pub quality: u8,

    /// Target output format.  `Original` keeps the input format.
    pub output_format: ImageOutputFormat,

    /// When set, resize the image so neither dimension exceeds this value.
    pub max_dimension: Option<u32>,

    /// Convert the output to grayscale.
    pub grayscale: bool,

    /// Enable JPEG progressive scan (ignored for PNG).
    pub progressive: bool,

    /// Lossless mode: for JPEG input with `Original` format, convert to PNG
    /// instead of re-encoding as lossy JPEG.
    pub lossless: bool,

    /// Estimate size without writing any output file.
    pub dry_run: bool,

    /// Compression level used to derive PNG effort and default quality.
    pub level: CompressionLevel,
}

impl ImageCompressorEngine {
    /// Create a new engine using the given `CompressionLevel` preset.
    pub fn new(level: CompressionLevel) -> Self {
        Self {
            quality: level.image_quality(),
            output_format: ImageOutputFormat::Original,
            max_dimension: None,
            grayscale: level.convert_to_grayscale(),
            progressive: true,
            lossless: false,
            dry_run: false,
            level,
        }
    }

    /// Compress `input` and write the result to `output`.
    ///
    /// Returns an [`ImageCompressionReport`] with size and dimension statistics.
    pub fn compress(&self, input: &Path, output: &Path) -> AppResult<ImageCompressionReport> {
        let started = Instant::now();

        // ── 1. Read input ──────────────────────────────────────────────────
        let input_bytes = std::fs::read(input)?;
        let original_size = input_bytes.len() as u64;

        info!(
            path = %input.display(),
            size_bytes = original_size,
            "Loading image"
        );

        // ── 2. Detect input format ─────────────────────────────────────────
        let input_fmt = detect_format_from_bytes(&input_bytes).ok_or_else(|| {
            AppError::UnsupportedFormat(format!(
                "Cannot detect image format for '{}'",
                input.display()
            ))
        })?;

        debug!(format = ?input_fmt, "Detected input format");

        // ── 3. Decode to DynamicImage ──────────────────────────────────────
        let img = image::load_from_memory(&input_bytes)
            .map_err(|e| AppError::ImageDecode(format!("Failed to decode image: {e}")))?;

        let original_dimensions = (img.width(), img.height());

        // ── 4. Resolve output format ───────────────────────────────────────
        // In lossless mode with JPEG input and no explicit format, convert to PNG.
        let output_fmt = if self.lossless
            && matches!(self.output_format, ImageOutputFormat::Original)
            && matches!(input_fmt, ImageFormat::Jpeg)
        {
            debug!("Lossless mode: converting JPEG → PNG");
            ImageFormat::Png
        } else {
            resolve_output_format(&self.output_format, &input_fmt)
        };

        // Warn if WebP output was requested but encoder is not yet available.
        if matches!(output_fmt, ImageFormat::WebP) {
            return Err(AppError::UnsupportedFormat(
                "WebP encoding is not yet available (coming in Phase E). \
                 Use --format jpeg or --format png instead."
                    .to_string(),
            ));
        }

        // Warn if --keep-metadata is requested (not yet supported for images).
        // metadata.rs documents why this always happens.

        // ── 5. Apply transforms ────────────────────────────────────────────
        let mut img = img;

        if let Some(max_dim) = self.max_dimension {
            img = transform::resize_to_max_dimension(img, max_dim);
            debug!(
                max_dimension = max_dim,
                new_w = img.width(),
                new_h = img.height(),
                "Image resized"
            );
        }

        if self.grayscale {
            img = transform::to_grayscale(img);
            debug!("Image converted to grayscale");
        }

        let output_dimensions = (img.width(), img.height());

        // ── 6. Encode ──────────────────────────────────────────────────────
        let encoded = self.encode(&img, &output_fmt)?;

        debug!(
            encoded_bytes = encoded.len(),
            original_bytes = input_bytes.len(),
            "Encoding complete"
        );

        // ── 7. Decide what to write ────────────────────────────────────────
        let is_format_conversion = output_fmt != input_fmt;

        let (final_bytes, compressed_size): (&[u8], u64) =
            if !is_format_conversion && encoded.len() >= input_bytes.len() {
                // Re-encoded version is not smaller; keep the original bytes.
                debug!("Re-encoded not smaller — keeping original bytes");
                (&input_bytes, original_size)
            } else {
                (&encoded, encoded.len() as u64)
            };

        // ── 8. Write output (unless dry-run) ──────────────────────────────
        if !self.dry_run {
            // Create parent directories if needed.
            if let Some(parent) = output.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(output, final_bytes)?;
        }

        let duration_ms = started.elapsed().as_millis() as u64;
        let bytes_saved = original_size as i64 - compressed_size as i64;
        let ratio = if original_size > 0 {
            (1.0 - compressed_size as f64 / original_size as f64) * 100.0
        } else {
            0.0
        };

        info!(
            duration_ms,
            original_bytes = original_size,
            compressed_bytes = compressed_size,
            ratio_pct = ratio,
            "Image compression complete"
        );

        Ok(ImageCompressionReport {
            input_path: input.to_path_buf(),
            output_path: output.to_path_buf(),
            input_format: input_fmt.as_str().to_string(),
            output_format: output_fmt.as_str().to_string(),
            original_size,
            compressed_size,
            bytes_saved,
            ratio,
            original_dimensions,
            output_dimensions,
            duration_ms,
            dry_run: self.dry_run,
        })
    }

    fn encode(&self, img: &image::DynamicImage, fmt: &ImageFormat) -> AppResult<Vec<u8>> {
        match fmt {
            ImageFormat::Jpeg => {
                jpeg::encode_jpeg(img, self.quality, self.grayscale, self.progressive)
            }
            ImageFormat::Png => {
                let effort = png_effort_for_level(self.level);
                png::encode_png(img, self.grayscale, effort)
            }
            // WebP is rejected earlier; this branch is unreachable in practice.
            other => Err(AppError::UnsupportedFormat(format!(
                "{} output format is not supported",
                other.as_str()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;
    use tempfile::NamedTempFile;

    fn make_jpeg(width: u32, height: u32) -> NamedTempFile {
        let img = image::DynamicImage::new_rgb8(width, height);
        let mut f = NamedTempFile::with_suffix(".jpg").unwrap();
        img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Jpeg)
            .unwrap();
        f
    }

    fn make_png(width: u32, height: u32) -> NamedTempFile {
        let img = image::DynamicImage::new_rgb8(width, height);
        let mut f = NamedTempFile::with_suffix(".png").unwrap();
        img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Png)
            .unwrap();
        f
    }

    fn engine_medium() -> ImageCompressorEngine {
        ImageCompressorEngine::new(CompressionLevel::Medium)
    }

    #[test]
    fn compress_dry_run_does_not_write_file() {
        let input = make_jpeg(64, 64);
        let output = NamedTempFile::with_suffix(".jpg").unwrap();
        let output_path = output.path().with_extension("out.jpg");

        let mut engine = engine_medium();
        engine.dry_run = true;

        engine.compress(input.path(), &output_path).unwrap();
        assert!(!output_path.exists(), "dry-run must not write output");
    }

    #[test]
    fn compress_jpeg_writes_output_file() {
        let input = make_jpeg(64, 64);
        let output = NamedTempFile::with_suffix(".jpg").unwrap();

        engine_medium()
            .compress(input.path(), output.path())
            .unwrap();

        assert!(output.path().exists());
        let meta = std::fs::metadata(output.path()).unwrap();
        assert!(meta.len() > 0);
    }

    #[test]
    fn compress_png_writes_output_file() {
        let input = make_png(32, 32);
        let output = NamedTempFile::with_suffix(".png").unwrap();

        engine_medium()
            .compress(input.path(), output.path())
            .unwrap();

        assert!(output.path().exists());
    }

    #[test]
    fn compress_png_to_jpeg_format_conversion() {
        let input = make_png(32, 32);
        let output = NamedTempFile::with_suffix(".jpg").unwrap();

        let mut engine = engine_medium();
        engine.output_format = ImageOutputFormat::Jpeg;
        engine.compress(input.path(), output.path()).unwrap();

        let bytes = std::fs::read(output.path()).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "Output must be valid JPEG");
    }

    #[test]
    fn compress_with_max_dimension_resizes() {
        let input = make_jpeg(200, 200);
        let output = NamedTempFile::with_suffix(".jpg").unwrap();

        let mut engine = engine_medium();
        engine.max_dimension = Some(100);
        let report = engine.compress(input.path(), output.path()).unwrap();

        assert!(
            report.output_dimensions.0 <= 100 && report.output_dimensions.1 <= 100,
            "Output dimensions {:?} exceed max_dimension=100",
            report.output_dimensions
        );
    }

    #[test]
    fn compress_report_has_correct_paths() {
        let input = make_jpeg(16, 16);
        let output = NamedTempFile::with_suffix(".jpg").unwrap();

        let report = engine_medium()
            .compress(input.path(), output.path())
            .unwrap();

        assert_eq!(report.input_path, input.path());
        assert_eq!(report.output_path, output.path());
        assert_eq!(report.input_format, "JPEG");
    }

    #[test]
    fn compress_webp_output_returns_error() {
        let input = make_jpeg(16, 16);
        let output = NamedTempFile::with_suffix(".webp").unwrap();

        let mut engine = engine_medium();
        engine.output_format = ImageOutputFormat::Webp;
        let err = engine.compress(input.path(), output.path()).unwrap_err();
        assert!(matches!(err, AppError::UnsupportedFormat(_)));
    }
}
