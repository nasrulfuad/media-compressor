use assert_cmd::Command;
use predicates::prelude::*;
use std::io::BufWriter;
use tempfile::NamedTempFile;

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
