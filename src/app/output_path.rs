use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

const COMPRESSED_SUFFIX: &str = "_compressed";

pub fn default_output_path(input: &Path, extension: &str) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let file_name = if extension.is_empty() {
        format!("{stem}{COMPRESSED_SUFFIX}")
    } else {
        format!("{stem}{COMPRESSED_SUFFIX}.{extension}")
    };
    input.with_file_name(file_name)
}

pub fn default_batch_output_dir(input_dir: &Path) -> PathBuf {
    let mut name = input_dir.file_name().unwrap_or_default().to_os_string();
    name.push(COMPRESSED_SUFFIX);
    input_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(name)
}

pub fn batch_output_path(
    file: &Path,
    input_dir: &Path,
    output_dir: &Path,
    extension: Option<&str>,
) -> AppResult<PathBuf> {
    let relative = file
        .strip_prefix(input_dir)
        .map_err(|_| AppError::PathOutsideInput(file.to_path_buf()))?;
    let mut output = output_dir.join(relative);
    if let Some(extension) = extension {
        output.set_extension(extension);
    }
    if output == file {
        return Err(AppError::OutputEqualsInput(file.to_path_buf()));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_output_path_appends_compressed() {
        assert_eq!(
            default_output_path(Path::new("document.pdf"), "pdf"),
            PathBuf::from("document_compressed.pdf")
        );
    }

    #[test]
    fn default_output_path_preserves_directory() {
        assert_eq!(
            default_output_path(Path::new("docs/reports/annual.pdf"), "pdf"),
            PathBuf::from("docs/reports/annual_compressed.pdf")
        );
    }

    #[test]
    fn default_output_path_uses_given_extension_for_files_without_one() {
        assert_eq!(
            default_output_path(Path::new("myfile"), "pdf"),
            PathBuf::from("myfile_compressed.pdf")
        );
    }

    #[test]
    fn default_output_path_changes_extension_for_conversions() {
        assert_eq!(
            default_output_path(Path::new("images/photo.jpg"), "png"),
            PathBuf::from("images/photo_compressed.png")
        );
    }

    #[test]
    fn default_output_path_handles_absolute_path_and_dotted_stem() {
        assert_eq!(
            default_output_path(Path::new("/home/user/my.report.final.pdf"), "pdf"),
            PathBuf::from("/home/user/my.report.final_compressed.pdf")
        );
    }

    #[test]
    fn default_output_path_with_empty_extension_has_no_extension() {
        assert_eq!(
            default_output_path(Path::new("raw"), ""),
            PathBuf::from("raw_compressed")
        );
    }

    #[test]
    fn default_batch_output_dir_is_sibling_with_suffix() {
        assert_eq!(
            default_batch_output_dir(Path::new("/data/pdfs")),
            PathBuf::from("/data/pdfs_compressed")
        );
        assert_eq!(
            default_batch_output_dir(Path::new("pdfs")),
            PathBuf::from("pdfs_compressed")
        );
    }

    #[test]
    fn batch_output_path_preserves_relative_structure() {
        let output = batch_output_path(
            Path::new("in/a/b.pdf"),
            Path::new("in"),
            Path::new("out"),
            None,
        )
        .unwrap();
        assert_eq!(output, PathBuf::from("out/a/b.pdf"));
    }

    #[test]
    fn batch_output_path_rejects_file_outside_input_dir() {
        let error = batch_output_path(
            Path::new("elsewhere/b.pdf"),
            Path::new("in"),
            Path::new("out"),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, AppError::PathOutsideInput(_)));
    }

    #[test]
    fn batch_output_path_rejects_output_equal_to_input() {
        let error = batch_output_path(
            Path::new("in/b.pdf"),
            Path::new("in"),
            Path::new("in"),
            None,
        )
        .unwrap_err();
        assert!(matches!(error, AppError::OutputEqualsInput(_)));
    }

    #[test]
    fn batch_output_path_replaces_extension_when_given() {
        let output = batch_output_path(
            Path::new("in/photo.png"),
            Path::new("in"),
            Path::new("out"),
            Some("jpg"),
        )
        .unwrap();
        assert_eq!(output, PathBuf::from("out/photo.jpg"));
    }
}
