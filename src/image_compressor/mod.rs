/// Image format detection from magic bytes and output format resolution.
pub mod format;

/// JPEG encoding via mozjpeg.
pub mod jpeg;

/// Metadata stripping notes and helpers.
pub mod metadata;

/// PNG encoding via oxipng.
pub mod png;

/// Geometric and colour-space transforms (resize, grayscale).
pub mod transform;

/// Image compression engine — orchestrates the full pipeline.
pub mod engine;

pub use engine::ImageCompressorEngine;

use std::path::Path;

use crate::error::AppResult;
use crate::stats::report::ImageCompressionReport;
use crate::strategy::{CompressorStrategy, FileTypeResolver, MediaType};

impl CompressorStrategy for ImageCompressorEngine {
    /// No separate config — the engine carries its own configuration fields.
    type Config = ();

    type Report = ImageCompressionReport;

    fn can_handle(file_path: &Path) -> AppResult<bool>
    where
        Self: Sized,
    {
        FileTypeResolver::detect(file_path).map(|t| matches!(t, MediaType::Image(_)))
    }

    fn compress(&self, input: &Path, output: &Path) -> AppResult<Self::Report> {
        // Delegate via UFCS to avoid ambiguity with the inherent method.
        ImageCompressorEngine::compress(self, input, output)
    }

    fn name(&self) -> &'static str {
        "Image"
    }
}
