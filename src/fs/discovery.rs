use std::path::{Path, PathBuf};

use globset::GlobBuilder;
use tracing::warn;
use walkdir::{DirEntry, WalkDir};

use crate::error::AppResult;

pub struct DiscoveryRequest<'a> {
    pub root: &'a Path,
    pub pattern: &'a str,
    pub recursive: bool,
    pub excluded_dir: Option<&'a Path>,
}

fn max_depth(recursive: bool) -> usize {
    if recursive {
        usize::MAX
    } else {
        1
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_excluded(entry: &DirEntry, excluded: Option<&PathBuf>) -> bool {
    match excluded {
        Some(excluded_dir) => {
            entry.file_type().is_dir() && canonical_or_original(entry.path()) == *excluded_dir
        }
        None => false,
    }
}

pub fn discover_files(request: &DiscoveryRequest<'_>) -> AppResult<Vec<PathBuf>> {
    let matcher = GlobBuilder::new(request.pattern)
        .case_insensitive(true)
        .build()?
        .compile_matcher();
    let excluded = request.excluded_dir.map(canonical_or_original);

    let mut files: Vec<PathBuf> = WalkDir::new(request.root)
        .max_depth(max_depth(request.recursive))
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !is_excluded(entry, excluded.as_ref()))
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(error) => {
                warn!(error = %error, "Skipping unreadable directory entry");
                None
            }
        })
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| matcher.is_match(name))
        })
        .map(DirEntry::into_path)
        .collect();

    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const ALL_MEDIA_PATTERN: &str = "*.{pdf,jpg,jpeg,png}";

    fn request<'a>(root: &'a Path, pattern: &'a str, recursive: bool) -> DiscoveryRequest<'a> {
        DiscoveryRequest {
            root,
            pattern,
            recursive,
            excluded_dir: None,
        }
    }

    fn names(root: &Path, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|file| {
                file.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn discover_files_empty_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let files = discover_files(&request(dir.path(), "*.pdf", false)).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn discover_files_filters_by_pattern_and_sorts() {
        let dir = TempDir::new().unwrap();
        for name in ["b.pdf", "a.pdf", "c.txt"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        let files = discover_files(&request(dir.path(), "*.pdf", false)).unwrap();
        assert_eq!(names(dir.path(), &files), vec!["a.pdf", "b.pdf"]);
    }

    #[test]
    fn discover_files_non_recursive_ignores_subdirectories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("deep.pdf"), b"x").unwrap();
        fs::write(dir.path().join("top.pdf"), b"x").unwrap();
        let files = discover_files(&request(dir.path(), "*.pdf", false)).unwrap();
        assert_eq!(names(dir.path(), &files), vec!["top.pdf"]);
    }

    #[test]
    fn discover_files_recursive_includes_subdirectories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("deep.pdf"), b"x").unwrap();
        fs::write(dir.path().join("top.pdf"), b"x").unwrap();
        let files = discover_files(&request(dir.path(), "*.pdf", true)).unwrap();
        assert_eq!(names(dir.path(), &files), vec!["sub/deep.pdf", "top.pdf"]);
    }

    #[test]
    fn discover_files_matches_pattern_case_insensitively() {
        let dir = TempDir::new().unwrap();
        for name in ["UPPER.PDF", "Photo.JpG", "skip.gif"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        let files = discover_files(&request(dir.path(), ALL_MEDIA_PATTERN, false)).unwrap();
        assert_eq!(names(dir.path(), &files), vec!["Photo.JpG", "UPPER.PDF"]);
    }

    #[test]
    fn discover_files_skips_excluded_output_dir() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("out");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("already.pdf"), b"x").unwrap();
        fs::write(dir.path().join("new.pdf"), b"x").unwrap();
        let request = DiscoveryRequest {
            root: dir.path(),
            pattern: "*.pdf",
            recursive: true,
            excluded_dir: Some(&output),
        };
        let files = discover_files(&request).unwrap();
        assert_eq!(names(dir.path(), &files), vec!["new.pdf"]);
    }

    #[cfg(unix)]
    #[test]
    fn discover_files_does_not_follow_symlinks() {
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.pdf"), b"x").unwrap();
        let root = TempDir::new().unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.pdf"),
            root.path().join("link.pdf"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked_dir")).unwrap();
        let files = discover_files(&request(root.path(), "*.pdf", true)).unwrap();
        assert!(files.is_empty(), "{files:?}");
    }

    #[test]
    fn discover_files_rejects_invalid_pattern() {
        let dir = TempDir::new().unwrap();
        let error = discover_files(&request(dir.path(), "[", false)).unwrap_err();
        assert!(matches!(error, crate::error::AppError::InvalidGlob(_)));
    }
}
