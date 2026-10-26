use crate::app::{BatchRequest, CompressRequest, CompressionSettings};
use crate::domain::level::CompressionLevel;
use crate::domain::options::{
    ColorMode, FailurePolicy, FontOptimization, ImageEncodingMode, ImageOptions, MetadataPolicy,
    PdfOptions, WriteMode,
};

use super::args::{BatchArgs, CompressArgs};

struct SharedFlags {
    grayscale: bool,
    lossless: bool,
    keep_metadata: bool,
    compress_fonts: bool,
}

fn write_mode(dry_run: bool, overwrite: bool) -> WriteMode {
    if dry_run {
        WriteMode::DryRun
    } else if overwrite {
        WriteMode::Overwrite
    } else {
        WriteMode::Write
    }
}

fn apply_flags(settings: &mut CompressionSettings, flags: &SharedFlags) {
    if flags.grayscale {
        settings.pdf.color_mode = ColorMode::Grayscale;
        settings.image.color_mode = ColorMode::Grayscale;
    }
    if flags.lossless {
        settings.pdf.image_encoding = ImageEncodingMode::Lossless;
        settings.image.encoding = ImageEncodingMode::Lossless;
    }
    if flags.keep_metadata {
        settings.pdf.metadata = MetadataPolicy::Keep;
        settings.image.metadata = MetadataPolicy::Keep;
    }
    if flags.compress_fonts {
        settings.pdf.fonts = FontOptimization::Subset;
    }
}

fn base_settings(level: CompressionLevel, flags: &SharedFlags) -> CompressionSettings {
    let mut settings = CompressionSettings {
        pdf: PdfOptions::for_level(level),
        image: ImageOptions::for_level(level),
    };
    apply_flags(&mut settings, flags);
    settings
}

impl From<CompressArgs> for CompressRequest {
    fn from(args: CompressArgs) -> Self {
        let flags = SharedFlags {
            grayscale: args.grayscale,
            lossless: args.lossless,
            keep_metadata: args.keep_metadata,
            compress_fonts: args.compress_fonts,
        };
        let mut settings = base_settings(args.level.into(), &flags);
        if let Some(quality) = args.quality {
            settings.pdf.jpeg_quality = quality;
            settings.image.quality = quality;
        }
        if let Some(quality) = args.image_quality {
            settings.pdf.jpeg_quality = quality;
        }
        if let Some(dpi) = args.dpi {
            settings.pdf.target_dpi = Some(dpi);
        }
        settings.image.output_format = args.format.into();
        settings.image.max_dimension = args.max_dimension;
        CompressRequest {
            input: args.input,
            output: args.output,
            settings,
            write_mode: write_mode(args.dry_run, args.overwrite),
        }
    }
}

fn failure_policy(continue_on_error: bool) -> FailurePolicy {
    if continue_on_error {
        FailurePolicy::ContinueOnError
    } else {
        FailurePolicy::StopOnFirstError
    }
}

impl From<BatchArgs> for BatchRequest {
    fn from(args: BatchArgs) -> Self {
        let flags = SharedFlags {
            grayscale: false,
            lossless: false,
            keep_metadata: args.keep_metadata,
            compress_fonts: args.compress_fonts,
        };
        let mut settings = base_settings(args.level.into(), &flags);
        if let Some(quality) = args.image_quality {
            settings.pdf.jpeg_quality = quality;
            settings.image.quality = quality;
        }
        if let Some(dpi) = args.dpi {
            settings.pdf.target_dpi = Some(dpi);
        }
        BatchRequest {
            input_dir: args.input_dir,
            output_dir: args.output_dir,
            pattern: args.pattern,
            recursive: args.recursive,
            workers: args.workers,
            failure_policy: failure_policy(args.continue_on_error),
            settings,
            write_mode: write_mode(args.dry_run, args.overwrite),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::*;
    use crate::cli::args::{Cli, Commands};
    use crate::domain::media::OutputFormat;

    fn compress_request(arguments: &[&str]) -> CompressRequest {
        let mut full = vec!["media-compressor", "compress"];
        full.extend_from_slice(arguments);
        match Cli::try_parse_from(full).unwrap().command {
            Commands::Compress(args) => args.into(),
            _ => panic!("expected compress"),
        }
    }

    fn batch_request(arguments: &[&str]) -> BatchRequest {
        let mut full = vec!["media-compressor", "batch"];
        full.extend_from_slice(arguments);
        match Cli::try_parse_from(full).unwrap().command {
            Commands::Batch(args) => args.into(),
            _ => panic!("expected batch"),
        }
    }

    #[test]
    fn compress_args_map_level_and_defaults() {
        let request = compress_request(&["in.pdf", "-l", "high"]);
        assert_eq!(
            request.settings.pdf,
            PdfOptions::for_level(CompressionLevel::High)
        );
        assert_eq!(
            request.settings.image,
            ImageOptions::for_level(CompressionLevel::High)
        );
        assert_eq!(request.write_mode, WriteMode::Write);
        assert_eq!(request.output, None);
    }

    #[test]
    fn compress_args_pass_quality_to_pdf_and_image_engines() {
        let request = compress_request(&["in.pdf", "--quality", "33"]);
        assert_eq!(request.settings.pdf.jpeg_quality, 33);
        assert_eq!(request.settings.image.quality, 33);
    }

    #[test]
    fn compress_args_image_quality_overrides_quality_for_pdf_only() {
        let request = compress_request(&["in.pdf", "--quality", "33", "--image-quality", "55"]);
        assert_eq!(request.settings.pdf.jpeg_quality, 55);
        assert_eq!(request.settings.image.quality, 33);
    }

    #[test]
    fn compress_args_map_grayscale_lossless_and_metadata_flags_to_both_engines() {
        let request = compress_request(&["in.pdf", "--grayscale", "--lossless", "--keep-metadata"]);
        assert_eq!(request.settings.pdf.color_mode, ColorMode::Grayscale);
        assert_eq!(request.settings.image.color_mode, ColorMode::Grayscale);
        assert_eq!(
            request.settings.pdf.image_encoding,
            ImageEncodingMode::Lossless
        );
        assert_eq!(request.settings.image.encoding, ImageEncodingMode::Lossless);
        assert_eq!(request.settings.pdf.metadata, MetadataPolicy::Keep);
        assert_eq!(request.settings.image.metadata, MetadataPolicy::Keep);
    }

    #[test]
    fn compress_args_map_image_only_options() {
        let request = compress_request(&[
            "in.png",
            "-f",
            "jpeg",
            "--max-dimension",
            "800",
            "--dpi",
            "120",
        ]);
        assert_eq!(request.settings.image.output_format, OutputFormat::Jpeg);
        assert_eq!(request.settings.image.max_dimension, Some(800));
        assert_eq!(request.settings.pdf.target_dpi, Some(120));
    }

    #[test]
    fn compress_args_map_extreme_level_to_grayscale_without_flag() {
        let request = compress_request(&["in.pdf", "-l", "extreme"]);
        assert_eq!(request.settings.pdf.color_mode, ColorMode::Grayscale);
    }

    #[test]
    fn compress_args_map_fonts_flag_to_subset_optimization() {
        let request = compress_request(&["in.pdf", "--compress-fonts"]);
        assert_eq!(request.settings.pdf.fonts, FontOptimization::Subset);
    }

    #[test]
    fn compress_args_map_dry_run_before_overwrite() {
        assert_eq!(
            compress_request(&["a.pdf", "--dry-run", "--overwrite"]).write_mode,
            WriteMode::DryRun
        );
        assert_eq!(
            compress_request(&["a.pdf", "--overwrite"]).write_mode,
            WriteMode::Overwrite
        );
    }

    #[test]
    fn batch_args_map_continue_on_error_to_failure_policy() {
        assert_eq!(
            batch_request(&["dir"]).failure_policy,
            FailurePolicy::StopOnFirstError
        );
        assert_eq!(
            batch_request(&["dir", "--continue-on-error"]).failure_policy,
            FailurePolicy::ContinueOnError
        );
    }

    #[test]
    fn batch_args_map_paths_pattern_and_workers() {
        let request = batch_request(&[
            "dir",
            "-o",
            "out",
            "-p",
            "*.pdf",
            "-r",
            "-w",
            "3",
            "--dry-run",
        ]);
        assert_eq!(request.input_dir, PathBuf::from("dir"));
        assert_eq!(request.output_dir, Some(PathBuf::from("out")));
        assert_eq!(request.pattern, "*.pdf");
        assert!(request.recursive);
        assert_eq!(request.workers, 3);
        assert_eq!(request.write_mode, WriteMode::DryRun);
    }

    #[test]
    fn batch_args_map_quality_dpi_and_metadata_overrides() {
        let request = batch_request(&[
            "dir",
            "--image-quality",
            "42",
            "--dpi",
            "110",
            "--keep-metadata",
        ]);
        assert_eq!(request.settings.pdf.jpeg_quality, 42);
        assert_eq!(request.settings.image.quality, 42);
        assert_eq!(request.settings.pdf.target_dpi, Some(110));
        assert_eq!(request.settings.pdf.metadata, MetadataPolicy::Keep);
    }
}
