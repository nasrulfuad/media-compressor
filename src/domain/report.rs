use std::path::PathBuf;

use serde::Serialize;

use super::media::{Dimensions, ImageFormat};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SizeStats {
    pub original_size: u64,
    pub compressed_size: u64,
    pub bytes_saved: i64,
    pub ratio: f64,
}

impl SizeStats {
    pub fn new(original_size: u64, compressed_size: u64) -> Self {
        let ratio = if original_size > 0 {
            (1.0 - compressed_size as f64 / original_size as f64) * 100.0
        } else {
            0.0
        };
        Self {
            original_size,
            compressed_size,
            bytes_saved: original_size as i64 - compressed_size as i64,
            ratio,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PassStat {
    pub pass: &'static str,
    pub bytes_saved: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MediaDetails {
    Pdf {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        passes: Vec<PassStat>,
    },
    Image {
        input_format: ImageFormat,
        output_format: ImageFormat,
        original_dimensions: Dimensions,
        output_dimensions: Dimensions,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileReport {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    #[serde(flatten)]
    pub sizes: SizeStats,
    pub kept_original: bool,
    pub duration_ms: u64,
    pub dry_run: bool,
    #[serde(flatten)]
    pub details: MediaDetails,
}

impl FileReport {
    pub fn file_type_label(&self) -> String {
        match &self.details {
            MediaDetails::Pdf { .. } => "PDF".to_string(),
            MediaDetails::Image { input_format, .. } => input_format.display_name().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status")]
pub enum FileOutcome {
    #[serde(rename = "ok")]
    Ok {
        #[serde(flatten)]
        sizes: SizeStats,
        file_type: String,
    },
    #[serde(rename = "skipped")]
    Skipped { reason: String },
    #[serde(rename = "error")]
    Error { error: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub outcome: FileOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchContext {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub duration_ms: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchReport {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub total_files: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_original_bytes: u64,
    pub total_compressed_bytes: u64,
    pub total_bytes_saved: i64,
    pub overall_ratio: f64,
    pub duration_ms: u64,
    pub dry_run: bool,
    pub files: Vec<FileResult>,
}

impl BatchReport {
    pub fn from_results(context: BatchContext, files: Vec<FileResult>) -> Self {
        let mut succeeded = 0;
        let mut skipped = 0;
        let mut failed = 0;
        let mut total_original = 0u64;
        let mut total_compressed = 0u64;
        for file in &files {
            match &file.outcome {
                FileOutcome::Ok { sizes, .. } => {
                    succeeded += 1;
                    total_original += sizes.original_size;
                    total_compressed += sizes.compressed_size;
                }
                FileOutcome::Skipped { .. } => skipped += 1,
                FileOutcome::Error { .. } => failed += 1,
            }
        }
        let totals = SizeStats::new(total_original, total_compressed);
        Self {
            input_dir: context.input_dir,
            output_dir: context.output_dir,
            total_files: files.len(),
            succeeded,
            skipped,
            failed,
            total_original_bytes: total_original,
            total_compressed_bytes: total_compressed,
            total_bytes_saved: totals.bytes_saved,
            overall_ratio: totals.ratio,
            duration_ms: context.duration_ms,
            dry_run: context.dry_run,
            files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InfoReport {
    pub path: String,
    pub file_size_bytes: u64,
    pub page_count: usize,
    pub pdf_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_report() -> FileReport {
        FileReport {
            input_path: PathBuf::from("in.pdf"),
            output_path: PathBuf::from("out.pdf"),
            sizes: SizeStats::new(1_000_000, 500_000),
            kept_original: false,
            duration_ms: 12,
            dry_run: false,
            details: MediaDetails::Pdf { passes: Vec::new() },
        }
    }

    fn image_report() -> FileReport {
        FileReport {
            input_path: PathBuf::from("in.png"),
            output_path: PathBuf::from("out.png"),
            sizes: SizeStats::new(100, 80),
            kept_original: false,
            duration_ms: 3,
            dry_run: true,
            details: MediaDetails::Image {
                input_format: ImageFormat::Png,
                output_format: ImageFormat::Jpeg,
                original_dimensions: Dimensions {
                    width: 64,
                    height: 32,
                },
                output_dimensions: Dimensions {
                    width: 32,
                    height: 16,
                },
            },
        }
    }

    #[test]
    fn size_stats_computes_saved_bytes_and_ratio() {
        let stats = SizeStats::new(1_000_000, 500_000);
        assert_eq!(stats.bytes_saved, 500_000);
        assert_eq!(stats.ratio, 50.0);
    }

    #[test]
    fn size_stats_reports_negative_savings_when_larger() {
        let stats = SizeStats::new(100, 150);
        assert_eq!(stats.bytes_saved, -50);
        assert_eq!(stats.ratio, -50.0);
    }

    #[test]
    fn size_stats_zero_original_has_zero_ratio() {
        assert_eq!(SizeStats::new(0, 0).ratio, 0.0);
    }

    #[test]
    fn pdf_report_json_omits_empty_passes_and_flattens_sizes() {
        let json = serde_json::to_value(pdf_report()).unwrap();
        assert_eq!(json["bytes_saved"], 500_000);
        assert_eq!(json["original_size"], 1_000_000);
        assert_eq!(json["compressed_size"], 500_000);
        assert_eq!(json["ratio"], 50.0);
        assert_eq!(json["kept_original"], false);
        assert_eq!(json["dry_run"], false);
        assert!(json.get("passes").is_none());
    }

    #[test]
    fn pdf_report_json_includes_passes_when_present() {
        let mut report = pdf_report();
        report.details = MediaDetails::Pdf {
            passes: vec![PassStat {
                pass: "streams",
                bytes_saved: 10,
            }],
        };
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["passes"][0]["pass"], "streams");
        assert_eq!(json["passes"][0]["bytes_saved"], 10);
    }

    #[test]
    fn image_report_json_flattens_formats_and_dimension_pairs() {
        let json = serde_json::to_value(image_report()).unwrap();
        assert_eq!(json["input_format"], "PNG");
        assert_eq!(json["output_format"], "JPEG");
        assert_eq!(json["original_dimensions"], serde_json::json!([64, 32]));
        assert_eq!(json["output_dimensions"], serde_json::json!([32, 16]));
        assert_eq!(json["dry_run"], true);
    }

    #[test]
    fn file_type_label_for_pdf_and_image() {
        assert_eq!(pdf_report().file_type_label(), "PDF");
        assert_eq!(image_report().file_type_label(), "PNG");
    }

    fn batch_context() -> BatchContext {
        BatchContext {
            input_dir: PathBuf::from("in"),
            output_dir: PathBuf::from("out"),
            duration_ms: 7,
            dry_run: false,
        }
    }

    #[test]
    fn batch_report_from_results_aggregates_counts_and_sizes() {
        let files = vec![
            FileResult {
                path: PathBuf::from("a.pdf"),
                outcome: FileOutcome::Ok {
                    sizes: SizeStats::new(1000, 400),
                    file_type: "PDF".into(),
                },
            },
            FileResult {
                path: PathBuf::from("b.png"),
                outcome: FileOutcome::Ok {
                    sizes: SizeStats::new(1000, 600),
                    file_type: "PNG".into(),
                },
            },
            FileResult {
                path: PathBuf::from("c.pdf"),
                outcome: FileOutcome::Skipped {
                    reason: "output already exists".into(),
                },
            },
            FileResult {
                path: PathBuf::from("d.pdf"),
                outcome: FileOutcome::Error {
                    error: "boom".into(),
                },
            },
        ];
        let report = BatchReport::from_results(batch_context(), files);
        assert_eq!(report.total_files, 4);
        assert_eq!(report.succeeded, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.total_original_bytes, 2000);
        assert_eq!(report.total_compressed_bytes, 1000);
        assert_eq!(report.total_bytes_saved, 1000);
        assert_eq!(report.overall_ratio, 50.0);
        assert_eq!(report.duration_ms, 7);
    }

    #[test]
    fn batch_report_with_no_files_is_empty() {
        let report = BatchReport::from_results(batch_context(), Vec::new());
        assert_eq!(report.total_files, 0);
        assert_eq!(report.overall_ratio, 0.0);
        assert!(report.files.is_empty());
    }

    #[test]
    fn file_result_json_flattens_status_and_sizes() {
        let result = FileResult {
            path: PathBuf::from("a.pdf"),
            outcome: FileOutcome::Ok {
                sizes: SizeStats::new(10, 5),
                file_type: "PDF".into(),
            },
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["bytes_saved"], 5);
        assert_eq!(json["file_type"], "PDF");
    }

    #[test]
    fn error_file_result_json_has_error_message() {
        let result = FileResult {
            path: PathBuf::from("a.pdf"),
            outcome: FileOutcome::Error {
                error: "bad".into(),
            },
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"], "bad");
    }
}
