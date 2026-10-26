//! Common helpers shared across integration tests.

#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;

/// Create a temporary file containing the given bytes and return it.
///
/// The file is deleted automatically when the returned [`NamedTempFile`] is
/// dropped.
pub fn temp_file_with_content(content: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("Failed to create temp file");
    f.write_all(content)
        .expect("Failed to write temp file content");
    f
}

/// Return the path to the project's `tests/` directory.
pub fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}
