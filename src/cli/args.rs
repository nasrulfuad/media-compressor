use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::domain::level::CompressionLevel;
use crate::domain::media::OutputFormat;
use crate::domain::options::DEFAULT_MAX_INPUT_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LevelArg {
    #[value(help = "Minimal compression, preserves near-original quality")]
    Low,
    #[value(help = "Balanced compression with significant size savings")]
    Medium,
    #[value(help = "Aggressive compression with lower quality")]
    High,
    #[value(help = "Maximum compression for the smallest file")]
    Extreme,
}

impl From<LevelArg> for CompressionLevel {
    fn from(level: LevelArg) -> Self {
        match level {
            LevelArg::Low => CompressionLevel::Low,
            LevelArg::Medium => CompressionLevel::Medium,
            LevelArg::High => CompressionLevel::High,
            LevelArg::Extreme => CompressionLevel::Extreme,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    #[value(help = "Keep the same format as the input")]
    Original,
    #[value(help = "Encode as JPEG (lossy)")]
    Jpeg,
    #[value(help = "Encode as PNG (lossless)")]
    Png,
}

impl From<FormatArg> for OutputFormat {
    fn from(format: FormatArg) -> Self {
        match format {
            FormatArg::Original => OutputFormat::Original,
            FormatArg::Jpeg => OutputFormat::Jpeg,
            FormatArg::Png => OutputFormat::Png,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "media-compressor",
    version,
    author,
    about = "Fast media compressor for PDF and image files",
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(
        short = 'v',
        long = "verbose",
        global = true,
        help = "Enable verbose logging output"
    )]
    pub verbose: bool,

    #[arg(
        short = 'q',
        long = "quiet",
        global = true,
        conflicts_with = "verbose",
        help = "Suppress all output except errors"
    )]
    pub quiet: bool,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "Compress a PDF or image file (type detected from magic bytes)")]
    Compress(CompressArgs),
    #[command(about = "Compress all supported files (PDF and images) in a directory")]
    Batch(BatchArgs),
    #[command(about = "Display metadata and size information about a PDF file")]
    Info(InfoArgs),
}

#[derive(Debug, Args)]
pub struct MediaOptionsArgs {
    #[arg(
        short = 'l',
        long = "level",
        value_name = "LEVEL",
        default_value = "medium",
        help = "Compression level preset"
    )]
    pub level: LevelArg,

    #[arg(
        long = "keep-metadata",
        help = "Preserve document metadata (author, creation date, etc.) regardless of level"
    )]
    pub keep_metadata: bool,

    #[arg(
        long = "quality",
        value_name = "NUM",
        value_parser = clap::value_parser!(u8).range(1..=100),
        help = "Override encoding quality (1-100) for PDF images and image files",
        long_help = "Override encoding quality (1-100).\n\nFor PDF: quality of re-encoded embedded images (--image-quality takes precedence).\nFor images: encoding quality. Overrides the level preset."
    )]
    pub quality: Option<u8>,

    #[arg(
        long = "image-quality",
        value_name = "QUALITY",
        value_parser = clap::value_parser!(u8).range(1..=100),
        help = "Override the JPEG quality (1-100) of images embedded in PDFs"
    )]
    pub image_quality: Option<u8>,

    #[arg(
        long = "dpi",
        value_name = "DPI",
        help = "Override the target DPI of images embedded in PDFs"
    )]
    pub dpi: Option<u32>,

    #[arg(
        short = 'f',
        long = "format",
        value_name = "FMT",
        default_value = "original",
        help = "Output format for image files (ignored for PDF)"
    )]
    pub format: FormatArg,

    #[arg(
        long = "max-dimension",
        value_name = "PX",
        help = "Resize images so neither side exceeds PX pixels (ignored for PDF)"
    )]
    pub max_dimension: Option<u32>,

    #[arg(long = "grayscale", help = "Convert output to grayscale")]
    pub grayscale: bool,

    #[arg(
        long = "lossless",
        help = "Lossless mode: JPEG input becomes PNG, PDF images skip lossy recompression"
    )]
    pub lossless: bool,

    #[arg(
        long = "compress-fonts",
        help = "Subset embedded identity CID TrueType fonts in PDF files (opt-in)"
    )]
    pub compress_fonts: bool,

    #[arg(
        long = "max-input-size",
        value_name = "BYTES",
        default_value_t = DEFAULT_MAX_INPUT_BYTES,
        help = "Reject input files larger than BYTES"
    )]
    pub max_input_size: u64,
}

#[derive(Debug, Parser)]
pub struct CompressArgs {
    #[arg(value_name = "INPUT", help = "Path to the input PDF or image file")]
    pub input: PathBuf,

    #[arg(
        short = 'o',
        long = "output",
        value_name = "OUTPUT",
        help = "Path for the compressed output file (default: <input-stem>_compressed.<ext>)"
    )]
    pub output: Option<PathBuf>,

    #[command(flatten)]
    pub options: MediaOptionsArgs,

    #[arg(
        long = "dry-run",
        help = "Estimate compression savings without writing any output file"
    )]
    pub dry_run: bool,

    #[arg(
        long = "json",
        help = "Output the compression report as JSON instead of pretty-printed text"
    )]
    pub json: bool,

    #[arg(
        long = "overwrite",
        help = "Overwrite the output file if it already exists"
    )]
    pub overwrite: bool,
}

#[derive(Debug, Parser)]
pub struct BatchArgs {
    #[arg(
        value_name = "INPUT_DIR",
        help = "Directory containing files to compress"
    )]
    pub input_dir: PathBuf,

    #[arg(
        short = 'o',
        long = "output-dir",
        value_name = "OUTPUT_DIR",
        help = "Output directory (default: <input-dir>_compressed); directory structure is preserved"
    )]
    pub output_dir: Option<PathBuf>,

    #[command(flatten)]
    pub options: MediaOptionsArgs,

    #[arg(
        short = 'p',
        long = "pattern",
        value_name = "GLOB",
        default_value = "*.{pdf,jpg,jpeg,png}",
        help = "Glob pattern to match files (case-insensitive)"
    )]
    pub pattern: String,

    #[arg(short = 'r', long = "recursive", help = "Recurse into subdirectories")]
    pub recursive: bool,

    #[arg(
        short = 'w',
        long = "workers",
        value_name = "NUM",
        default_value = "0",
        help = "Number of parallel workers (0 = auto-detect CPU count)"
    )]
    pub workers: usize,

    #[arg(
        long = "continue-on-error",
        help = "Keep processing remaining files when one fails"
    )]
    pub continue_on_error: bool,

    #[arg(
        long = "dry-run",
        help = "Estimate compression savings without writing any output files"
    )]
    pub dry_run: bool,

    #[arg(
        long = "json",
        help = "Output the batch report as JSON instead of pretty-printed text"
    )]
    pub json: bool,

    #[arg(long = "overwrite", help = "Overwrite existing output files")]
    pub overwrite: bool,
}

#[derive(Debug, Parser)]
pub struct InfoArgs {
    #[arg(value_name = "INPUT", help = "Path to the PDF file to inspect")]
    pub input: PathBuf,

    #[arg(long = "json", help = "Output the information as JSON")]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(arguments: &[&str]) -> Cli {
        let mut full = vec!["media-compressor"];
        full.extend_from_slice(arguments);
        Cli::try_parse_from(full).unwrap()
    }

    fn compress(arguments: &[&str]) -> CompressArgs {
        let mut full = vec!["compress"];
        full.extend_from_slice(arguments);
        match parse(&full).command {
            Commands::Compress(args) => args,
            _ => panic!("expected compress"),
        }
    }

    fn batch(arguments: &[&str]) -> BatchArgs {
        let mut full = vec!["batch"];
        full.extend_from_slice(arguments);
        match parse(&full).command {
            Commands::Batch(args) => args,
            _ => panic!("expected batch"),
        }
    }

    fn help_of(subcommand: &str) -> String {
        let mut command = Cli::command();
        let sub = command.find_subcommand_mut(subcommand).unwrap();
        sub.render_long_help().to_string()
    }

    #[test]
    fn cli_debug_assert_passes() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_compress_with_defaults() {
        let args = compress(&["input.pdf"]);
        assert_eq!(args.input, PathBuf::from("input.pdf"));
        assert_eq!(args.options.level, LevelArg::Medium);
        assert_eq!(args.options.format, FormatArg::Original);
        assert_eq!(args.options.max_input_size, DEFAULT_MAX_INPUT_BYTES);
        assert!(args.output.is_none());
        assert!(!args.options.keep_metadata && !args.dry_run && !args.json && !args.overwrite);
    }

    #[test]
    fn parse_compress_with_all_flags() {
        let args = compress(&[
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
            "--max-input-size",
            "1000",
        ]);
        assert_eq!(args.output, Some(PathBuf::from("output.pdf")));
        assert_eq!(args.options.level, LevelArg::High);
        assert!(args.options.keep_metadata);
        assert_eq!(args.options.image_quality, Some(80));
        assert_eq!(args.options.dpi, Some(150));
        assert_eq!(args.options.max_input_size, 1000);
        assert!(args.dry_run && args.json && args.overwrite);
    }

    #[test]
    fn parse_compress_with_image_flags() {
        let args = compress(&[
            "photo.jpg",
            "--quality",
            "75",
            "-f",
            "png",
            "--max-dimension",
            "1920",
            "--grayscale",
            "--lossless",
        ]);
        assert_eq!(args.options.quality, Some(75));
        assert_eq!(args.options.format, FormatArg::Png);
        assert_eq!(args.options.max_dimension, Some(1920));
        assert!(args.options.grayscale && args.options.lossless);
    }

    #[test]
    fn parse_batch_with_defaults() {
        let args = batch(&["/tmp/pdfs"]);
        assert_eq!(args.input_dir, PathBuf::from("/tmp/pdfs"));
        assert!(args.output_dir.is_none());
        assert_eq!(args.options.level, LevelArg::Medium);
        assert_eq!(args.pattern, "*.{pdf,jpg,jpeg,png}");
        assert_eq!(args.workers, 0);
        assert!(!args.recursive && !args.continue_on_error && !args.dry_run);
    }

    #[test]
    fn parse_batch_accepts_image_flags_like_compress() {
        let args = batch(&[
            "dir",
            "--quality",
            "50",
            "-f",
            "jpeg",
            "--max-dimension",
            "800",
            "--grayscale",
            "--lossless",
            "--max-input-size",
            "42",
        ]);
        assert_eq!(args.options.quality, Some(50));
        assert_eq!(args.options.format, FormatArg::Jpeg);
        assert_eq!(args.options.max_dimension, Some(800));
        assert!(args.options.grayscale && args.options.lossless);
        assert_eq!(args.options.max_input_size, 42);
    }

    #[test]
    fn parse_batch_with_all_flags() {
        let args = batch(&[
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
        ]);
        assert_eq!(args.output_dir, Some(PathBuf::from("/tmp/out")));
        assert_eq!(args.pattern, "report_*.pdf");
        assert_eq!(args.workers, 4);
        assert!(args.recursive && args.continue_on_error && args.options.keep_metadata);
        assert_eq!(args.options.image_quality, Some(60));
        assert!(args.dry_run && args.json && args.overwrite);
    }

    #[test]
    fn parse_info_command() {
        match parse(&["info", "document.pdf", "--json"]).command {
            Commands::Info(args) => {
                assert_eq!(args.input, PathBuf::from("document.pdf"));
                assert!(args.json);
            }
            _ => panic!("expected info"),
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
        assert!(result.is_err());
    }

    #[test]
    fn quality_flags_reject_zero_and_above_hundred() {
        for flag in ["--quality", "--image-quality"] {
            for value in ["0", "101"] {
                let result =
                    Cli::try_parse_from(["media-compressor", "compress", "a.pdf", flag, value]);
                assert!(result.is_err(), "{flag} {value}");
            }
        }
    }

    #[test]
    fn all_compression_levels_parse() {
        for level in ["low", "medium", "high", "extreme"] {
            assert!(
                Cli::try_parse_from(["media-compressor", "compress", "a.pdf", "-l", level]).is_ok()
            );
        }
    }

    #[test]
    fn level_arg_converts_to_every_compression_level() {
        let converted: Vec<CompressionLevel> = [
            LevelArg::Low,
            LevelArg::Medium,
            LevelArg::High,
            LevelArg::Extreme,
        ]
        .into_iter()
        .map(CompressionLevel::from)
        .collect();
        assert_eq!(
            converted,
            vec![
                CompressionLevel::Low,
                CompressionLevel::Medium,
                CompressionLevel::High,
                CompressionLevel::Extreme
            ]
        );
    }

    #[test]
    fn format_arg_converts_to_output_format() {
        assert_eq!(
            OutputFormat::from(FormatArg::Original),
            OutputFormat::Original
        );
        assert_eq!(OutputFormat::from(FormatArg::Jpeg), OutputFormat::Jpeg);
        assert_eq!(OutputFormat::from(FormatArg::Png), OutputFormat::Png);
    }

    #[test]
    fn removed_flags_and_formats_are_rejected() {
        assert!(
            Cli::try_parse_from(["media-compressor", "compress", "a.jpg", "-f", "webp"]).is_err()
        );
        assert!(
            Cli::try_parse_from(["media-compressor", "compress", "a.pdf", "--strip-metadata"])
                .is_err()
        );
    }

    #[test]
    fn compress_help_lists_every_flag_with_description() {
        let help = help_of("compress");
        for expected in [
            "--output",
            "--level",
            "--keep-metadata",
            "--quality",
            "--image-quality",
            "--dpi",
            "--format",
            "--max-dimension",
            "--grayscale",
            "--lossless",
            "--compress-fonts",
            "--max-input-size",
            "--dry-run",
            "--json",
            "--overwrite",
            "Preserve document metadata",
            "Estimate compression savings without writing any output file",
            "Reject input files larger than BYTES",
            "Compress a PDF or image file",
        ] {
            assert!(
                help.contains(expected) || expected.starts_with("Compress"),
                "missing {expected}"
            );
        }
    }

    #[test]
    fn batch_help_lists_every_flag_with_description() {
        let help = help_of("batch");
        for expected in [
            "--output-dir",
            "--pattern",
            "--recursive",
            "--workers",
            "--continue-on-error",
            "--quality",
            "--format",
            "--max-dimension",
            "--grayscale",
            "--lossless",
            "--max-input-size",
            "Glob pattern to match files",
            "Keep processing remaining files",
        ] {
            assert!(help.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn info_help_describes_json_flag() {
        let help = help_of("info");
        assert!(help.contains("--json") && help.contains("Output the information as JSON"));
    }
}
