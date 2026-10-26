mod common;

use common::{cli, jpeg_bytes, minimal_pdf_bytes, png_bytes};
use tempfile::TempDir;

fn redundant_pdf() -> Vec<u8> {
    minimal_pdf_bytes(1, &b"BT   (Hello)   Tj   ET   ".repeat(60))
}

fn run_batch_json(arguments: &[&str]) -> (std::process::ExitStatus, serde_json::Value) {
    let output = cli()
        .arg("batch")
        .args(arguments)
        .arg("--json")
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap_or(serde_json::Value::Null);
    (output.status, report)
}

#[test]
fn batch_continues_after_corrupt_image_with_continue_on_error() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("a.pdf"), redundant_pdf()).unwrap();
    std::fs::write(input.join("broken.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0]).unwrap();
    let output_dir = dir.path().join("out");

    let (status, report) = run_batch_json(&[
        input.to_str().unwrap(),
        "-o",
        output_dir.to_str().unwrap(),
        "--continue-on-error",
        "--overwrite",
    ]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(report["succeeded"], 1);
    assert_eq!(report["failed"], 1);
    assert!(output_dir.join("a.pdf").exists());
    let statuses: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["status"].as_str().unwrap())
        .collect();
    assert_eq!(statuses, vec!["ok", "error"]);
}

#[test]
fn batch_without_continue_on_error_skips_files_after_first_failure() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(
        input.join("a_broken.jpg"),
        [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0],
    )
    .unwrap();
    std::fs::write(input.join("b.pdf"), redundant_pdf()).unwrap();
    let output_dir = dir.path().join("out");

    let (status, report) = run_batch_json(&[
        input.to_str().unwrap(),
        "-o",
        output_dir.to_str().unwrap(),
        "-w",
        "1",
        "--overwrite",
    ]);

    assert_eq!(status.code(), Some(1));
    assert_eq!(report["failed"], 1);
    assert_eq!(report["skipped"], 1);
    assert!(!output_dir.join("b.pdf").exists());
}

#[test]
fn batch_compresses_mixed_media_and_preserves_structure() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir_all(input.join("sub")).unwrap();
    std::fs::write(input.join("a.pdf"), redundant_pdf()).unwrap();
    std::fs::write(input.join("sub").join("b.png"), png_bytes(48, 48)).unwrap();
    std::fs::write(input.join("sub").join("c.JPG"), jpeg_bytes(48, 48)).unwrap();
    std::fs::write(input.join("ignored.txt"), b"text").unwrap();
    let output_dir = dir.path().join("out");

    let (status, report) = run_batch_json(&[
        input.to_str().unwrap(),
        "-o",
        output_dir.to_str().unwrap(),
        "-r",
        "--overwrite",
    ]);

    assert!(status.success());
    assert_eq!(report["total_files"], 3);
    assert_eq!(report["succeeded"], 3);
    assert!(output_dir.join("a.pdf").exists());
    assert!(output_dir.join("sub").join("b.png").exists());
    assert!(output_dir.join("sub").join("c.JPG").exists());
}

#[test]
fn batch_dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("a.pdf"), redundant_pdf()).unwrap();
    let output_dir = dir.path().join("out");

    let (status, report) = run_batch_json(&[
        input.to_str().unwrap(),
        "-o",
        output_dir.to_str().unwrap(),
        "--dry-run",
    ]);

    assert!(status.success());
    assert_eq!(report["dry_run"], true);
    assert!(!output_dir.exists());
}

#[test]
fn batch_skips_existing_outputs_without_overwrite() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("a.pdf"), redundant_pdf()).unwrap();
    let output_dir = dir.path().join("out");
    std::fs::create_dir(&output_dir).unwrap();
    std::fs::write(output_dir.join("a.pdf"), b"existing").unwrap();

    let (status, report) =
        run_batch_json(&[input.to_str().unwrap(), "-o", output_dir.to_str().unwrap()]);

    assert!(status.success());
    assert_eq!(report["skipped"], 1);
    assert_eq!(
        std::fs::read(output_dir.join("a.pdf")).unwrap(),
        b"existing"
    );
}

#[test]
fn batch_converts_images_with_format_flag() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("b.png"), png_bytes(48, 48)).unwrap();
    let output_dir = dir.path().join("out");

    let (status, _) = run_batch_json(&[
        input.to_str().unwrap(),
        "-o",
        output_dir.to_str().unwrap(),
        "-f",
        "jpeg",
        "--overwrite",
    ]);

    assert!(status.success());
    let converted = std::fs::read(output_dir.join("b.jpg")).unwrap();
    assert_eq!(&converted[..2], &[0xFF, 0xD8]);
}
