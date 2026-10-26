mod common;

use std::io::Write;

use common::{cli, minimal_pdf_bytes, write_temp};
use predicates::prelude::*;
use tempfile::{NamedTempFile, TempDir};

#[test]
fn compress_existing_output_without_overwrite_exits_with_code_2() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(
        &dir,
        "doc.pdf",
        &minimal_pdf_bytes(1, &b"BT   ET   ".repeat(40)),
    );
    write_temp(&dir, "doc_compressed.pdf", b"already here");

    cli()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn batch_on_file_instead_of_directory_exits_with_code_2() {
    let dir = TempDir::new().unwrap();
    let file = write_temp(&dir, "doc.pdf", b"%PDF-1.4");

    cli()
        .args(["batch", file.to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn invalid_glob_exits_with_code_2() {
    let dir = TempDir::new().unwrap();

    cli()
        .args(["batch", dir.path().to_str().unwrap(), "-p", "["])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid glob pattern"));
}

#[test]
fn compress_missing_input_exits_with_code_1_and_names_the_file() {
    cli()
        .args(["compress", "/nonexistent/dir/missing_input.pdf"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("missing_input.pdf"));
}

#[test]
fn compress_rejects_input_larger_than_max_input_size() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "doc.pdf", &minimal_pdf_bytes(1, b"BT ET"));

    cli()
        .args([
            "compress",
            input.to_str().unwrap(),
            "--max-input-size",
            "10",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("exceeds the limit of 10 bytes"));
}

#[test]
fn compress_unknown_file_type_reports_magic_bytes() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "data.bin", &[0xDE, 0xAD, 0xBE, 0xEF]);

    cli()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("de ad be ef"));
}

#[test]
fn compress_gif_input_is_rejected_as_unsupported() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "anim.gif", b"GIF89a\x01\x00\x01\x00");

    cli()
        .args(["compress", input.to_str().unwrap()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("GIF input is not supported"));
}

#[test]
fn strip_metadata_flag_is_rejected() {
    cli()
        .args(["compress", "a.pdf", "--strip-metadata"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--strip-metadata"));
}

#[test]
fn webp_format_is_rejected() {
    cli()
        .args(["compress", "a.jpg", "--format", "webp"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("webp"));
}

#[test]
fn info_on_image_reports_pdf_only() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "photo.png", &common::png_bytes(8, 8));

    cli()
        .args(["info", input.to_str().unwrap()])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("info supports PDF files only"));
}

#[test]
fn info_reports_page_count_as_json() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "doc.pdf", &minimal_pdf_bytes(3, b"BT ET"));

    let output = cli()
        .args(["info", input.to_str().unwrap(), "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["page_count"], 3);
    assert_eq!(report["pdf_version"], "1.5");
}

#[test]
fn help_flag_exits_successfully() {
    cli()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("media-compressor"));
}

#[test]
fn version_flag_exits_successfully() {
    cli()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("media-compressor"));
}

#[test]
fn compress_subcommand_help_exits_successfully() {
    cli()
        .args(["compress", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--max-input-size"));
}

#[test]
fn info_subcommand_help_exits_successfully() {
    cli()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("metadata"));
}

#[test]
fn compress_nonexistent_file_returns_error() {
    cli()
        .args(["compress", "/nonexistent/path/does_not_exist.pdf"])
        .assert()
        .failure();
}

#[test]
fn info_nonexistent_file_returns_error() {
    cli()
        .args(["info", "/nonexistent/path/does_not_exist.pdf"])
        .assert()
        .failure();
}

#[test]
fn compress_non_pdf_file_returns_error() {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(b"This is definitely not a PDF").unwrap();

    cli()
        .args(["compress", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn info_non_pdf_file_returns_error() {
    let mut tmp = NamedTempFile::new().unwrap();
    tmp.write_all(b"Not a PDF file").unwrap();

    cli()
        .args(["info", tmp.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn compress_without_input_fails_with_usage() {
    cli()
        .arg("compress")
        .assert()
        .failure()
        .stderr(predicate::str::contains("INPUT").or(predicate::str::contains("required")));
}

#[test]
fn no_subcommand_prints_help() {
    cli()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage").or(predicate::str::contains("USAGE")));
}

#[test]
fn compress_invalid_level_returns_error() {
    cli()
        .args(["compress", "a.pdf", "--level", "ultramax"])
        .assert()
        .failure();
}

#[test]
fn compress_image_quality_zero_is_rejected() {
    cli()
        .args(["compress", "a.pdf", "--image-quality", "0"])
        .assert()
        .failure();
}

#[test]
fn batch_subcommand_help_exits_successfully() {
    cli()
        .args(["batch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Compress all supported files"));
}

#[test]
fn batch_nonexistent_dir_returns_error() {
    cli()
        .args(["batch", "/nonexistent/path/does_not_exist"])
        .assert()
        .failure();
}

#[test]
fn batch_without_input_fails_with_usage() {
    cli()
        .arg("batch")
        .assert()
        .failure()
        .stderr(predicate::str::contains("INPUT_DIR").or(predicate::str::contains("required")));
}

#[test]
fn batch_empty_dir_succeeds_with_zero_files() {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .args(["batch", dir.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Total files:").and(predicate::str::contains("0")));
}

#[test]
fn verbose_and_quiet_conflict() {
    cli()
        .args(["--verbose", "--quiet", "compress", "a.pdf"])
        .assert()
        .failure();
}
