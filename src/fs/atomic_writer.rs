use std::io::Write;
use std::path::Path;

use crate::error::{AppError, AppResult};

fn parent_directory(destination: &Path) -> &Path {
    match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

pub fn write_atomically(destination: &Path, bytes: &[u8]) -> AppResult<()> {
    let directory = parent_directory(destination);
    std::fs::create_dir_all(directory)?;
    let mut temp_file = tempfile::NamedTempFile::new_in(directory)?;
    temp_file.write_all(bytes)?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(destination)
        .map_err(|error| AppError::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn directory_entry_names(directory: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_atomically_creates_file_with_exact_bytes() {
        let dir = TempDir::new().unwrap();
        let destination = dir.path().join("out.bin");
        write_atomically(&destination, b"hello").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"hello");
    }

    #[test]
    fn write_atomically_replaces_existing_file() {
        let dir = TempDir::new().unwrap();
        let destination = dir.path().join("out.bin");
        std::fs::write(&destination, b"old content").unwrap();
        write_atomically(&destination, b"new").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    }

    #[test]
    fn write_atomically_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let destination = dir.path().join("a").join("b").join("out.bin");
        write_atomically(&destination, b"x").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"x");
    }

    #[test]
    fn write_atomically_leaves_no_temp_file_on_success() {
        let dir = TempDir::new().unwrap();
        write_atomically(&dir.path().join("out.bin"), b"x").unwrap();
        assert_eq!(directory_entry_names(dir.path()), vec!["out.bin"]);
    }

    #[test]
    fn write_atomically_leaves_no_temp_file_when_destination_is_a_directory() {
        let dir = TempDir::new().unwrap();
        let destination = dir.path().join("occupied");
        std::fs::create_dir(&destination).unwrap();
        let error = write_atomically(&destination, b"x").unwrap_err();
        assert!(matches!(error, AppError::Io(_)));
        assert_eq!(directory_entry_names(dir.path()), vec!["occupied"]);
    }

    #[test]
    fn concurrent_write_atomically_to_same_destination_yields_one_complete_file() {
        let dir = TempDir::new().unwrap();
        let destination = dir.path().join("out.bin");
        let size = 1024 * 1024;
        std::thread::scope(|scope| {
            for value in [1u8, 2u8] {
                let destination = &destination;
                scope.spawn(move || {
                    write_atomically(destination, &vec![value; size]).unwrap();
                });
            }
        });
        let contents = std::fs::read(&destination).unwrap();
        assert_eq!(contents.len(), size);
        assert!(contents.iter().all(|byte| *byte == contents[0]));
    }

    #[test]
    fn parent_directory_of_bare_file_name_is_current_directory() {
        assert_eq!(parent_directory(Path::new("out.pdf")), Path::new("."));
        assert_eq!(parent_directory(Path::new("a/out.pdf")), Path::new("a"));
    }
}
