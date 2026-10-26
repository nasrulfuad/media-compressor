use std::path::Path;

use crate::detection::detect_media_type;
use crate::domain::media::MediaType;
use crate::domain::report::InfoReport;
use crate::error::{AppError, AppResult};
use crate::fs::reader::read_bounded;
use crate::pdf::engine::parse_pdf;

pub fn read_pdf_info(path: &Path, max_input_bytes: u64) -> AppResult<InfoReport> {
    let bytes = read_bounded(path, max_input_bytes)?;
    if !matches!(detect_media_type(&bytes), MediaType::Pdf) {
        return Err(AppError::UnsupportedFormat(
            "info supports PDF files only".to_string(),
        ));
    }
    let document = parse_pdf(&bytes)?;
    Ok(InfoReport {
        path: path.display().to_string(),
        file_size_bytes: bytes.len() as u64,
        page_count: document.get_pages().len(),
        pdf_version: document.version.clone(),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_support::PdfFixture;

    #[test]
    fn read_pdf_info_reports_pages_size_and_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("doc.pdf");
        let bytes = PdfFixture::new()
            .page_with_content(b"q Q")
            .page_with_content(b"BT ET")
            .into_bytes();
        std::fs::write(&path, &bytes).unwrap();
        let report = read_pdf_info(&path, u64::MAX).unwrap();
        assert_eq!(report.page_count, 2);
        assert_eq!(report.file_size_bytes, bytes.len() as u64);
        assert_eq!(report.pdf_version, "1.5");
        assert_eq!(report.path, path.display().to_string());
    }

    #[test]
    fn read_pdf_info_rejects_non_pdf_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("photo.png");
        std::fs::write(&path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        let error = read_pdf_info(&path, u64::MAX).unwrap_err();
        assert!(
            matches!(error, AppError::UnsupportedFormat(message) if message == "info supports PDF files only")
        );
    }

    #[test]
    fn read_pdf_info_reports_missing_file() {
        let error = read_pdf_info(Path::new("/nonexistent/x.pdf"), u64::MAX).unwrap_err();
        assert!(matches!(error, AppError::InputNotFound(_)));
    }

    #[test]
    fn read_pdf_info_rejects_corrupt_pdf() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.pdf");
        std::fs::write(&path, b"%PDF-1.7\ngarbage").unwrap();
        assert!(read_pdf_info(&path, u64::MAX).is_err());
    }
}
