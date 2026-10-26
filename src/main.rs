use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;
use tracing::info;

use media_compressor::batch::processor::{self, BatchConfig};
use media_compressor::cli::args::{Cli, Commands};
use media_compressor::error::AppError;
use media_compressor::io::reader::load_pdf;
use media_compressor::strategy::{CompressorDispatcher, DispatchConfig};
use media_compressor::utils::{default_image_output_path, default_output_path};

fn main() -> Result<()> {
    let cli = Cli::parse();

    // ── Logging setup ──────────────────────────────────────────────────────
    // Respect -q/--quiet and -v/--verbose, but allow RUST_LOG to override.
    let default_filter = if cli.quiet {
        "error"
    } else if cli.verbose {
        "debug"
    } else {
        "info"
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // ── Command dispatch ───────────────────────────────────────────────────
    match cli.command {
        Commands::Compress(args) => {
            // Resolve output path.
            // For explicit format conversions (e.g. --format png), change extension.
            let output_path = args.output.clone().unwrap_or_else(|| {
                if let Some(ext) = args.format.extension() {
                    default_image_output_path(&args.input, ext)
                } else {
                    default_output_path(&args.input)
                }
            });

            // Guard against accidental overwrites.
            if !args.overwrite && !args.dry_run && output_path.exists() {
                eprintln!(
                    "{} Output file '{}' already exists. Use --overwrite to replace it.",
                    "Error:".red().bold(),
                    output_path.display()
                );
                std::process::exit(2);
            }

            // Build dispatch configuration from CLI args.
            let config = DispatchConfig {
                level: args.level,
                dry_run: args.dry_run,
                keep_metadata: args.keep_metadata,
                quality: args.quality,
                format: args.format.clone(),
                max_dimension: args.max_dimension,
                // Combine explicit --grayscale with level preset (Extreme converts to grayscale).
                grayscale: args.grayscale || args.level.convert_to_grayscale(),
                strip_metadata: args.strip_metadata,
                lossless: args.lossless,
                pdf_image_quality: args.image_quality,
                pdf_target_dpi: args.dpi,
                pdf_compress_fonts: args.compress_fonts,
            };

            info!(
                input = %args.input.display(),
                output = %output_path.display(),
                level = %args.level,
                dry_run = args.dry_run,
                "Starting compression"
            );

            // Detect file type and dispatch to the correct strategy.
            let result = CompressorDispatcher
                .compress(&config, &args.input, &output_path)
                .with_context(|| format!("Failed to compress '{}'", args.input.display()))?;

            // Print results.
            if args.json {
                let json = result
                    .to_json()
                    .with_context(|| "Failed to serialise report")?;
                println!("{}", json);
            } else {
                result.print_pretty();
            }
        }

        Commands::Batch(args) => {
            // Validate input directory.
            if !args.input_dir.is_dir() {
                eprintln!(
                    "{} '{}' is not a directory.",
                    "Error:".red().bold(),
                    args.input_dir.display()
                );
                std::process::exit(2);
            }

            // Resolve output directory.
            let output_dir = args.output_dir.clone().unwrap_or_else(|| {
                let mut name = args
                    .input_dir
                    .file_name()
                    .unwrap_or_default()
                    .to_os_string();
                name.push("_compressed");
                args.input_dir
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(name)
            });

            let config = BatchConfig {
                input_dir: args.input_dir.clone(),
                output_dir: output_dir.clone(),
                level: args.level,
                pattern: args.pattern.clone(),
                recursive: args.recursive,
                workers: args.workers,
                continue_on_error: args.continue_on_error,
                keep_metadata: args.keep_metadata,
                image_quality: args.image_quality,
                target_dpi: args.dpi,
                dry_run: args.dry_run,
                overwrite: args.overwrite,
                compress_fonts: args.compress_fonts,
            };

            info!(
                input_dir = %config.input_dir.display(),
                output_dir = %config.output_dir.display(),
                level = %config.level,
                pattern = %config.pattern,
                recursive = config.recursive,
                workers = config.workers,
                "Starting batch compression"
            );

            let report =
                processor::process_batch(&config).with_context(|| "Batch compression failed")?;

            if args.json {
                let json = report
                    .to_json()
                    .with_context(|| "Failed to serialise batch report")?;
                println!("{}", json);
            } else {
                report.print_pretty();
            }

            // Exit non-zero if any files failed and continue-on-error was set.
            if report.failed > 0 {
                std::process::exit(1);
            }
        }

        Commands::Info(args) => {
            let doc = load_pdf(&args.input).map_err(|e| match e {
                AppError::InvalidPdf(msg) => anyhow::anyhow!("Invalid PDF: {}", msg),
                AppError::EncryptedPdf => {
                    anyhow::anyhow!("The PDF '{}' is password-protected.", args.input.display())
                }
                AppError::Io(io_err) => anyhow::anyhow!("I/O error: {}", io_err),
                other => anyhow::anyhow!("{}", other),
            })?;

            let metadata = std::fs::metadata(&args.input).with_context(|| {
                format!("Cannot read file metadata for '{}'", args.input.display())
            })?;

            let page_count = doc.get_pages().len();
            let file_size = metadata.len();

            if args.json {
                let info = serde_json::json!({
                    "path": args.input.display().to_string(),
                    "file_size_bytes": file_size,
                    "page_count": page_count,
                    "pdf_version": doc.version,
                });
                println!("{}", serde_json::to_string_pretty(&info)?);
            } else {
                let separator = "─".repeat(50);
                println!("{}", separator.dimmed());
                println!("{}", "PDF Information".bold());
                println!("{}", separator.dimmed());
                println!(
                    "  {:<18} {}",
                    "File:".dimmed(),
                    args.input.display().to_string().cyan()
                );
                println!(
                    "  {:<18} {}",
                    "Size:".dimmed(),
                    humansize::format_size(file_size, humansize::DECIMAL).white()
                );
                println!(
                    "  {:<18} {}",
                    "Pages:".dimmed(),
                    page_count.to_string().white()
                );
                println!("  {:<18} {}", "PDF version:".dimmed(), doc.version.white());
                println!("{}", separator.dimmed());
            }
        }
    }

    Ok(())
}
