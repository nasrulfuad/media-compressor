use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use colored::Colorize;
use globset::{Glob, GlobMatcher};
use humansize::{format_size, DECIMAL};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::prelude::*;
use serde::Serialize;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::compressor::level::CompressionLevel;
use crate::error::AppResult;
use crate::strategy::{CompressorDispatcher, DispatchConfig, ImageOutputFormat};

/// Configuration for a batch compression run.
#[derive(Debug, Clone)]
pub struct BatchConfig {
    /// Root directory to scan for PDF files.
    pub input_dir: PathBuf,

    /// Directory where compressed files are written.
    /// Directory structure relative to `input_dir` is preserved.
    pub output_dir: PathBuf,

    /// Compression level preset.
    pub level: CompressionLevel,

    /// Glob pattern to filter files (e.g. `"*.pdf"`).
    pub pattern: String,

    /// Recurse into subdirectories.
    pub recursive: bool,

    /// Maximum number of parallel workers (0 = use rayon default = num CPUs).
    pub workers: usize,

    /// Continue processing remaining files when one fails.
    pub continue_on_error: bool,

    /// Preserve document metadata regardless of level.
    pub keep_metadata: bool,

    /// Override the JPEG image quality (1-100).
    pub image_quality: Option<u8>,

    /// Override the target DPI for image downsampling.
    pub target_dpi: Option<u32>,

    /// Estimate compression without writing output files.
    pub dry_run: bool,

    /// Overwrite existing output files.
    pub overwrite: bool,

    /// Run font deduplication and subsetting on PDF files.
    pub compress_fonts: bool,
}

/// Outcome for a single file within a batch run.
#[derive(Debug, Clone, Serialize)]
pub struct FileResult {
    /// Path to the source PDF file.
    pub path: PathBuf,
    /// Whether compression succeeded, was skipped, or failed.
    #[serde(flatten)]
    pub outcome: FileOutcome,
}

/// Success or failure of a single file compression.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum FileOutcome {
    /// Compression completed successfully.
    #[serde(rename = "ok")]
    Ok {
        /// Original file size in bytes.
        original_size: u64,
        /// Compressed file size in bytes.
        compressed_size: u64,
        /// Bytes saved (positive = smaller output).
        bytes_saved: i64,
        /// Compression ratio percentage.
        ratio: f64,
        /// Detected file type (e.g. "PDF", "JPEG", "PNG").
        file_type: String,
    },
    /// File was skipped (e.g. output already exists).
    #[serde(rename = "skipped")]
    Skipped {
        /// Human-readable reason the file was skipped.
        reason: String,
    },
    /// Compression failed with an error.
    #[serde(rename = "error")]
    Error {
        /// Human-readable error description.
        error: String,
    },
}

/// Aggregate statistics for an entire batch run.
#[derive(Debug, Clone, Serialize)]
pub struct BatchReport {
    /// Root input directory that was scanned.
    pub input_dir: PathBuf,
    /// Directory where compressed files were written.
    pub output_dir: PathBuf,
    /// Total number of files discovered by the glob pattern.
    pub total_files: usize,
    /// Number of files compressed successfully.
    pub succeeded: usize,
    /// Number of files skipped (e.g. output already exists).
    pub skipped: usize,
    /// Number of files that failed to compress.
    pub failed: usize,
    /// Sum of original file sizes across all succeeded files.
    pub total_original_bytes: u64,
    /// Sum of compressed file sizes across all succeeded files.
    pub total_compressed_bytes: u64,
    /// `total_original_bytes - total_compressed_bytes` (positive = space saved).
    pub total_bytes_saved: i64,
    /// `(1 - total_compressed / total_original) * 100`. Positive means smaller output.
    pub overall_ratio: f64,
    /// Wall-clock time for the entire batch run in milliseconds.
    pub duration_ms: u64,
    /// When `true`, no output files were written; sizes are estimates.
    pub dry_run: bool,
    /// Per-file results in sorted order.
    pub files: Vec<FileResult>,
}

impl BatchReport {
    /// Print a coloured summary to stdout.
    pub fn print_pretty(&self) {
        let sep = "─".repeat(60);
        println!("{}", sep.dimmed());
        println!(
            "{} {}",
            "Batch Compression Result".bold(),
            if self.dry_run {
                "(dry run)".yellow().to_string()
            } else {
                String::new()
            }
        );
        println!("{}", sep.dimmed());

        println!(
            "  {:<22} {}",
            "Input dir:".dimmed(),
            self.input_dir.display().to_string().cyan()
        );
        if !self.dry_run {
            println!(
                "  {:<22} {}",
                "Output dir:".dimmed(),
                self.output_dir.display().to_string().cyan()
            );
        }
        println!(
            "  {:<22} {}",
            "Total files:".dimmed(),
            self.total_files.to_string().white()
        );
        println!(
            "  {:<22} {}",
            "Succeeded:".dimmed(),
            self.succeeded.to_string().green()
        );
        if self.skipped > 0 {
            println!(
                "  {:<22} {}",
                "Skipped:".dimmed(),
                self.skipped.to_string().yellow()
            );
        }
        if self.failed > 0 {
            println!(
                "  {:<22} {}",
                "Failed:".dimmed(),
                self.failed.to_string().red()
            );
        }
        println!(
            "  {:<22} {}",
            "Total original:".dimmed(),
            format_size(self.total_original_bytes, DECIMAL).white()
        );

        let size_label = if self.dry_run {
            "Est. total:"
        } else {
            "Total compressed:"
        };
        let compressed_str = format_size(self.total_compressed_bytes, DECIMAL);
        let compressed_colored = if self.total_bytes_saved >= 0 {
            compressed_str.green()
        } else {
            compressed_str.red()
        };
        println!("  {:<22} {}", size_label.dimmed(), compressed_colored);

        let saved_str = if self.total_bytes_saved >= 0 {
            format!("-{}", format_size(self.total_bytes_saved as u64, DECIMAL))
                .green()
                .to_string()
        } else {
            format!(
                "+{}",
                format_size(self.total_bytes_saved.unsigned_abs(), DECIMAL)
            )
            .red()
            .to_string()
        };
        println!("  {:<22} {}", "Total saved:".dimmed(), saved_str);

        let ratio_str = format!("{:.1}%", self.overall_ratio);
        let ratio_colored = match self.overall_ratio {
            r if r >= 30.0 => ratio_str.green(),
            r if r >= 10.0 => ratio_str.yellow(),
            r if r >= 0.0 => ratio_str.white(),
            _ => ratio_str.red(),
        };
        println!("  {:<22} {}", "Overall reduction:".dimmed(), ratio_colored);

        println!(
            "  {:<22} {}ms",
            "Duration:".dimmed(),
            self.duration_ms.to_string().white()
        );
        println!("{}", sep.dimmed());

        // Per-file errors/skips summary.
        let failures: Vec<&FileResult> = self
            .files
            .iter()
            .filter(|f| matches!(f.outcome, FileOutcome::Error { .. }))
            .collect();
        if !failures.is_empty() {
            println!("\n{}", "Failed files:".red().bold());
            for f in failures {
                if let FileOutcome::Error { ref error } = f.outcome {
                    println!(
                        "  {} {}",
                        f.path.display().to_string().red(),
                        error.dimmed()
                    );
                }
            }
        }
    }

    /// Serialise the batch report to a JSON string.
    pub fn to_json(&self) -> AppResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| {
            crate::error::AppError::Custom(format!("JSON serialisation failed: {}", e))
        })
    }
}

/// Discover PDF files under `input_dir` matching `pattern`.
fn discover_files(input_dir: &Path, pattern: &str, recursive: bool) -> AppResult<Vec<PathBuf>> {
    let glob = Glob::new(pattern)
        .map_err(|e| crate::error::AppError::Custom(format!("Invalid glob pattern: {}", e)))?;
    let matcher: GlobMatcher = glob.compile_matcher();

    let max_depth = if recursive { usize::MAX } else { 1 };

    let mut files: Vec<PathBuf> = WalkDir::new(input_dir)
        .max_depth(max_depth)
        .follow_links(true)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|name| matcher.is_match(name))
                .unwrap_or(false)
        })
        .map(|entry| entry.into_path())
        .collect();

    files.sort();
    Ok(files)
}

/// Compute the output path for a file, preserving directory structure.
fn output_path_for(file: &Path, input_dir: &Path, output_dir: &Path) -> PathBuf {
    let relative = file.strip_prefix(input_dir).unwrap_or(file);
    output_dir.join(relative)
}

/// Run a batch compression job.
pub fn process_batch(config: &BatchConfig) -> AppResult<BatchReport> {
    let started_at = Instant::now();

    // Discover files.
    let files = discover_files(&config.input_dir, &config.pattern, config.recursive)?;
    let total_files = files.len();

    info!(
        dir = %config.input_dir.display(),
        pattern = %config.pattern,
        recursive = config.recursive,
        files_found = total_files,
        "Batch scan complete"
    );

    if total_files == 0 {
        return Ok(BatchReport {
            input_dir: config.input_dir.clone(),
            output_dir: config.output_dir.clone(),
            total_files: 0,
            succeeded: 0,
            skipped: 0,
            failed: 0,
            total_original_bytes: 0,
            total_compressed_bytes: 0,
            total_bytes_saved: 0,
            overall_ratio: 0.0,
            duration_ms: started_at.elapsed().as_millis() as u64,
            dry_run: config.dry_run,
            files: Vec::new(),
        });
    }

    // Configure rayon thread pool if workers is non-zero.
    if config.workers > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(config.workers)
            .build_global()
            .ok(); // Ignore if already built — only the first call wins.
    }

    // Progress bar.
    let multi = MultiProgress::new();
    let overall_pb = multi.add(ProgressBar::new(total_files as u64));
    overall_pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    overall_pb.set_message("Compressing...");

    // Atomic counters for aggregation.
    let succeeded = AtomicU64::new(0);
    let skipped = AtomicU64::new(0);
    let failed = AtomicU64::new(0);
    let total_original = AtomicU64::new(0);
    let total_compressed = AtomicU64::new(0);
    let results: Mutex<Vec<FileResult>> = Mutex::new(Vec::with_capacity(total_files));

    // Process files in parallel.
    files.par_iter().for_each(|file| {
        let output = output_path_for(file, &config.input_dir, &config.output_dir);

        // Skip if output exists and overwrite is not enabled.
        if !config.overwrite && !config.dry_run && output.exists() {
            let result = FileResult {
                path: file.clone(),
                outcome: FileOutcome::Skipped {
                    reason: "output already exists".to_string(),
                },
            };
            skipped.fetch_add(1, Ordering::Relaxed);
            results.lock().unwrap().push(result);
            overall_pb.inc(1);
            return;
        }

        // Build dispatch config from batch config.
        let dispatch_config = DispatchConfig {
            level: config.level,
            dry_run: config.dry_run,
            keep_metadata: config.keep_metadata,
            quality: None,
            format: ImageOutputFormat::Original,
            max_dimension: None,
            grayscale: false,
            strip_metadata: false,
            lossless: false,
            pdf_image_quality: config.image_quality,
            pdf_target_dpi: config.target_dpi,
            pdf_compress_fonts: config.compress_fonts,
        };

        match CompressorDispatcher.compress(&dispatch_config, file, &output) {
            Ok(result) => {
                // Extract common fields from either report type.
                let (original_size, compressed_size, bytes_saved, ratio, file_type) = match &result
                {
                    crate::strategy::CompressResult::Pdf(r) => (
                        r.original_size,
                        r.compressed_size,
                        r.bytes_saved,
                        r.ratio,
                        "PDF".to_string(),
                    ),
                    crate::strategy::CompressResult::Image(r) => (
                        r.original_size,
                        r.compressed_size,
                        r.bytes_saved,
                        r.ratio,
                        r.input_format.clone(),
                    ),
                };
                debug!(
                    file = %file.display(),
                    ratio = format!("{:.1}%", ratio),
                    "File compressed"
                );
                total_original.fetch_add(original_size, Ordering::Relaxed);
                total_compressed.fetch_add(compressed_size, Ordering::Relaxed);
                succeeded.fetch_add(1, Ordering::Relaxed);
                results.lock().unwrap().push(FileResult {
                    path: file.clone(),
                    outcome: FileOutcome::Ok {
                        original_size,
                        compressed_size,
                        bytes_saved,
                        ratio,
                        file_type,
                    },
                });
            }
            Err(e) => {
                warn!(file = %file.display(), error = %e, "Compression failed");
                failed.fetch_add(1, Ordering::Relaxed);
                results.lock().unwrap().push(FileResult {
                    path: file.clone(),
                    outcome: FileOutcome::Error {
                        error: e.to_string(),
                    },
                });
                if !config.continue_on_error {
                    // We can't abort par_iter early, but we log prominently.
                    warn!("continue-on-error is disabled; remaining files will still be processed by rayon");
                }
            }
        }

        overall_pb.inc(1);
    });

    overall_pb.finish_with_message("Done");

    let succeeded = succeeded.load(Ordering::Relaxed) as usize;
    let skipped = skipped.load(Ordering::Relaxed) as usize;
    let failed = failed.load(Ordering::Relaxed) as usize;
    let total_original = total_original.load(Ordering::Relaxed);
    let total_compressed = total_compressed.load(Ordering::Relaxed);
    let total_bytes_saved = total_original as i64 - total_compressed as i64;
    let overall_ratio = if total_original > 0 {
        (1.0 - total_compressed as f64 / total_original as f64) * 100.0
    } else {
        0.0
    };

    let mut file_results = results.into_inner().unwrap();
    file_results.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(BatchReport {
        input_dir: config.input_dir.clone(),
        output_dir: config.output_dir.clone(),
        total_files,
        succeeded,
        skipped,
        failed,
        total_original_bytes: total_original,
        total_compressed_bytes: total_compressed,
        total_bytes_saved,
        overall_ratio,
        duration_ms: started_at.elapsed().as_millis() as u64,
        dry_run: config.dry_run,
        files: file_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn discover_files_empty_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let files = discover_files(dir.path(), "*.pdf", false).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn discover_files_finds_matching_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.pdf"), b"fake").unwrap();
        fs::write(dir.path().join("b.txt"), b"fake").unwrap();
        fs::write(dir.path().join("c.pdf"), b"fake").unwrap();

        let files = discover_files(dir.path(), "*.pdf", false).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.extension().unwrap() == "pdf"));
    }

    #[test]
    fn discover_files_respects_recursive_flag() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(dir.path().join("top.pdf"), b"fake").unwrap();
        fs::write(sub.join("nested.pdf"), b"fake").unwrap();

        let flat = discover_files(dir.path(), "*.pdf", false).unwrap();
        assert_eq!(flat.len(), 1, "Non-recursive should only find top-level");

        let deep = discover_files(dir.path(), "*.pdf", true).unwrap();
        assert_eq!(deep.len(), 2, "Recursive should find nested files");
    }

    #[test]
    fn output_path_preserves_structure() {
        let input_dir = Path::new("/data/pdfs");
        let output_dir = Path::new("/data/compressed");
        let file = Path::new("/data/pdfs/sub/doc.pdf");

        let out = output_path_for(file, input_dir, output_dir);
        assert_eq!(out, PathBuf::from("/data/compressed/sub/doc.pdf"));
    }

    #[test]
    fn batch_report_to_json_produces_valid_json() {
        let report = BatchReport {
            input_dir: PathBuf::from("/tmp/in"),
            output_dir: PathBuf::from("/tmp/out"),
            total_files: 2,
            succeeded: 1,
            skipped: 0,
            failed: 1,
            total_original_bytes: 1_000_000,
            total_compressed_bytes: 500_000,
            total_bytes_saved: 500_000,
            overall_ratio: 50.0,
            duration_ms: 1234,
            dry_run: false,
            files: Vec::new(),
        };
        let json = report.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["total_files"], 2);
        assert_eq!(parsed["succeeded"], 1);
    }

    #[test]
    fn batch_report_print_pretty_does_not_panic() {
        let report = BatchReport {
            input_dir: PathBuf::from("/tmp/in"),
            output_dir: PathBuf::from("/tmp/out"),
            total_files: 0,
            succeeded: 0,
            skipped: 0,
            failed: 0,
            total_original_bytes: 0,
            total_compressed_bytes: 0,
            total_bytes_saved: 0,
            overall_ratio: 0.0,
            duration_ms: 0,
            dry_run: false,
            files: Vec::new(),
        };
        report.print_pretty();
    }

    #[test]
    fn process_batch_empty_dir_returns_zero_files() {
        let dir = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        let config = BatchConfig {
            input_dir: dir.path().to_path_buf(),
            output_dir: out.path().to_path_buf(),
            level: CompressionLevel::Medium,
            pattern: "*.pdf".to_string(),
            recursive: false,
            workers: 1,
            continue_on_error: true,
            keep_metadata: false,
            image_quality: None,
            target_dpi: None,
            dry_run: false,
            overwrite: false,
            compress_fonts: false,
        };
        let report = process_batch(&config).unwrap();
        assert_eq!(report.total_files, 0);
        assert_eq!(report.succeeded, 0);
    }
}
