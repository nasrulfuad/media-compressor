use super::level::CompressionLevel;
use super::media::OutputFormat;

pub const DEFAULT_MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;
pub const DEFAULT_MAX_IMAGE_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_stream_bytes: usize,
    pub max_image_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_stream_bytes: DEFAULT_MAX_STREAM_BYTES,
            max_image_pixels: DEFAULT_MAX_IMAGE_PIXELS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Preserve,
    Grayscale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncodingMode {
    Lossy,
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    FollowLevel,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontOptimization {
    Disabled,
    Subset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Write,
    Overwrite,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicy {
    StopOnFirstError,
    ContinueOnError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOptions {
    pub level: CompressionLevel,
    pub jpeg_quality: u8,
    pub target_dpi: Option<u32>,
    pub color_mode: ColorMode,
    pub image_encoding: ImageEncodingMode,
    pub metadata: MetadataPolicy,
    pub fonts: FontOptimization,
    pub limits: Limits,
}

impl PdfOptions {
    pub fn for_level(level: CompressionLevel) -> Self {
        let preset = level.preset();
        Self {
            level,
            jpeg_quality: preset.jpeg_quality,
            target_dpi: preset.target_dpi,
            color_mode: color_mode_for(preset.grayscale),
            image_encoding: ImageEncodingMode::Lossy,
            metadata: MetadataPolicy::FollowLevel,
            fonts: FontOptimization::Disabled,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOptions {
    pub level: CompressionLevel,
    pub quality: u8,
    pub output_format: OutputFormat,
    pub max_dimension: Option<u32>,
    pub color_mode: ColorMode,
    pub encoding: ImageEncodingMode,
    pub metadata: MetadataPolicy,
    pub limits: Limits,
}

impl ImageOptions {
    pub fn for_level(level: CompressionLevel) -> Self {
        let preset = level.preset();
        Self {
            level,
            quality: preset.jpeg_quality,
            output_format: OutputFormat::Original,
            max_dimension: None,
            color_mode: color_mode_for(preset.grayscale),
            encoding: ImageEncodingMode::Lossy,
            metadata: MetadataPolicy::FollowLevel,
            limits: Limits::default(),
        }
    }
}

fn color_mode_for(grayscale: bool) -> ColorMode {
    if grayscale {
        ColorMode::Grayscale
    } else {
        ColorMode::Preserve
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_options_for_level_follow_preset() {
        let options = PdfOptions::for_level(CompressionLevel::High);
        assert_eq!(options.jpeg_quality, 60);
        assert_eq!(options.target_dpi, Some(150));
        assert_eq!(options.color_mode, ColorMode::Preserve);
        assert_eq!(options.metadata, MetadataPolicy::FollowLevel);
        assert_eq!(options.fonts, FontOptimization::Disabled);
        assert_eq!(options.image_encoding, ImageEncodingMode::Lossy);
    }

    #[test]
    fn extreme_level_selects_grayscale_for_pdf_and_image() {
        assert_eq!(
            PdfOptions::for_level(CompressionLevel::Extreme).color_mode,
            ColorMode::Grayscale
        );
        assert_eq!(
            ImageOptions::for_level(CompressionLevel::Extreme).color_mode,
            ColorMode::Grayscale
        );
    }

    #[test]
    fn image_options_for_level_follow_preset() {
        let options = ImageOptions::for_level(CompressionLevel::Low);
        assert_eq!(options.quality, 90);
        assert_eq!(options.output_format, OutputFormat::Original);
        assert_eq!(options.max_dimension, None);
        assert_eq!(options.encoding, ImageEncodingMode::Lossy);
    }

    #[test]
    fn default_limits_use_documented_constants() {
        let limits = Limits::default();
        assert_eq!(limits.max_input_bytes, DEFAULT_MAX_INPUT_BYTES);
        assert_eq!(limits.max_stream_bytes, DEFAULT_MAX_STREAM_BYTES);
        assert_eq!(limits.max_image_pixels, DEFAULT_MAX_IMAGE_PIXELS);
    }
}
