use lopdf::Document;

use crate::domain::options::{FontOptimization, MetadataPolicy, PdfOptions};
use crate::error::AppResult;

use super::passes::cleanup::remove_unnecessary_objects;
use super::passes::dedup::deduplicate_streams;
use super::passes::fonts::subset_fonts;
use super::passes::images::{build_page_dpi_map, compress_images};
use super::passes::inline_images::convert_inline_images;
use super::passes::merge_content::merge_page_content_streams;
use super::passes::metadata::strip_metadata;
use super::passes::streams::compress_streams;

pub trait PdfPass {
    fn name(&self) -> &'static str;
    fn is_enabled(&self, options: &PdfOptions) -> bool;
    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64>;
}

struct CleanupPass;
struct InlineImagesPass;
struct DedupPass;
struct StreamsPass;
struct ImagesPass;
struct MergeContentPass;
struct FontsPass;
struct MetadataPass;

impl PdfPass for CleanupPass {
    fn name(&self) -> &'static str {
        "cleanup"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
        remove_unnecessary_objects(document, &options.level.preset().cleanup)?;
        Ok(0)
    }
}

impl PdfPass for InlineImagesPass {
    fn name(&self) -> &'static str {
        "inline_images"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
        convert_inline_images(document, options)?;
        Ok(0)
    }
}

impl PdfPass for DedupPass {
    fn name(&self) -> &'static str {
        "dedup"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, _options: &PdfOptions) -> AppResult<i64> {
        deduplicate_streams(document)
    }
}

impl PdfPass for StreamsPass {
    fn name(&self) -> &'static str {
        "streams"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
        compress_streams(document, options)
    }
}

impl PdfPass for ImagesPass {
    fn name(&self) -> &'static str {
        "images"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
        let page_dimensions = build_page_dpi_map(document);
        compress_images(document, options, &page_dimensions)
    }
}

impl PdfPass for MergeContentPass {
    fn name(&self) -> &'static str {
        "merge_content"
    }

    fn is_enabled(&self, _options: &PdfOptions) -> bool {
        true
    }

    fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64> {
        merge_page_content_streams(document, options)
    }
}

impl PdfPass for FontsPass {
    fn name(&self) -> &'static str {
        "fonts"
    }

    fn is_enabled(&self, options: &PdfOptions) -> bool {
        options.fonts == FontOptimization::Subset
    }

    fn run(&self, document: &mut Document, _options: &PdfOptions) -> AppResult<i64> {
        Ok(subset_fonts(document)? as i64)
    }
}

impl PdfPass for MetadataPass {
    fn name(&self) -> &'static str {
        "metadata"
    }

    fn is_enabled(&self, options: &PdfOptions) -> bool {
        options.metadata == MetadataPolicy::FollowLevel && options.level.preset().strip_metadata
    }

    fn run(&self, document: &mut Document, _options: &PdfOptions) -> AppResult<i64> {
        strip_metadata(document);
        Ok(0)
    }
}

pub fn standard_pipeline() -> Vec<Box<dyn PdfPass>> {
    vec![
        Box::new(CleanupPass),
        Box::new(InlineImagesPass),
        Box::new(DedupPass),
        Box::new(StreamsPass),
        Box::new(ImagesPass),
        Box::new(MergeContentPass),
        Box::new(FontsPass),
        Box::new(MetadataPass),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::level::CompressionLevel;

    fn enabled_names(options: &PdfOptions) -> Vec<&'static str> {
        standard_pipeline()
            .iter()
            .filter(|pass| pass.is_enabled(options))
            .map(|pass| pass.name())
            .collect()
    }

    #[test]
    fn standard_pipeline_orders_passes_with_inline_images_before_dedup() {
        let names: Vec<&str> = standard_pipeline().iter().map(|pass| pass.name()).collect();
        assert_eq!(
            names,
            vec![
                "cleanup",
                "inline_images",
                "dedup",
                "streams",
                "images",
                "merge_content",
                "fonts",
                "metadata"
            ]
        );
    }

    #[test]
    fn fonts_and_metadata_are_disabled_for_default_medium_level() {
        let options = PdfOptions::for_level(CompressionLevel::Medium);
        assert_eq!(
            enabled_names(&options),
            vec![
                "cleanup",
                "inline_images",
                "dedup",
                "streams",
                "images",
                "merge_content"
            ]
        );
    }

    #[test]
    fn metadata_pass_is_enabled_at_high_level_unless_policy_keeps_metadata() {
        let mut options = PdfOptions::for_level(CompressionLevel::High);
        assert!(enabled_names(&options).contains(&"metadata"));
        options.metadata = MetadataPolicy::Keep;
        assert!(!enabled_names(&options).contains(&"metadata"));
    }

    #[test]
    fn fonts_pass_is_enabled_only_when_font_optimization_is_requested() {
        let mut options = PdfOptions::for_level(CompressionLevel::Medium);
        options.fonts = FontOptimization::Subset;
        assert!(enabled_names(&options).contains(&"fonts"));
    }
}
