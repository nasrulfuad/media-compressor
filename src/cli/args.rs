use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::compressor::level::CompressionLevel;
use crate::strategy::ImageOutputFormat;

/// Fast media compressor — compress PDF and image files with configurable quality levels.
#[derive(Debug, Parser)]
#[command(
    name = "media-compressor",
    version,
    author,
    about,
    long_about = None,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose logging output.
    #[arg(short = 'v', long = "verbose", global = true)]
    pub verbose: bool,

    /// Suppress all output except errors.
    #[arg(short = 'q', long = "quiet", global = true, conflicts_with = "verbose")]
    pub quiet: bool,
}

/// Available subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Compress any supported file (auto-detects PDF or image via magic bytes).
    Compress(CompressArgs),

    /// Compress all supported files (PDF and images) in a directory.
    Batch(BatchArgs),

    /// Display metadata and size information about a PDF file.
    Info(InfoArgs),
}

/// Arguments for the `compress` subcommand.
#[derive(Debug, Parser)]
pub struct CompressArgs {
    /// Path to the input PDF file.
    #[arg(value_name = "INPUT")]
    pub input: PathBuf,

    /// Path for the compressed output file.
    ///
    /// Defaults to `<input-stem>_compressed.pdf` in the same directory as the input.
    #[arg(short = 'o', long = "output", value_name = "OUTPUT")]
    pub output: Option<PathBuf>,

    /// Compression level preset.
    #[arg(
        short = 'l',
        long = "level",
        value_name = "LEVEL",
        default_value = "medium"
    )]
    pub level: CompressionLevel,

    /// Preserve document metadata (author, creation date, etc.) regardless of level.
    #[arg(long = "keep-metadata")]
    pub keep_metadata: bool,

    /// Override the JPEG image quality (1–100). Overrides the level preset.
    #[arg(long = "image-quality", value_name = "QUALITY", value_parser = clap::value_parser!(u8).range(1..=100))]
    pub image_quality: Option<u8>,

    /// Override the target image DPI. Overrides the level preset.
    #[arg(long = "dpi", value_name = "DPI")]
    pub dpi: Option<u32>,

    /// Estimate compression savings without writing any output file.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Output the compression report as JSON instead of pretty-printed text.
    #[arg(long = "json")]
    pub json: bool,

    /// Overwrite the output file if it already exists.
    #[arg(long = "overwrite")]
    pub overwrite: bool,

    // ── Image-specific flags (ignored for PDF) ─────────────────────────────
    /// Override compression quality (1-100).
    ///
    /// For PDF: quality of re-encoded embedded images (overrides --image-quality).
    /// For images: encoding quality.  Overrides the level preset.
    #[arg(
        long = "quality",
        value_name = "NUM",
        value_parser = clap::value_parser!(u8).range(1..=100)
    )]
    pub quality: Option<u8>,

    /// Output format for image files [default: original].
    ///
    /// For PDF files this flag is silently ignored.
    #[arg(
        short = 'f',
        long = "format",
        value_name = "FMT",
        default_value = "original"
    )]
    pub format: ImageOutputFormat,

    /// Resize image so neither dimension exceeds PX pixels (aspect ratio preserved).
    ///
    /// For PDF files this flag is silently ignored.
    #[arg(long = "max-dimension", value_name = "PX")]
    pub max_dimension: Option<u32>,

    /// Convert output to grayscale.
    #[arg(long = "grayscale")]
    pub grayscale: bool,

    /// Strip metadata (EXIF for images, /Info+XMP for PDF).
    ///
    /// Default behaviour is determined by --level; this flag forces stripping.
    #[arg(long = "strip-metadata", conflicts_with = "keep_metadata")]
    pub strip_metadata: bool,

    /// Lossless compression mode.
    ///
    /// For images: PNG/WebP lossless encode; JPEG input is converted to PNG.
    /// For PDF: skip lossy image recompression.
    #[arg(long = "lossless")]
    pub lossless: bool,

    /// Run font deduplication and subsetting on PDF files.
    ///
    /// Disabled by default. When enabled, unused glyphs are stripped and
    /// duplicate font streams are merged, which can reduce file size but
    /// may affect rendering on some PDF viewers.
    #[arg(long = "compress-fonts")]
    pub compress_fonts: bool,
}

/// Arguments for the `batch` subcommand.
#[derive(Debug, Parser)]
pub struct BatchArgs {
    /// Directory containing PDF files to compress.
    #[arg(value_name = "INPUT_DIR")]
    pub input_dir: PathBuf,

    /// Output directory for compressed files.
    ///
    /// Defaults to `<input-dir>_compressed`. Directory structure is preserved.
    #[arg(short = 'o', long = "output-dir", value_name = "OUTPUT_DIR")]
    pub output_dir: Option<PathBuf>,

    /// Compression level preset.
    #[arg(
        short = 'l',
        long = "level",
        value_name = "LEVEL",
        default_value = "medium"
    )]
    pub level: CompressionLevel,

    /// Glob pattern to match files.
    #[arg(
        short = 'p',
        long = "pattern",
        value_name = "GLOB",
        default_value = "*.{pdf,jpg,jpeg,png,webp,tif,tiff,bmp,gif}"
    )]
    pub pattern: String,

    /// Recurse into subdirectories.
    #[arg(short = 'r', long = "recursive")]
    pub recursive: bool,

    /// Number of parallel workers (0 = auto-detect CPU count).
    #[arg(short = 'w', long = "workers", value_name = "NUM", default_value = "0")]
    pub workers: usize,

    /// Continue processing when a file fails.
    #[arg(long = "continue-on-error")]
    pub continue_on_error: bool,

    /// Preserve document metadata (author, creation date, etc.) regardless of level.
    #[arg(long = "keep-metadata")]
    pub keep_metadata: bool,

    /// Override the JPEG image quality (1-100). Overrides the level preset.
    #[arg(long = "image-quality", value_name = "QUALITY", value_parser = clap::value_parser!(u8).range(1..=100))]
    pub image_quality: Option<u8>,

    /// Override the target image DPI. Overrides the level preset.
    #[arg(long = "dpi", value_name = "DPI")]
    pub dpi: Option<u32>,

    /// Estimate compression savings without writing any output files.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Output the batch report as JSON instead of pretty-printed text.
    #[arg(long = "json")]
    pub json: bool,

    /// Overwrite existing output files.
    #[arg(long = "overwrite")]
    pub overwrite: bool,

    /// Run font deduplication and subsetting on PDF files.
    ///
    /// Disabled by default. See `compress --compress-fonts` for details.
    #[arg(long = "compress-fonts")]
    pub compress_fonts: bool,
}

/// Arguments for the `info` subcommand.
#[derive(Debug, Parser)]
pub struct InfoArgs {
    /// Path to the PDF file to inspect.
    #[arg(value_name = "INPUT")]
    pub input: PathBuf,

    /// Output the information as JSON.
    #[arg(long = "json")]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_debug_assert_passes() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_compress_with_defaults() {
        let cli = Cli::try_parse_from(["media-compressor", "compress", "input.pdf"]).unwrap();
        match cli.command {
            Commands::Compress(args) => {
                assert_eq!(args.input, PathBuf::from("input.pdf"));
                assert_eq!(args.level, CompressionLevel::Medium);
                assert!(args.output.is_none());
                assert!(!args.keep_metadata);
                assert!(!args.dry_run);
                assert!(!args.json);
                assert!(!args.overwrite);
            }
            _ => panic!("Expected Compress command"),
        }
    }

    #[test]
    fn parse_compress_with_all_flags() {
        let cli = Cli::try_parse_from([
            "media-compressor",
            "compress",
            "input.pdf",
            "-o",
            "output.pdf",
            "-l",
            "high",
            "--keep-metadata",
            "--image-quality",
            "80",
            "--dpi",
            "150",
            "--dry-run",
            "--json",
            "--overwrite",
        ])
        .unwrap();

        match cli.command {
            Commands::Compress(args) => {
                assert_eq!(args.input, PathBuf::from("input.pdf"));
                assert_eq!(args.output, Some(PathBuf::from("output.pdf")));
                assert_eq!(args.level, CompressionLevel::High);
                assert!(args.keep_metadata);
                assert_eq!(args.image_quality, Some(80));
                assert_eq!(args.dpi, Some(150));
                assert!(args.dry_run);
                assert!(args.json);
                assert!(args.overwrite);
            }
            _ => panic!("Expected Compress command"),
        }
    }

    #[test]
    fn parse_info_command() {
        let cli =
            Cli::try_parse_from(["media-compressor", "info", "document.pdf", "--json"]).unwrap();
        match cli.command {
            Commands::Info(args) => {
                assert_eq!(args.input, PathBuf::from("document.pdf"));
                assert!(args.json);
            }
            _ => panic!("Expected Info command"),
        }
    }

    #[test]
    fn verbose_and_quiet_conflict() {
        let result = Cli::try_parse_from([
            "media-compressor",
            "--verbose",
            "--quiet",
            "compress",
            "a.pdf",
        ]);
        assert!(result.is_err(), "verbose and quiet should conflict");
    }

    #[test]
    fn image_quality_rejects_zero() {
        let result = Cli::try_parse_from([
            "media-compressor",
            "compress",
            "a.pdf",
            "--image-quality",
            "0",
        ]);
        assert!(result.is_err(), "image-quality 0 should be rejected");
    }

    #[test]
    fn all_compression_levels_parse() {
        for level in &["low", "medium", "high", "extreme"] {
            let result =
                Cli::try_parse_from(["media-compressor", "compress", "a.pdf", "-l", level]);
            assert!(
                result.is_ok(),
                "Level '{}' should parse successfully",
                level
            );
        }
    }

    #[test]
    fn parse_batch_with_defaults() {
        let cli = Cli::try_parse_from(["media-compressor", "batch", "/tmp/pdfs"]).unwrap();
        match cli.command {
            Commands::Batch(args) => {
                assert_eq!(args.input_dir, PathBuf::from("/tmp/pdfs"));
                assert!(args.output_dir.is_none());
                assert_eq!(args.level, CompressionLevel::Medium);
                assert_eq!(args.pattern, "*.{pdf,jpg,jpeg,png,webp,tif,tiff,bmp,gif}");
                assert!(!args.recursive);
                assert_eq!(args.workers, 0);
                assert!(!args.continue_on_error);
                assert!(!args.dry_run);
                assert!(!args.json);
                assert!(!args.overwrite);
            }
            _ => panic!("Expected Batch command"),
        }
    }

    #[test]
    fn parse_batch_with_all_flags() {
        let cli = Cli::try_parse_from([
            "media-compressor",
            "batch",
            "/tmp/pdfs",
            "-o",
            "/tmp/out",
            "-l",
            "high",
            "-p",
            "report_*.pdf",
            "-r",
            "-w",
            "4",
            "--continue-on-error",
            "--keep-metadata",
            "--image-quality",
            "60",
            "--dpi",
            "150",
            "--dry-run",
            "--json",
            "--overwrite",
        ])
        .unwrap();

        match cli.command {
            Commands::Batch(args) => {
                assert_eq!(args.input_dir, PathBuf::from("/tmp/pdfs"));
                assert_eq!(args.output_dir, Some(PathBuf::from("/tmp/out")));
                assert_eq!(args.level, CompressionLevel::High);
                assert_eq!(args.pattern, "report_*.pdf");
                assert!(args.recursive);
                assert_eq!(args.workers, 4);
                assert!(args.continue_on_error);
                assert!(args.keep_metadata);
                assert_eq!(args.image_quality, Some(60));
                assert_eq!(args.dpi, Some(150));
                assert!(args.dry_run);
                assert!(args.json);
                assert!(args.overwrite);
            }
            _ => panic!("Expected Batch command"),
        }
    }

    #[test]
    fn parse_compress_with_image_flags() {
        let cli = Cli::try_parse_from([
            "media-compressor",
            "compress",
            "photo.jpg",
            "--quality",
            "75",
            "-f",
            "png",
            "--max-dimension",
            "1920",
            "--grayscale",
            "--strip-metadata",
            "--lossless",
        ])
        .unwrap();
        match cli.command {
            Commands::Compress(args) => {
                assert_eq!(args.quality, Some(75));
                assert_eq!(args.format, ImageOutputFormat::Png);
                assert_eq!(args.max_dimension, Some(1920));
                assert!(args.grayscale);
                assert!(args.strip_metadata);
                assert!(args.lossless);
            }
            _ => panic!("Expected Compress command"),
        }
    }

    #[test]
    fn quality_zero_is_rejected() {
        let result =
            Cli::try_parse_from(["media-compressor", "compress", "a.pdf", "--quality", "0"]);
        assert!(result.is_err(), "quality 0 should be rejected");
    }

    #[test]
    fn strip_metadata_and_keep_metadata_conflict() {
        let result = Cli::try_parse_from([
            "media-compressor",
            "compress",
            "a.pdf",
            "--strip-metadata",
            "--keep-metadata",
        ]);
        assert!(
            result.is_err(),
            "--strip-metadata and --keep-metadata should conflict"
        );
    }
}
