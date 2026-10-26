use std::path::PathBuf;

use crate::domain::options::{FailurePolicy, WriteMode};

pub mod batch;
pub mod compress_file;
pub mod image_compressor;
pub mod info;
pub mod media_compressor;
pub mod output_path;
pub mod pdf_compressor;
pub mod progress;
pub mod registry;

pub use media_compressor::CompressionSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressRequest {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub settings: CompressionSettings,
    pub write_mode: WriteMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRequest {
    pub input_dir: PathBuf,
    pub output_dir: Option<PathBuf>,
    pub pattern: String,
    pub recursive: bool,
    pub workers: usize,
    pub failure_policy: FailurePolicy,
    pub settings: CompressionSettings,
    pub write_mode: WriteMode,
}

#[cfg(test)]
pub(crate) fn test_settings() -> CompressionSettings {
    use crate::domain::level::CompressionLevel;
    use crate::domain::options::{ImageOptions, PdfOptions};

    CompressionSettings {
        pdf: PdfOptions::for_level(CompressionLevel::Medium),
        image: ImageOptions::for_level(CompressionLevel::Medium),
    }
}
