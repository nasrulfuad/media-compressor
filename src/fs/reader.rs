use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::Path;

use crate::error::{AppError, AppResult};

fn open_input(path: &Path) -> AppResult<File> {
    File::open(path).map_err(|error| match error.kind() {
        ErrorKind::NotFound => AppError::InputNotFound(path.to_path_buf()),
        _ => AppError::Io(error),
    })
}

fn too_large(path: &Path, size: u64, limit: u64) -> AppError {
    AppError::InputTooLarge {
        path: path.to_path_buf(),
        size,
        limit,
    }
}

pub fn read_bounded(path: &Path, max_bytes: u64) -> AppResult<Vec<u8>> {
    let file = open_input(path)?;
    let declared_size = file.metadata()?.len();
    if declared_size > max_bytes {
        return Err(too_large(path, declared_size, max_bytes));
    }
    let mut bytes = Vec::with_capacity(declared_size as usize);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large(path, bytes.len() as u64, max_bytes));
    }
    Ok(bytes)
}

pub fn read_leading_bytes(path: &Path, count: usize) -> AppResult<Vec<u8>> {
    let file = open_input(path)?;
    let mut bytes = Vec::with_capacity(count);
    file.take(count as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn temp_file_with(bytes: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    #[test]
    fn read_bounded_returns_input_not_found() {
        let error = read_bounded(Path::new("/nonexistent/path/file.pdf"), 100).unwrap_err();
        assert!(matches!(error, AppError::InputNotFound(path) if path.ends_with("file.pdf")));
    }

    #[test]
    fn read_bounded_rejects_file_above_limit() {
        let file = temp_file_with(&[1u8; 11]);
        let error = read_bounded(file.path(), 10).unwrap_err();
        assert!(matches!(
            error,
            AppError::InputTooLarge {
                size: 11,
                limit: 10,
                ..
            }
        ));
    }

    #[test]
    fn read_bounded_reads_file_at_limit() {
        let file = temp_file_with(&[2u8; 10]);
        assert_eq!(read_bounded(file.path(), 10).unwrap(), vec![2u8; 10]);
    }

    #[test]
    fn read_bounded_reads_empty_file() {
        let file = temp_file_with(&[]);
        assert!(read_bounded(file.path(), 10).unwrap().is_empty());
    }

    #[test]
    fn read_bounded_on_directory_returns_io_error() {
        let directory = tempfile::TempDir::new().unwrap();
        let error = read_bounded(directory.path(), 1_000_000).unwrap_err();
        assert!(matches!(error, AppError::Io(_)));
    }

    #[test]
    fn read_leading_bytes_returns_prefix_of_file() {
        let file = temp_file_with(b"0123456789");
        assert_eq!(read_leading_bytes(file.path(), 4).unwrap(), b"0123");
    }

    #[test]
    fn read_leading_bytes_returns_whole_short_file() {
        let file = temp_file_with(b"ab");
        assert_eq!(read_leading_bytes(file.path(), 16).unwrap(), b"ab");
    }

    #[test]
    fn read_leading_bytes_reports_missing_file() {
        let error = read_leading_bytes(Path::new("/nonexistent/x"), 4).unwrap_err();
        assert!(matches!(error, AppError::InputNotFound(_)));
    }
}
