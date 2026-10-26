use std::fs::File;
use std::io::Read;
use std::path::Path;
use tracing::debug;

use crate::error::{AppError, AppResult};

/// Load a PDF document from `path`, performing basic validation first.
///
/// # Errors
///
/// - [`AppError::Io`] if the file cannot be opened or read.
/// - [`AppError::InvalidPdf`] if the file does not begin with the `%PDF-` magic
///   header.
/// - [`AppError::EncryptedPdf`] if lopdf reports that the document is encrypted.
/// - [`AppError::Lopdf`] for any other lopdf parsing failure.
pub fn load_pdf(path: &Path) -> AppResult<lopdf::Document> {
    // ── 1. Existence check ─────────────────────────────────────────────────
    if !path.exists() {
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("File not found: {}", path.display()),
        )));
    }

    // ── 2. Magic-byte validation ───────────────────────────────────────────
    validate_pdf_magic(path)?;

    // ── 3. Parse with lopdf ────────────────────────────────────────────────
    debug!(path = %path.display(), "Parsing PDF with lopdf");

    let doc = lopdf::Document::load(path).map_err(|e| {
        // Map known lopdf errors to more user-friendly variants.
        match &e {
            lopdf::Error::Type => AppError::InvalidPdf(format!(
                "Unexpected object type while parsing '{}'",
                path.display()
            )),
            lopdf::Error::ContentDecode => AppError::InvalidPdf(format!(
                "Content stream could not be decoded in '{}'",
                path.display()
            )),
            other => {
                let msg = other.to_string().to_lowercase();
                if msg.contains("encrypt") {
                    AppError::EncryptedPdf
                } else {
                    AppError::Lopdf(e)
                }
            }
        }
    })?;

    Ok(doc)
}

/// Verify that the first 5 bytes of the file are `%PDF-`.
fn validate_pdf_magic(path: &Path) -> AppResult<()> {
    let mut f = File::open(path)?;
    let mut header = [0u8; 5];

    let bytes_read = f.read(&mut header)?;

    if bytes_read < 5 || &header != b"%PDF-" {
        return Err(AppError::InvalidPdf(format!(
            "'{}' does not appear to be a valid PDF file (missing %PDF- header)",
            path.display()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn load_pdf_returns_error_for_missing_file() {
        let result = load_pdf(Path::new("/nonexistent/path/file.pdf"));
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::Io(_) => {}
            other => panic!("Expected Io error, got {:?}", other),
        }
    }

    #[test]
    fn load_pdf_returns_invalid_for_non_pdf() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"This is not a PDF file at all").unwrap();
        let result = load_pdf(tmp.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::InvalidPdf(_) => {}
            other => panic!("Expected InvalidPdf error, got {:?}", other),
        }
    }

    #[test]
    fn load_pdf_returns_invalid_for_truncated_magic() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"%PDF").unwrap(); // one byte short
        let result = load_pdf(tmp.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::InvalidPdf(_) => {}
            other => panic!("Expected InvalidPdf error, got {:?}", other),
        }
    }

    #[test]
    fn validate_pdf_magic_passes_for_valid_header() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all(b"%PDF-1.7\n%%EOF").unwrap();
        // Magic check passes; lopdf parsing will fail because the body is
        // not a real PDF, but that is tested separately.
        let result = validate_pdf_magic(tmp.path());
        assert!(result.is_ok());
    }
}
