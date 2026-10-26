use std::fmt::Write;

use colored::{ColoredString, Colorize};
use humansize::{format_size, DECIMAL};

use crate::domain::media::Dimensions;
use crate::domain::report::{
    BatchReport, FileOutcome, FileReport, InfoReport, MediaDetails, SizeStats,
};

const FILE_REPORT_SEPARATOR_WIDTH: usize = 50;
const BATCH_REPORT_SEPARATOR_WIDTH: usize = 60;
const FILE_LABEL_WIDTH: usize = 20;
const BATCH_LABEL_WIDTH: usize = 22;
const INFO_LABEL_WIDTH: usize = 18;
const GOOD_RATIO_PERCENT: f64 = 30.0;
const OK_RATIO_PERCENT: f64 = 10.0;

fn separator(width: usize) -> String {
    "─".repeat(width).dimmed().to_string()
}

fn color_ratio(ratio: f64) -> ColoredString {
    let text = format!("{ratio:.1}%");
    if ratio >= GOOD_RATIO_PERCENT {
        text.green()
    } else if ratio >= OK_RATIO_PERCENT {
        text.yellow()
    } else if ratio >= 0.0 {
        text.white()
    } else {
        text.red()
    }
}

fn format_saved(bytes_saved: i64) -> String {
    if bytes_saved >= 0 {
        format!("-{}", format_size(bytes_saved as u64, DECIMAL))
            .green()
            .to_string()
    } else {
        format!("+{}", format_size(bytes_saved.unsigned_abs(), DECIMAL))
            .red()
            .to_string()
    }
}

fn color_size(size: u64, bytes_saved: i64) -> ColoredString {
    let text = format_size(size, DECIMAL);
    if bytes_saved >= 0 {
        text.green()
    } else {
        text.red()
    }
}

fn write_row(out: &mut String, width: usize, label: &str, value: impl std::fmt::Display) {
    let _ = writeln!(out, "  {:<width$} {}", label.dimmed(), value, width = width);
}

fn file_row(out: &mut String, label: &str, value: impl std::fmt::Display) {
    write_row(out, FILE_LABEL_WIDTH, label, value);
}

fn batch_row(out: &mut String, label: &str, value: impl std::fmt::Display) {
    write_row(out, BATCH_LABEL_WIDTH, label, value);
}

fn info_row(out: &mut String, label: &str, value: impl std::fmt::Display) {
    write_row(out, INFO_LABEL_WIDTH, label, value);
}

fn dry_run_suffix(dry_run: bool, text: &str) -> String {
    if dry_run {
        text.yellow().to_string()
    } else {
        String::new()
    }
}

fn media_title(details: &MediaDetails) -> &'static str {
    match details {
        MediaDetails::Pdf { .. } => "PDF Compressor Result",
        MediaDetails::Image { .. } => "Image Compressor Result",
    }
}

fn write_image_rows(out: &mut String, details: &MediaDetails) {
    let MediaDetails::Image {
        input_format,
        output_format,
        original_dimensions,
        output_dimensions,
    } = details
    else {
        return;
    };
    if input_format == output_format {
        file_row(out, "Format:", input_format.display_name().white());
    } else {
        let change = format!(
            "{} → {}",
            input_format.display_name().white(),
            output_format.display_name().green()
        );
        file_row(out, "Format:", change);
    }
    file_row(
        out,
        "Dimensions:",
        format_dimensions(*original_dimensions, *output_dimensions),
    );
}

fn format_dimensions(original: Dimensions, output: Dimensions) -> String {
    if original == output {
        return format!("{}×{}", original.width, original.height);
    }
    format!(
        "{}×{} → {}×{}",
        original.width,
        original.height,
        output.width.to_string().green(),
        output.height.to_string().green()
    )
}

fn write_size_rows(out: &mut String, sizes: &SizeStats, dry_run: bool) {
    file_row(
        out,
        "Original size:",
        format_size(sizes.original_size, DECIMAL).white(),
    );
    let label = if dry_run {
        "Est. size:"
    } else {
        "Compressed size:"
    };
    file_row(
        out,
        label,
        color_size(sizes.compressed_size, sizes.bytes_saved),
    );
    file_row(out, "Bytes saved:", format_saved(sizes.bytes_saved));
    file_row(out, "Reduction ratio:", color_ratio(sizes.ratio));
}

pub fn render_file_report(report: &FileReport) -> String {
    let rule = separator(FILE_REPORT_SEPARATOR_WIDTH);
    let mut out = String::new();
    let suffix = dry_run_suffix(report.dry_run, "(dry run — no file written)");
    let _ = writeln!(out, "{rule}");
    let _ = writeln!(out, "{} {}", media_title(&report.details).bold(), suffix);
    let _ = writeln!(out, "{rule}");
    file_row(
        &mut out,
        "Input:",
        report.input_path.display().to_string().cyan(),
    );
    if !report.dry_run {
        file_row(
            &mut out,
            "Output:",
            report.output_path.display().to_string().cyan(),
        );
    }
    write_image_rows(&mut out, &report.details);
    write_size_rows(&mut out, &report.sizes, report.dry_run);
    if report.kept_original {
        file_row(
            &mut out,
            "Note:",
            "output not smaller; original bytes kept".yellow(),
        );
    }
    file_row(
        &mut out,
        "Duration:",
        format!("{}ms", report.duration_ms).white(),
    );
    let _ = writeln!(out, "{rule}");
    out
}

fn write_batch_totals(out: &mut String, report: &BatchReport) {
    batch_row(
        out,
        "Total original:",
        format_size(report.total_original_bytes, DECIMAL).white(),
    );
    let label = if report.dry_run {
        "Est. total:"
    } else {
        "Total compressed:"
    };
    batch_row(
        out,
        label,
        color_size(report.total_compressed_bytes, report.total_bytes_saved),
    );
    batch_row(out, "Total saved:", format_saved(report.total_bytes_saved));
    batch_row(out, "Overall reduction:", color_ratio(report.overall_ratio));
}

fn write_batch_counts(out: &mut String, report: &BatchReport) {
    batch_row(
        out,
        "Input dir:",
        report.input_dir.display().to_string().cyan(),
    );
    if !report.dry_run {
        batch_row(
            out,
            "Output dir:",
            report.output_dir.display().to_string().cyan(),
        );
    }
    batch_row(out, "Total files:", report.total_files.to_string().white());
    batch_row(out, "Succeeded:", report.succeeded.to_string().green());
    if report.skipped > 0 {
        batch_row(out, "Skipped:", report.skipped.to_string().yellow());
    }
    if report.failed > 0 {
        batch_row(out, "Failed:", report.failed.to_string().red());
    }
}

fn write_failures(out: &mut String, report: &BatchReport) {
    let failures: Vec<_> = report
        .files
        .iter()
        .filter_map(|file| match &file.outcome {
            FileOutcome::Error { error } => Some((&file.path, error)),
            _ => None,
        })
        .collect();
    if failures.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{}", "Failed files:".red().bold());
    for (path, error) in failures {
        let _ = writeln!(
            out,
            "  {} {}",
            path.display().to_string().red(),
            error.dimmed()
        );
    }
}

pub fn render_batch_report(report: &BatchReport) -> String {
    let rule = separator(BATCH_REPORT_SEPARATOR_WIDTH);
    let mut out = String::new();
    let suffix = dry_run_suffix(report.dry_run, "(dry run)");
    let _ = writeln!(out, "{rule}");
    let _ = writeln!(out, "{} {}", "Batch Compression Result".bold(), suffix);
    let _ = writeln!(out, "{rule}");
    write_batch_counts(&mut out, report);
    write_batch_totals(&mut out, report);
    batch_row(
        &mut out,
        "Duration:",
        format!("{}ms", report.duration_ms).white(),
    );
    let _ = writeln!(out, "{rule}");
    write_failures(&mut out, report);
    out
}

pub fn render_info_report(report: &InfoReport) -> String {
    let rule = separator(FILE_REPORT_SEPARATOR_WIDTH);
    let mut out = String::new();
    let _ = writeln!(out, "{rule}");
    let _ = writeln!(out, "{}", "PDF Information".bold());
    let _ = writeln!(out, "{rule}");
    info_row(&mut out, "File:", report.path.cyan());
    info_row(
        &mut out,
        "Size:",
        format_size(report.file_size_bytes, DECIMAL).white(),
    );
    info_row(&mut out, "Pages:", report.page_count.to_string().white());
    info_row(&mut out, "PDF version:", report.pdf_version.white());
    let _ = writeln!(out, "{rule}");
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::media::ImageFormat;
    use crate::domain::report::{BatchContext, FileResult};

    fn plain() {
        colored::control::set_override(false);
    }

    fn pdf_report(dry_run: bool, saved: bool) -> FileReport {
        FileReport {
            input_path: PathBuf::from("in.pdf"),
            output_path: PathBuf::from("out.pdf"),
            sizes: if saved {
                SizeStats::new(2_000_000, 1_000_000)
            } else {
                SizeStats::new(1_000_000, 1_500_000)
            },
            kept_original: false,
            duration_ms: 42,
            dry_run,
            details: MediaDetails::Pdf { passes: Vec::new() },
        }
    }

    fn image_report(output_format: ImageFormat, output: Dimensions) -> FileReport {
        FileReport {
            input_path: PathBuf::from("in.png"),
            output_path: PathBuf::from("out.jpg"),
            sizes: SizeStats::new(1000, 500),
            kept_original: false,
            duration_ms: 5,
            dry_run: false,
            details: MediaDetails::Image {
                input_format: ImageFormat::Png,
                output_format,
                original_dimensions: Dimensions {
                    width: 640,
                    height: 480,
                },
                output_dimensions: output,
            },
        }
    }

    #[test]
    fn render_file_report_for_pdf_lists_sizes_and_output() {
        plain();
        let text = render_file_report(&pdf_report(false, true));
        assert!(text.contains("PDF Compressor Result"));
        assert!(text.contains("Input:               in.pdf"));
        assert!(text.contains("Output:              out.pdf"));
        assert!(text.contains("Original size:       2 MB"));
        assert!(text.contains("Compressed size:     1 MB"));
        assert!(text.contains("Bytes saved:         -1 MB"));
        assert!(text.contains("Reduction ratio:     50.0%"));
        assert!(text.contains("Duration:            42ms"));
        assert!(!text.contains("dry run"));
    }

    #[test]
    fn render_file_report_dry_run_hides_output_and_labels_estimate() {
        plain();
        let text = render_file_report(&pdf_report(true, true));
        assert!(text.contains("(dry run — no file written)"));
        assert!(text.contains("Est. size:"));
        assert!(!text.contains("Output:"));
    }

    #[test]
    fn render_file_report_marks_growth_with_plus_sign() {
        plain();
        let text = render_file_report(&pdf_report(false, false));
        assert!(text.contains("Bytes saved:         +500 kB"));
        assert!(text.contains("-50.0%"));
    }

    #[test]
    fn render_file_report_for_image_shows_format_change_and_resize() {
        plain();
        let text = render_file_report(&image_report(
            ImageFormat::Jpeg,
            Dimensions {
                width: 320,
                height: 240,
            },
        ));
        assert!(text.contains("Image Compressor Result"));
        assert!(text.contains("PNG → JPEG"));
        assert!(text.contains("640×480 → 320×240"));
    }

    #[test]
    fn render_file_report_for_image_without_changes_shows_plain_values() {
        plain();
        let text = render_file_report(&image_report(
            ImageFormat::Png,
            Dimensions {
                width: 640,
                height: 480,
            },
        ));
        assert!(text.contains("Format:              PNG\n"));
        assert!(text.contains("Dimensions:          640×480\n"));
    }

    #[test]
    fn render_file_report_notes_when_original_bytes_were_kept() {
        plain();
        let mut report = pdf_report(false, true);
        report.kept_original = true;
        assert!(render_file_report(&report).contains("output not smaller; original bytes kept"));
    }

    fn batch_report(dry_run: bool) -> BatchReport {
        let files = vec![
            FileResult {
                path: PathBuf::from("a.pdf"),
                outcome: FileOutcome::Ok {
                    sizes: SizeStats::new(2000, 1000),
                    file_type: "PDF".into(),
                },
            },
            FileResult {
                path: PathBuf::from("b.pdf"),
                outcome: FileOutcome::Skipped {
                    reason: "output already exists".into(),
                },
            },
            FileResult {
                path: PathBuf::from("c.jpg"),
                outcome: FileOutcome::Error {
                    error: "boom".into(),
                },
            },
        ];
        BatchReport::from_results(
            BatchContext {
                input_dir: PathBuf::from("in"),
                output_dir: PathBuf::from("out"),
                duration_ms: 9,
                dry_run,
            },
            files,
        )
    }

    #[test]
    fn render_batch_report_lists_counts_totals_and_failures() {
        plain();
        let text = render_batch_report(&batch_report(false));
        assert!(text.contains("Batch Compression Result"));
        assert!(text.contains("Total files:           3"));
        assert!(text.contains("Succeeded:             1"));
        assert!(text.contains("Skipped:               1"));
        assert!(text.contains("Failed:                1"));
        assert!(text.contains("Output dir:            out"));
        assert!(text.contains("Total compressed:      1 kB"));
        assert!(text.contains("Overall reduction:     50.0%"));
        assert!(text.contains("Failed files:"));
        assert!(text.contains("c.jpg boom"));
    }

    #[test]
    fn render_batch_report_dry_run_uses_estimate_labels() {
        plain();
        let text = render_batch_report(&batch_report(true));
        assert!(text.contains("(dry run)"));
        assert!(text.contains("Est. total:"));
        assert!(!text.contains("Output dir:"));
    }

    #[test]
    fn render_batch_report_omits_skipped_and_failed_rows_when_zero() {
        plain();
        let report = BatchReport::from_results(
            BatchContext {
                input_dir: PathBuf::from("in"),
                output_dir: PathBuf::from("out"),
                duration_ms: 1,
                dry_run: false,
            },
            Vec::new(),
        );
        let text = render_batch_report(&report);
        assert!(!text.contains("Skipped:"));
        assert!(!text.contains("Failed"));
        assert!(text.contains("Total files:           0"));
    }

    #[test]
    fn render_info_report_lists_file_facts() {
        plain();
        let text = render_info_report(&InfoReport {
            path: "doc.pdf".into(),
            file_size_bytes: 2048,
            page_count: 7,
            pdf_version: "1.7".into(),
        });
        assert!(text.contains("PDF Information"));
        assert!(text.contains(&format!("  {:<18} doc.pdf", "File:")));
        assert!(text.contains(&format!("  {:<18} 2.05 kB", "Size:")));
        assert!(text.contains(&format!("  {:<18} 7", "Pages:")));
        assert!(text.contains(&format!("  {:<18} 1.7", "PDF version:")));
    }

    #[test]
    fn color_ratio_picks_distinct_text_for_each_band() {
        plain();
        assert_eq!(color_ratio(45.0).to_string(), "45.0%");
        assert_eq!(color_ratio(15.0).to_string(), "15.0%");
        assert_eq!(color_ratio(5.0).to_string(), "5.0%");
        assert_eq!(color_ratio(-5.0).to_string(), "-5.0%");
    }
}
