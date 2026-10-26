use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    Low,
    #[default]
    Medium,
    High,
    Extreme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPolicy {
    pub remove_markup_annotations: bool,
    pub remove_forms: bool,
    pub remove_struct_tree: bool,
    pub remove_embedded_files: bool,
    pub remove_output_intents: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPreset {
    pub jpeg_quality: u8,
    pub target_dpi: Option<u32>,
    pub deflate_level: u32,
    pub png_effort: u8,
    pub strip_metadata: bool,
    pub grayscale: bool,
    pub cleanup: CleanupPolicy,
}

impl CompressionLevel {
    pub fn preset(self) -> LevelPreset {
        match self {
            Self::Low => LevelPreset {
                jpeg_quality: 90,
                target_dpi: None,
                deflate_level: 6,
                png_effort: 2,
                strip_metadata: false,
                grayscale: false,
                cleanup: CleanupPolicy {
                    remove_markup_annotations: false,
                    remove_forms: false,
                    remove_struct_tree: false,
                    remove_embedded_files: false,
                    remove_output_intents: false,
                },
            },
            Self::Medium => LevelPreset {
                jpeg_quality: 75,
                target_dpi: Some(200),
                deflate_level: 9,
                png_effort: 4,
                strip_metadata: false,
                grayscale: false,
                cleanup: CleanupPolicy {
                    remove_markup_annotations: false,
                    remove_forms: false,
                    remove_struct_tree: false,
                    remove_embedded_files: false,
                    remove_output_intents: false,
                },
            },
            Self::High => LevelPreset {
                jpeg_quality: 60,
                target_dpi: Some(150),
                deflate_level: 9,
                png_effort: 5,
                strip_metadata: true,
                grayscale: false,
                cleanup: CleanupPolicy {
                    remove_markup_annotations: true,
                    remove_forms: false,
                    remove_struct_tree: true,
                    remove_embedded_files: true,
                    remove_output_intents: false,
                },
            },
            Self::Extreme => LevelPreset {
                jpeg_quality: 40,
                target_dpi: Some(96),
                deflate_level: 9,
                png_effort: 6,
                strip_metadata: true,
                grayscale: true,
                cleanup: CleanupPolicy {
                    remove_markup_annotations: true,
                    remove_forms: true,
                    remove_struct_tree: true,
                    remove_embedded_files: true,
                    remove_output_intents: true,
                },
            },
        }
    }
}

impl fmt::Display for CompressionLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Extreme => "extreme",
        };
        formatter.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_LEVELS: [CompressionLevel; 4] = [
        CompressionLevel::Low,
        CompressionLevel::Medium,
        CompressionLevel::High,
        CompressionLevel::Extreme,
    ];

    #[test]
    fn jpeg_quality_matches_level_table() {
        let qualities: Vec<u8> = ALL_LEVELS.iter().map(|l| l.preset().jpeg_quality).collect();
        assert_eq!(qualities, vec![90, 75, 60, 40]);
    }

    #[test]
    fn target_dpi_matches_level_table() {
        let dpis: Vec<Option<u32>> = ALL_LEVELS.iter().map(|l| l.preset().target_dpi).collect();
        assert_eq!(dpis, vec![None, Some(200), Some(150), Some(96)]);
    }

    #[test]
    fn deflate_and_png_effort_match_level_table() {
        let deflate: Vec<u32> = ALL_LEVELS
            .iter()
            .map(|l| l.preset().deflate_level)
            .collect();
        let effort: Vec<u8> = ALL_LEVELS.iter().map(|l| l.preset().png_effort).collect();
        assert_eq!(deflate, vec![6, 9, 9, 9]);
        assert_eq!(effort, vec![2, 4, 5, 6]);
    }

    #[test]
    fn metadata_stripped_only_at_high_and_extreme() {
        let flags: Vec<bool> = ALL_LEVELS
            .iter()
            .map(|l| l.preset().strip_metadata)
            .collect();
        assert_eq!(flags, vec![false, false, true, true]);
    }

    #[test]
    fn grayscale_only_at_extreme() {
        let flags: Vec<bool> = ALL_LEVELS.iter().map(|l| l.preset().grayscale).collect();
        assert_eq!(flags, vec![false, false, false, true]);
    }

    #[test]
    fn cleanup_policy_matches_level_table() {
        let low = CompressionLevel::Low.preset().cleanup;
        let medium = CompressionLevel::Medium.preset().cleanup;
        let high = CompressionLevel::High.preset().cleanup;
        let extreme = CompressionLevel::Extreme.preset().cleanup;
        assert_eq!(low, medium);
        assert!(!low.remove_markup_annotations && !low.remove_struct_tree);
        assert!(high.remove_markup_annotations && high.remove_struct_tree);
        assert!(high.remove_embedded_files);
        assert!(!high.remove_forms && !high.remove_output_intents);
        assert!(extreme.remove_forms && extreme.remove_output_intents);
    }

    #[test]
    fn display_format_is_lowercase() {
        let names: Vec<String> = ALL_LEVELS.iter().map(|l| l.to_string()).collect();
        assert_eq!(names, vec!["low", "medium", "high", "extreme"]);
    }

    #[test]
    fn default_level_is_medium() {
        assert_eq!(CompressionLevel::default(), CompressionLevel::Medium);
    }
}
