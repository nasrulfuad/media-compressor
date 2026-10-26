use std::path::{Path, PathBuf};

/// Derive a default output path from an input PDF path.
///
/// Given `foo.pdf`, returns `foo_compressed.pdf`.
/// Given `foo` (no extension), returns `foo_compressed`.
/// Given `path/to/foo.pdf`, returns `path/to/foo_compressed.pdf`.
pub fn default_output_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let new_name = format!("{}_compressed", stem);

    let mut output = input.with_file_name(new_name);

    if let Some(ext) = input.extension() {
        output.set_extension(ext);
    }

    output
}

/// Derive a default output path for an image file, using the specified extension.
///
/// Given `photo.jpg` with ext `"png"`, returns `photo_compressed.png`.
/// Given `path/to/photo.jpg` with ext `"webp"`, returns `path/to/photo_compressed.webp`.
pub fn default_image_output_path(input: &Path, ext: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let new_name = format!("{}_compressed", stem);
    input.with_file_name(new_name).with_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_path_appends_compressed() {
        let input = Path::new("document.pdf");
        let output = default_output_path(input);
        assert_eq!(output, PathBuf::from("document_compressed.pdf"));
    }

    #[test]
    fn default_output_path_preserves_directory() {
        let input = Path::new("docs/reports/annual.pdf");
        let output = default_output_path(input);
        assert_eq!(output, PathBuf::from("docs/reports/annual_compressed.pdf"));
    }

    #[test]
    fn default_output_path_works_without_extension() {
        let input = Path::new("myfile");
        let output = default_output_path(input);
        assert_eq!(output, PathBuf::from("myfile_compressed"));
    }

    #[test]
    fn default_output_path_preserves_non_pdf_extension() {
        let input = Path::new("archive.gz");
        let output = default_output_path(input);
        // Rust splits only on the last '.': stem = "archive", extension = "gz"
        assert_eq!(output, PathBuf::from("archive_compressed.gz"));
    }

    #[test]
    fn default_image_output_path_changes_extension() {
        let input = Path::new("photo.jpg");
        let output = default_image_output_path(input, "png");
        assert_eq!(output, PathBuf::from("photo_compressed.png"));
    }

    #[test]
    fn default_image_output_path_preserves_directory() {
        let input = Path::new("images/photo.jpg");
        let output = default_image_output_path(input, "webp");
        assert_eq!(output, PathBuf::from("images/photo_compressed.webp"));
    }

    #[test]
    fn default_output_path_handles_absolute_path() {
        let input = Path::new("/home/user/documents/report.pdf");
        let output = default_output_path(input);
        assert_eq!(
            output,
            PathBuf::from("/home/user/documents/report_compressed.pdf")
        );
    }
}
