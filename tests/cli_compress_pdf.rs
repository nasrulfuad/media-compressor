mod common;

use std::collections::BTreeSet;

use common::{cli, minimal_pdf_bytes, pdf_with_flate_rgb_image_bytes, png_bytes, write_temp};
use lopdf::Document;
use tempfile::TempDir;

fn json_keys(stdout: &[u8]) -> BTreeSet<String> {
    let value: serde_json::Value = serde_json::from_slice(stdout).expect("stdout must be JSON");
    value
        .as_object()
        .expect("JSON root must be an object")
        .keys()
        .cloned()
        .collect()
}

fn key_set(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| key.to_string()).collect()
}

#[test]
fn compress_pdf_json_report_has_stable_keys() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "doc.pdf", &minimal_pdf_bytes(1, b"BT ET"));

    let output = cli()
        .args(["compress", input.to_str().unwrap(), "--json", "--dry-run"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        json_keys(&output.stdout),
        key_set(&[
            "input_path",
            "output_path",
            "original_size",
            "compressed_size",
            "bytes_saved",
            "ratio",
            "duration_ms",
            "dry_run",
            "kept_original",
            "passes",
        ])
    );
}

#[test]
fn compress_image_json_report_has_stable_keys() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "photo.png", &png_bytes(32, 32));

    let output = cli()
        .args(["compress", input.to_str().unwrap(), "--json", "--dry-run"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        json_keys(&output.stdout),
        key_set(&[
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
            "kept_original",
        ])
    );
}

#[test]
fn batch_json_report_has_stable_keys() {
    let dir = TempDir::new().unwrap();
    let input_dir = dir.path().join("in");
    std::fs::create_dir(&input_dir).unwrap();
    std::fs::write(input_dir.join("a.pdf"), minimal_pdf_bytes(1, b"BT ET")).unwrap();

    let output = cli()
        .args(["batch", input_dir.to_str().unwrap(), "--json", "--dry-run"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let keys: BTreeSet<String> = report.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        key_set(&[
            "input_dir",
            "output_dir",
            "total_files",
            "succeeded",
            "skipped",
            "failed",
            "total_original_bytes",
            "total_compressed_bytes",
            "total_bytes_saved",
            "overall_ratio",
            "duration_ms",
            "dry_run",
            "files",
        ])
    );
    let file_keys: BTreeSet<String> = report["files"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        file_keys,
        key_set(&[
            "path",
            "status",
            "original_size",
            "compressed_size",
            "bytes_saved",
            "ratio",
            "file_type",
        ])
    );
}

#[test]
fn compress_pdf_preserves_page_count_at_every_level() {
    let dir = TempDir::new().unwrap();
    let input = write_temp(&dir, "doc.pdf", &pdf_with_flate_rgb_image_bytes(64, 64));

    for level in ["low", "medium", "high", "extreme"] {
        let output = dir.path().join(format!("out_{level}.pdf"));
        cli()
            .args([
                "compress",
                input.to_str().unwrap(),
                "-o",
                output.to_str().unwrap(),
                "--level",
                level,
            ])
            .assert()
            .success();

        let reloaded = Document::load(&output).unwrap();
        assert_eq!(reloaded.get_pages().len(), 1, "level {level}");
    }
}
