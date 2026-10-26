use std::path::Path;

use crate::domain::media::MediaType;
use crate::domain::options::{ImageOptions, PdfOptions};
use crate::domain::report::MediaDetails;
use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionSettings {
    pub pdf: PdfOptions,
    pub image: ImageOptions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedMedia {
    pub bytes: Vec<u8>,
    pub kept_original: bool,
    pub details: MediaDetails,
}

pub trait MediaCompressor: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, media_type: &MediaType) -> bool;
    fn output_extension(
        &self,
        input_path: &Path,
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> String;
    fn compress(
        &self,
        input: &[u8],
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> AppResult<CompressedMedia>;
}
