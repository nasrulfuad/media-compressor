/// File-type resolver — magic bytes detection.
pub mod resolver;

/// Compressor dispatcher — route files to the correct strategy.
pub mod dispatcher;

pub use dispatcher::{CompressResult, CompressorDispatcher, DispatchConfig};
pub use resolver::FileTypeResolver;

use std::path::Path;

use crate::error::AppResult;

/// Contract that every compressor strategy must satisfy.
///
/// Each file type (PDF, image, …) implements this trait. The dispatcher detects
/// the file type via magic bytes and calls the matching strategy.
pub trait CompressorStrategy {
    /// Configuration type specific to this strategy.
    type Config;

    /// Report type returned after a successful compression.
    type Report;

    /// Return `true` if this strategy can handle the file at `file_path`.
    ///
    /// Detection is based on magic bytes, not the file extension.
    fn can_handle(file_path: &Path) -> AppResult<bool>
    where
        Self: Sized;

    /// Compress `input` and write the result to `output`.
    fn compress(&self, input: &Path, output: &Path) -> AppResult<Self::Report>;

    /// Human-readable name of this strategy (e.g. `"PDF"`, `"Image"`).
    fn name(&self) -> &'static str;
}

/// Target output format for image compression.
#[derive(Debug, Clone, PartialEq, Default, clap::ValueEnum)]
pub enum ImageOutputFormat {
    /// Keep the same format as the input.
    #[default]
    Original,
    /// Encode as JPEG (lossy).
    Jpeg,
    /// Encode as PNG (lossless).
    Png,
    /// Encode as WebP. Note: WebP encoding arrives in Phase E; selecting this
    /// before then returns an `UnsupportedFormat` error at compression time.
    Webp,
}

impl ImageOutputFormat {
    /// File extension for this output format. Returns `None` for `Original`.
    pub fn extension(&self) -> Option<&'static str> {
        match self {
            ImageOutputFormat::Original => None,
            ImageOutputFormat::Jpeg => Some("jpg"),
            ImageOutputFormat::Png => Some("png"),
            ImageOutputFormat::Webp => Some("webp"),
        }
    }
}

/// Detected media type, resolved from magic bytes.
#[derive(Debug, PartialEq)]
pub enum MediaType {
    /// PDF document.
    Pdf,
    /// Raster image in a known format.
    Image(ImageFormat),
    /// Unknown — contains the first ≤16 bytes for error reporting.
    Unknown(Vec<u8>),
}

/// Supported image formats (detected from magic bytes).
#[derive(Debug, Clone, PartialEq)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Tiff,
    Bmp,
    Gif,
}

impl ImageFormat {
    /// Human-readable name used in error messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "JPEG",
            ImageFormat::Png => "PNG",
            ImageFormat::WebP => "WebP",
            ImageFormat::Tiff => "TIFF",
            ImageFormat::Bmp => "BMP",
            ImageFormat::Gif => "GIF",
        }
    }
}
