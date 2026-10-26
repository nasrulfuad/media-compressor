use clap::ValueEnum;
use std::fmt;

/// Compression level presets that control the trade-off between file size
/// reduction and output quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum CompressionLevel {
    /// Minimal compression — preserves near-original quality (10–20% reduction).
    Low,

    /// Balanced compression — good quality with significant size savings (30–50% reduction).
    #[default]
    Medium,

    /// Aggressive compression — lower quality, high size savings (50–70% reduction).
    High,

    /// Maximum compression — significant quality reduction for smallest file (70–85% reduction).
    Extreme,
}

impl CompressionLevel {
    /// JPEG quality factor (0–100) for image re-encoding.
    pub fn image_quality(&self) -> u8 {
        match self {
            CompressionLevel::Low => 90,
            CompressionLevel::Medium => 75,
            CompressionLevel::High => 60,
            CompressionLevel::Extreme => 40,
        }
    }

    /// Optional DPI target for image downsampling. `None` means no downsampling.
    pub fn target_dpi(&self) -> Option<u32> {
        match self {
            CompressionLevel::Low => None,
            CompressionLevel::Medium => Some(200),
            CompressionLevel::High => Some(150),
            CompressionLevel::Extreme => Some(96),
        }
    }

    /// The flate2 compression level to use when re-compressing PDF content streams.
    pub fn stream_compression_level(&self) -> flate2::Compression {
        match self {
            CompressionLevel::Low => flate2::Compression::default(),
            CompressionLevel::Medium => flate2::Compression::best(),
            CompressionLevel::High => flate2::Compression::best(),
            CompressionLevel::Extreme => flate2::Compression::best(),
        }
    }

    /// Whether metadata should be stripped at this level by default.
    pub fn strip_metadata(&self) -> bool {
        match self {
            CompressionLevel::Low => false,
            CompressionLevel::Medium => false,
            CompressionLevel::High => true,
            CompressionLevel::Extreme => true,
        }
    }

    /// Whether colour images should be converted to grayscale (Extreme only).
    pub fn convert_to_grayscale(&self) -> bool {
        matches!(self, CompressionLevel::Extreme)
    }

    /// Whether the document structure tree should be removed (High and Extreme).
    pub fn remove_struct_tree(&self) -> bool {
        matches!(self, CompressionLevel::High | CompressionLevel::Extreme)
    }

    /// Whether AcroForm fields should be removed (Extreme only).
    pub fn remove_acroform(&self) -> bool {
        matches!(self, CompressionLevel::Extreme)
    }

    /// Whether embedded files should be removed (High and Extreme).
    pub fn remove_embedded_files(&self) -> bool {
        matches!(self, CompressionLevel::High | CompressionLevel::Extreme)
    }
}

impl fmt::Display for CompressionLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompressionLevel::Low => write!(f, "low"),
            CompressionLevel::Medium => write!(f, "medium"),
            CompressionLevel::High => write!(f, "high"),
            CompressionLevel::Extreme => write!(f, "extreme"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_quality_decreases_with_level() {
        assert!(CompressionLevel::Low.image_quality() > CompressionLevel::Medium.image_quality());
        assert!(CompressionLevel::Medium.image_quality() > CompressionLevel::High.image_quality());
        assert!(CompressionLevel::High.image_quality() > CompressionLevel::Extreme.image_quality());
    }

    #[test]
    fn low_level_has_no_dpi_target() {
        assert_eq!(CompressionLevel::Low.target_dpi(), None);
    }

    #[test]
    fn higher_levels_have_dpi_targets() {
        assert!(CompressionLevel::Medium.target_dpi().is_some());
        assert!(CompressionLevel::High.target_dpi().is_some());
        assert!(CompressionLevel::Extreme.target_dpi().is_some());
    }

    #[test]
    fn dpi_target_decreases_with_level() {
        assert!(
            CompressionLevel::Medium.target_dpi().unwrap()
                > CompressionLevel::High.target_dpi().unwrap()
        );
        assert!(
            CompressionLevel::High.target_dpi().unwrap()
                > CompressionLevel::Extreme.target_dpi().unwrap()
        );
    }

    #[test]
    fn metadata_stripped_only_at_high_and_extreme() {
        assert!(!CompressionLevel::Low.strip_metadata());
        assert!(!CompressionLevel::Medium.strip_metadata());
        assert!(CompressionLevel::High.strip_metadata());
        assert!(CompressionLevel::Extreme.strip_metadata());
    }

    #[test]
    fn display_format_is_lowercase() {
        assert_eq!(CompressionLevel::Low.to_string(), "low");
        assert_eq!(CompressionLevel::Medium.to_string(), "medium");
        assert_eq!(CompressionLevel::High.to_string(), "high");
        assert_eq!(CompressionLevel::Extreme.to_string(), "extreme");
    }

    #[test]
    fn default_level_is_medium() {
        assert_eq!(CompressionLevel::default(), CompressionLevel::Medium);
    }
}
