use colored::Colorize;
use humansize::{format_size, DECIMAL};
use serde::Serialize;
use std::path::PathBuf;

use crate::error::AppResult;

/// Statistics produced by a single compression run.
#[derive(Debug, Clone, Serialize)]
pub struct CompressionReport {
    /// Path of the source PDF.
    pub input_path: PathBuf,

    /// Path of the output PDF (may be the same as input if overwriting).
    pub output_path: PathBuf,

    /// File size of the source PDF in bytes.
    pub original_size: u64,

    /// File size of the compressed PDF in bytes (or estimate when dry-run).
    pub compressed_size: u64,

    /// `original_size - compressed_size` (positive = space saved, negative = larger).
    pub bytes_saved: i64,

    /// `(1 - compressed / original) * 100`. Positive means smaller output.
    pub ratio: f64,

    /// Wall-clock time taken to complete the operation in milliseconds.
    pub duration_ms: u64,

    /// When `true`, no output file was written; sizes are estimates.
    pub dry_run: bool,
}

impl CompressionReport {
    /// Print a coloured, human-readable summary to stdout.
    pub fn print_pretty(&self) {
        let separator = "─".repeat(50);

        println!("{}", separator.dimmed());
        println!(
            "{} {}",
            "PDF Compressor Result".bold(),
            if self.dry_run {
                "(dry run — no file written)".yellow().to_string()
            } else {
                String::new()
            }
        );
        println!("{}", separator.dimmed());

        println!(
            "  {:<18} {}",
            "Input:".dimmed(),
            self.input_path.display().to_string().cyan()
        );

        if !self.dry_run {
            println!(
                "  {:<18} {}",
                "Output:".dimmed(),
                self.output_path.display().to_string().cyan()
            );
        }

        println!(
            "  {:<18} {}",
            "Original size:".dimmed(),
            format_size(self.original_size, DECIMAL).white()
        );

        let size_label = if self.dry_run {
            "Est. size:"
        } else {
            "Compressed size:"
        };

        let size_str = format_size(self.compressed_size, DECIMAL);
        let size_colored = if self.bytes_saved >= 0 {
            size_str.green()
        } else {
            size_str.red()
        };
        println!("  {:<18} {}", size_label.dimmed(), size_colored);

        let saved_str = if self.bytes_saved >= 0 {
            format!("-{}", format_size(self.bytes_saved as u64, DECIMAL))
                .green()
                .to_string()
        } else {
            format!("+{}", format_size(self.bytes_saved.unsigned_abs(), DECIMAL))
                .red()
                .to_string()
        };
        println!("  {:<18} {}", "Bytes saved:".dimmed(), saved_str);

        let ratio_str = format!("{:.1}%", self.ratio);
        let ratio_colored = match self.ratio {
            r if r >= 30.0 => ratio_str.green(),
            r if r >= 10.0 => ratio_str.yellow(),
            r if r >= 0.0 => ratio_str.white(),
            _ => ratio_str.red(),
        };
        println!("  {:<18} {}", "Reduction ratio:".dimmed(), ratio_colored);

        println!(
            "  {:<18} {}ms",
            "Duration:".dimmed(),
            self.duration_ms.to_string().white()
        );

        println!("{}", separator.dimmed());
    }

    /// Serialise the report to a JSON string.
    pub fn to_json(&self) -> AppResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| {
            crate::error::AppError::Custom(format!("JSON serialisation failed: {}", e))
        })
    }
}

/// Statistics produced by a single image compression run.
#[derive(Debug, Clone, Serialize)]
pub struct ImageCompressionReport {
    /// Path of the source image.
    pub input_path: PathBuf,

    /// Path of the output image (may be same as input if overwriting).
    pub output_path: PathBuf,

    /// Detected input format (e.g. `"JPEG"`, `"PNG"`).
    pub input_format: String,

    /// Actual output format after any conversion.
    pub output_format: String,

    /// File size of the source image in bytes.
    pub original_size: u64,

    /// File size of the output image in bytes (or estimate when dry-run).
    pub compressed_size: u64,

    /// `original_size - compressed_size`.
    pub bytes_saved: i64,

    /// `(1 - compressed / original) * 100`.
    pub ratio: f64,

    /// Width × height of the source image.
    pub original_dimensions: (u32, u32),

    /// Width × height of the output image (may differ after resize).
    pub output_dimensions: (u32, u32),

    /// Wall-clock time in milliseconds.
    pub duration_ms: u64,

    /// When `true`, no output file was written; sizes are estimates.
    pub dry_run: bool,
}

impl ImageCompressionReport {
    /// Print a coloured, human-readable summary to stdout.
    pub fn print_pretty(&self) {
        let separator = "─".repeat(50);

        println!("{}", separator.dimmed());
        println!(
            "{} {}",
            "Image Compressor Result".bold(),
            if self.dry_run {
                "(dry run — no file written)".yellow().to_string()
            } else {
                String::new()
            }
        );
        println!("{}", separator.dimmed());

        println!(
            "  {:<20} {}",
            "Input:".dimmed(),
            self.input_path.display().to_string().cyan()
        );

        if !self.dry_run {
            println!(
                "  {:<20} {}",
                "Output:".dimmed(),
                self.output_path.display().to_string().cyan()
            );
        }

        let fmt_changed = self.input_format != self.output_format;
        if fmt_changed {
            println!(
                "  {:<20} {} → {}",
                "Format:".dimmed(),
                self.input_format.white(),
                self.output_format.green()
            );
        } else {
            println!("  {:<20} {}", "Format:".dimmed(), self.input_format.white());
        }

        let dims_changed = self.original_dimensions != self.output_dimensions;
        if dims_changed {
            println!(
                "  {:<20} {}×{} → {}×{}",
                "Dimensions:".dimmed(),
                self.original_dimensions.0,
                self.original_dimensions.1,
                self.output_dimensions.0.to_string().green(),
                self.output_dimensions.1.to_string().green()
            );
        } else {
            println!(
                "  {:<20} {}×{}",
                "Dimensions:".dimmed(),
                self.original_dimensions.0,
                self.original_dimensions.1
            );
        }

        println!(
            "  {:<20} {}",
            "Original size:".dimmed(),
            format_size(self.original_size, DECIMAL).white()
        );

        let size_label = if self.dry_run {
            "Est. size:"
        } else {
            "Compressed size:"
        };
        let size_str = format_size(self.compressed_size, DECIMAL);
        let size_colored = if self.bytes_saved >= 0 {
            size_str.green()
        } else {
            size_str.red()
        };
        println!("  {:<20} {}", size_label.dimmed(), size_colored);

        let saved_str = if self.bytes_saved >= 0 {
            format!("-{}", format_size(self.bytes_saved as u64, DECIMAL))
                .green()
                .to_string()
        } else {
            format!("+{}", format_size(self.bytes_saved.unsigned_abs(), DECIMAL))
                .red()
                .to_string()
        };
        println!("  {:<20} {}", "Bytes saved:".dimmed(), saved_str);

        let ratio_str = format!("{:.1}%", self.ratio);
        let ratio_colored = match self.ratio {
            r if r >= 30.0 => ratio_str.green(),
            r if r >= 10.0 => ratio_str.yellow(),
            r if r >= 0.0 => ratio_str.white(),
            _ => ratio_str.red(),
        };
        println!("  {:<20} {}", "Reduction ratio:".dimmed(), ratio_colored);

        println!(
            "  {:<20} {}ms",
            "Duration:".dimmed(),
            self.duration_ms.to_string().white()
        );

        println!("{}", separator.dimmed());
    }

    /// Serialise the report to a JSON string.
    pub fn to_json(&self) -> AppResult<String> {
        serde_json::to_string_pretty(self).map_err(|e| {
            crate::error::AppError::Custom(format!("JSON serialisation failed: {}", e))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_report() -> CompressionReport {
        CompressionReport {
            input_path: PathBuf::from("input.pdf"),
            output_path: PathBuf::from("output.pdf"),
            original_size: 1_000_000,
            compressed_size: 600_000,
            bytes_saved: 400_000,
            ratio: 40.0,
            duration_ms: 250,
            dry_run: false,
        }
    }

    #[test]
    fn ratio_computed_correctly() {
        let r = sample_report();
        // 1 - 600_000/1_000_000 = 0.4 → 40%
        let expected = (1.0 - 600_000.0_f64 / 1_000_000.0) * 100.0;
        assert!((r.ratio - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn to_json_produces_valid_json() {
        let r = sample_report();
        let json = r.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["original_size"], 1_000_000u64);
        assert_eq!(parsed["compressed_size"], 600_000u64);
        assert_eq!(parsed["bytes_saved"], 400_000i64);
        assert!(!parsed["dry_run"].as_bool().unwrap());
    }

    #[test]
    fn to_json_includes_all_fields() {
        let r = sample_report();
        let json = r.to_json().unwrap();
        for field in &[
            "input_path",
            "output_path",
            "original_size",
            "compressed_size",
            "bytes_saved",
            "ratio",
            "duration_ms",
            "dry_run",
        ] {
            assert!(json.contains(field), "JSON missing field: {}", field);
        }
    }

    #[test]
    fn print_pretty_does_not_panic() {
        // Smoke-test that the method runs without panicking.
        let r = sample_report();
        r.print_pretty();
    }

    #[test]
    fn print_pretty_dry_run_does_not_panic() {
        let mut r = sample_report();
        r.dry_run = true;
        r.print_pretty();
    }

    #[test]
    fn negative_bytes_saved_handled_gracefully() {
        let mut r = sample_report();
        r.compressed_size = 1_200_000;
        r.bytes_saved = -200_000;
        r.ratio = -20.0;
        r.print_pretty(); // must not panic
    }

    // ── ImageCompressionReport tests ─────────────────────────────────────

    fn sample_image_report() -> ImageCompressionReport {
        ImageCompressionReport {
            input_path: PathBuf::from("photo.jpg"),
            output_path: PathBuf::from("photo_compressed.jpg"),
            input_format: "JPEG".to_string(),
            output_format: "JPEG".to_string(),
            original_size: 500_000,
            compressed_size: 300_000,
            bytes_saved: 200_000,
            ratio: 40.0,
            original_dimensions: (1920, 1080),
            output_dimensions: (1920, 1080),
            duration_ms: 120,
            dry_run: false,
        }
    }

    #[test]
    fn image_report_print_pretty_does_not_panic() {
        sample_image_report().print_pretty();
    }

    #[test]
    fn image_report_print_pretty_dry_run_does_not_panic() {
        let mut r = sample_image_report();
        r.dry_run = true;
        r.print_pretty();
    }

    #[test]
    fn image_report_print_pretty_format_conversion_does_not_panic() {
        let mut r = sample_image_report();
        r.input_format = "PNG".to_string();
        r.output_format = "JPEG".to_string();
        r.print_pretty();
    }

    #[test]
    fn image_report_print_pretty_resize_does_not_panic() {
        let mut r = sample_image_report();
        r.output_dimensions = (960, 540);
        r.print_pretty();
    }

    #[test]
    fn image_report_to_json_produces_valid_json() {
        let r = sample_image_report();
        let json = r.to_json().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["original_size"], 500_000u64);
        assert_eq!(parsed["input_format"], "JPEG");
    }

    #[test]
    fn image_report_to_json_includes_all_fields() {
        let r = sample_image_report();
        let json = r.to_json().unwrap();
        for field in &[
            "input_path",
            "output_path",
            "input_format",
            "output_format",
            "original_size",
            "compressed_size",
            "bytes_saved",
            "ratio",
            "original_dimensions",
            "output_dimensions",
            "duration_ms",
            "dry_run",
        ] {
            assert!(json.contains(field), "JSON missing field: {field}");
        }
    }
}
