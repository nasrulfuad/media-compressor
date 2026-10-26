mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::{BufWriter, Write};
use tempfile::NamedTempFile;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Return a configured `Command` pointing at the `media-compressor` binary.
fn cmd() -> Command {
    Command::cargo_bin("media-compressor").expect("media-compressor binary not found")
}

fn create_test_jpeg(width: u32, height: u32) -> NamedTempFile {
    let img = image::DynamicImage::new_rgb8(width, height);
    let mut f = NamedTempFile::with_suffix(".jpg").unwrap();
    img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Jpeg)
        .unwrap();
    f
}

fn create_test_png(width: u32, height: u32) -> NamedTempFile {
    let img = image::DynamicImage::new_rgb8(width, height);
    let mut f = NamedTempFile::with_suffix(".png").unwrap();
    img.write_to(&mut BufWriter::new(&mut f), image::ImageFormat::Png)
        .unwrap();
    f
}

// ── --help / --version ────────────────────────────────────────────────────

#[test]
fn help_flag_exits_successfully() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("media-compressor"));
}

#[test]
fn version_flag_exits_successfully() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("media-compressor"));
}

#[test]
fn compress_subcommand_help_exits_successfully() {
    cmd()
        .args(["compress", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compress any supported file"));
}

#[test]
fn info_subcommand_help_exits_successfully() {
    cmd()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("metadata"));
}

// ── Error cases ───────────────────────────────────────────────────────────

#[test]
fn compress_nonexistent_file_returns_error() {
    cmd()
        .args(["compress", "/nonexistent/path/does_not_exist.pdf"])
        .assert()
        .failure();
}

#[test]
fn info_nonexistent_file_returns_error() {
    cmd()
        .args(["info", "/nonexistent/path/does_not_exist.pdf"])
        .assert()
        .failure();
}

#[test]
fn compress_non_pdf_file_returns_error() {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(b"This is definitely not a PDF").unwrap();

    cmd()
        .args(["compress", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn info_non_pdf_file_returns_error() {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(b"Not a PDF file").unwrap();

    cmd()
        .args(["info", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

// ── Missing required argument ─────────────────────────────────────────────

#[test]
fn compress_without_input_fails_with_usage() {
    cmd()
        .arg("compress")
        .assert()
        .failure()
        .stderr(predicate::str::contains("INPUT").or(predicate::str::contains("required")));
}

#[test]
fn no_subcommand_prints_help() {
    cmd()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage").or(predicate::str::contains("USAGE")));
}

// ── Invalid argument values ───────────────────────────────────────────────

#[test]
fn compress_invalid_level_returns_error() {
    cmd()
        .args(["compress", "a.pdf", "--level", "ultramax"])
        .assert()
        .failure();
}

#[test]
fn compress_image_quality_zero_is_rejected() {
    cmd()
        .args(["compress", "a.pdf", "--image-quality", "0"])
        .assert()
        .failure();
}

// ── Batch subcommand ──────────────────────────────────────────────────────

#[test]
fn batch_subcommand_help_exits_successfully() {
    cmd()
        .args(["batch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compress all supported files"));
}

#[test]
fn batch_nonexistent_dir_returns_error() {
    cmd()
        .args(["batch", "/nonexistent/path/does_not_exist"])
        .assert()
        .failure();
}

#[test]
fn batch_without_input_fails_with_usage() {
    cmd()
        .arg("batch")
        .assert()
        .failure()
        .stderr(predicate::str::contains("INPUT_DIR").or(predicate::str::contains("required")));
}

#[test]
fn batch_empty_dir_succeeds_with_zero_files() {
    let dir = tempfile::tempdir().unwrap();
    cmd()
        .args(["batch", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total files:").and(predicate::str::contains("0")));
}

// ── Flag conflicts ────────────────────────────────────────────────────────

#[test]
fn verbose_and_quiet_conflict() {
    cmd()
        .args(["--verbose", "--quiet", "compress", "a.pdf"])
        .assert()
        .failure();
}

// ── Image compression ─────────────────────────────────────────────────────

#[test]
fn compress_jpeg_file_succeeds() {
    let input = create_test_jpeg(64, 64);
    let output = NamedTempFile::with_suffix(".jpg").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--overwrite",
        ])
        .assert()
        .success();
}

#[test]
fn compress_jpeg_with_dry_run_succeeds() {
    let input = create_test_jpeg(64, 64);
    cmd()
        .args(["compress", input.path().to_str().unwrap(), "--dry-run"])
        .assert()
        .success();
}

#[test]
fn compress_jpeg_with_max_dimension_succeeds() {
    let input = create_test_jpeg(200, 200);
    let output = NamedTempFile::with_suffix(".jpg").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--max-dimension",
            "100",
            "--overwrite",
        ])
        .assert()
        .success();
}

#[test]
fn compress_png_file_succeeds() {
    let input = create_test_png(32, 32);
    let output = NamedTempFile::with_suffix(".png").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--overwrite",
        ])
        .assert()
        .success();
}

#[test]
fn compress_png_to_jpeg_format_conversion() {
    let input = create_test_png(32, 32);
    let output = NamedTempFile::with_suffix(".jpg").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--format",
            "jpeg",
            "--overwrite",
        ])
        .assert()
        .success();
    let bytes = std::fs::read(output.path()).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "Output must be a valid JPEG");
}

#[test]
fn compress_jpeg_with_json_output() {
    let input = create_test_jpeg(32, 32);
    let output = NamedTempFile::with_suffix(".jpg").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--json",
            "--overwrite",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("compressed_size"));
}

#[test]
fn compress_webp_output_returns_error() {
    let input = create_test_jpeg(16, 16);
    let output = NamedTempFile::with_suffix(".webp").unwrap();
    cmd()
        .args([
            "compress",
            input.path().to_str().unwrap(),
            "-o",
            output.path().to_str().unwrap(),
            "--format",
            "webp",
            "--overwrite",
        ])
        .assert()
        .failure();
}
