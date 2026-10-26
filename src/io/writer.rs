use std::path::Path;
use tracing::debug;

use crate::error::AppResult;

/// Write `doc` to `path` using an atomic temp-file-then-rename strategy.
///
/// The document is first saved to `<path>.tmp`. On success the temp file is
/// renamed to `path`, which is an atomic operation on most operating systems.
/// If any step fails the temp file is removed (best-effort) and the original
/// destination is left untouched.
pub fn save_pdf(doc: &mut lopdf::Document, path: &Path) -> AppResult<()> {
    // Ensure the parent directory exists.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let tmp_path = path.with_extension("pdf.tmp");

    debug!(
        tmp = %tmp_path.display(),
        destination = %path.display(),
        "Writing PDF to temporary file"
    );

    // Save to the temp file; clean up on error.
    let save_result = doc.save(&tmp_path);
    if let Err(e) = save_result {
        // Best-effort cleanup — ignore secondary errors.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    // Atomically rename the temp file to the final destination.
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    debug!(path = %path.display(), "PDF written successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Build the smallest valid-enough lopdf document for save testing.
    fn minimal_doc() -> lopdf::Document {
        lopdf::Document::with_version("1.5")
    }

    #[test]
    fn save_pdf_creates_output_file() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output.pdf");

        let mut doc = minimal_doc();
        save_pdf(&mut doc, &output).unwrap();

        assert!(output.exists(), "Output file should exist after save");
    }

    #[test]
    fn save_pdf_does_not_leave_temp_file_on_success() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("out.pdf");
        let tmp = output.with_extension("pdf.tmp");

        let mut doc = minimal_doc();
        save_pdf(&mut doc, &output).unwrap();

        assert!(!tmp.exists(), "Temp file should be cleaned up after rename");
    }

    #[test]
    fn save_pdf_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("nested").join("deep").join("output.pdf");

        let mut doc = minimal_doc();
        save_pdf(&mut doc, &output).unwrap();

        assert!(
            output.exists(),
            "Output file should exist in nested directory"
        );
    }
}
