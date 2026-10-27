mod common;

use std::io::Write;

use common::{cli, minimal_pdf_bytes, write_temp};
use predicates::prelude::*;
use tempfile::{NamedTempFile, TempDir};

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
fn info_subcommand_help_exits_successfully() {
    cli()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("metadata"));
}

#[test]
fn info_nonexistent_file_returns_error() {
    cli()
        .args(["info", "/nonexistent/path/does_not_exist.pdf"])
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
