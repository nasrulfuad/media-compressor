use std::path::Path;
use std::time::Instant;
use tracing::{debug, info};

use crate::compressor::cleanup::remove_unnecessary_objects;
use crate::compressor::dedup::deduplicate_streams;
use crate::compressor::font::{deduplicate_fonts, subset_fonts};
use crate::compressor::image::{build_page_dpi_map, compress_images};
use crate::compressor::inline::convert_inline_images;
use crate::compressor::level::CompressionLevel;
use crate::compressor::metadata::strip_metadata;
use crate::compressor::stream::{compress_streams, merge_page_content_streams};
use crate::error::AppResult;
use crate::io::{reader::load_pdf, writer::save_pdf};
use crate::stats::report::CompressionReport;
use crate::strategy::{CompressorStrategy, FileTypeResolver, MediaType};

// Phase 2.5.9 note: Incremental updates are already handled by lopdf's
// `Document::load()` which collapses all incremental update sections into
// a single unified object table during parsing. No additional step is needed.

// Phase 2.5.3 note: `Document::compress()` and `delete_unused_objects()` are
// not part of lopdf 0.34's public API and are therefore not called here.
// We rely on `doc.prune_objects()` + `doc.renumber_objects()` instead.

/// Core compression engine.
///
/// Create one engine per desired configuration, then call [`CompressorEngine::compress`] for
/// each input file.
#[derive(Debug, Clone)]
pub struct CompressorEngine {
    /// The compression level preset.
    pub level: CompressionLevel,

    /// When `true`, document metadata is preserved regardless of the level
    /// preset's default behaviour.
    pub keep_metadata: bool,

    /// Override the image quality (0–100). When `None`, the level preset is used.
    pub image_quality: Option<u8>,

    /// Override the target DPI for image downsampling. When `None`, the level
    /// preset is used.
    pub target_dpi: Option<u32>,

    /// When `true`, the engine estimates compression results without writing
    /// any output file.
    pub dry_run: bool,

    /// When `true`, font deduplication and subsetting are performed.
    /// Defaults to `false` — font compression is opt-in.
    pub compress_fonts: bool,
}

impl CompressorEngine {
    /// Create a new engine with the given compression level and sensible defaults.
    pub fn new(level: CompressionLevel) -> Self {
        Self {
            level,
            keep_metadata: false,
            image_quality: None,
            target_dpi: None,
            dry_run: false,
            compress_fonts: false,
        }
    }

    /// Compress `input` and write the result to `output`.
    ///
    /// When `dry_run` is enabled, `output` is never created; the reported
    /// compressed size is an estimate based solely on stream savings.
    pub fn compress(&self, input: &Path, output: &Path) -> AppResult<CompressionReport> {
        let started_at = Instant::now();

        // ── 1. Original file size ──────────────────────────────────────────
        let original_size = std::fs::metadata(input)?.len();
        info!(
            path = %input.display(),
            size_bytes = original_size,
            "Loading PDF"
        );

        // ── 2. Load the document ───────────────────────────────────────────
        let mut doc = load_pdf(input)?;
        debug!("PDF loaded successfully");

        // ── 3. Remove unnecessary objects (thumbnails, JS, annotations, …) ─
        let cleanup_bytes = remove_unnecessary_objects(&mut doc, self.level)?;
        debug!(bytes_freed = cleanup_bytes, "Unnecessary objects removed");

        // ── 4. Deduplicate identical streams ──────────────────────────────
        let dedup_bytes = deduplicate_streams(&mut doc)?;
        debug!(bytes_saved = dedup_bytes, "Streams deduplicated");

        // ── 5. Stream compression ──────────────────────────────────────────
        let stream_level = self.level.stream_compression_level();
        let stream_bytes_saved = compress_streams(&mut doc, stream_level)?;
        debug!(bytes_saved = stream_bytes_saved, "Streams compressed");

        // ── 6. Build page→image DPI context ───────────────────────────────
        let page_dpi_map = build_page_dpi_map(&doc);
        debug!(image_count = page_dpi_map.len(), "Page DPI map built");

        // ── 7. Image compression ───────────────────────────────────────────
        let quality = self
            .image_quality
            .unwrap_or_else(|| self.level.image_quality());
        let target_dpi = self.target_dpi.or_else(|| self.level.target_dpi());
        let image_bytes_saved =
            compress_images(&mut doc, quality, target_dpi, self.level, &page_dpi_map)?;
        debug!(bytes_saved = image_bytes_saved, "Images compressed");

        // ── 8. Merge per-page content streams (2.5.5) ──────────────────────
        let merge_bytes = merge_page_content_streams(&mut doc)?;
        debug!(bytes_saved = merge_bytes, "Content streams merged");

        // ── 9. Font deduplication (2.5.7) ──────────────────────────────────
        let font_bytes = if self.compress_fonts {
            let b = deduplicate_fonts(&mut doc)?;
            debug!(bytes_saved = b, "Font streams deduplicated");
            b
        } else {
            debug!("Font deduplication skipped (--compress-fonts not set)");
            0
        };

        // ── 9b. Font subsetting (2.5.7) ────────────────────────────────────
        let subset_bytes = if self.compress_fonts {
            let b = subset_fonts(&mut doc)?;
            debug!(bytes_saved = b, "Fonts subsetted");
            b
        } else {
            debug!("Font subsetting skipped (--compress-fonts not set)");
            0
        };

        // ── 10. Inline image conversion (2.5.8) ────────────────────────────
        let inline_count = convert_inline_images(&mut doc, quality, self.level)?;
        debug!(
            converted = inline_count,
            "Inline images converted to XObjects"
        );

        // ── 11. Metadata stripping ─────────────────────────────────────────
        if !self.keep_metadata && self.level.strip_metadata() {
            let meta_bytes_removed = strip_metadata(&mut doc);
            debug!(bytes_removed = meta_bytes_removed, "Metadata stripped");
        }

        // ── 12. Prune unreferenced objects and compact object numbering ─────
        let pruned = doc.prune_objects();
        debug!(count = pruned.len(), "Pruned unreferenced objects");
        doc.renumber_objects();
        debug!("Objects renumbered");

        // ── 13. Write output (unless dry-run) ──────────────────────────────
        let compressed_size = if self.dry_run {
            // Estimate: subtract the bytes we know we saved across all passes.
            let total_saved = stream_bytes_saved as i64
                + image_bytes_saved
                + cleanup_bytes as i64
                + dedup_bytes as i64
                + merge_bytes as i64
                + font_bytes as i64
                + subset_bytes as i64;
            if total_saved > 0 {
                original_size.saturating_sub(total_saved as u64)
            } else {
                original_size
            }
        } else {
            save_pdf(&mut doc, output)?;
            std::fs::metadata(output)?.len()
        };

        let duration_ms = started_at.elapsed().as_millis() as u64;

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
            "Compression complete"
        );

        Ok(CompressionReport {
            input_path: input.to_path_buf(),
            output_path: output.to_path_buf(),
            original_size,
            compressed_size,
            bytes_saved,
            ratio,
            duration_ms,
            dry_run: self.dry_run,
        })
    }
}

impl Default for CompressorEngine {
    fn default() -> Self {
        Self::new(CompressionLevel::default())
    }
}

impl CompressorStrategy for CompressorEngine {
    /// No separate config — the engine carries its own configuration fields.
    type Config = ();

    type Report = CompressionReport;

    fn can_handle(file_path: &Path) -> AppResult<bool> {
        FileTypeResolver::detect(file_path).map(|t| matches!(t, MediaType::Pdf))
    }

    fn compress(&self, input: &Path, output: &Path) -> AppResult<Self::Report> {
        // Delegate to the inherent method via UFCS to avoid infinite recursion.
        CompressorEngine::compress(self, input, output)
    }

    fn name(&self) -> &'static str {
        "PDF"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_new_sets_correct_defaults() {
        let engine = CompressorEngine::new(CompressionLevel::High);
        assert_eq!(engine.level, CompressionLevel::High);
        assert!(!engine.keep_metadata);
        assert!(engine.image_quality.is_none());
        assert!(engine.target_dpi.is_none());
        assert!(!engine.dry_run);
        assert!(!engine.compress_fonts);
    }

    #[test]
    fn engine_default_uses_medium_level() {
        let engine = CompressorEngine::default();
        assert_eq!(engine.level, CompressionLevel::Medium);
    }
}
