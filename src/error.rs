use std::path::PathBuf;

use thiserror::Error;

pub const EXIT_CODE_RUNTIME_ERROR: u8 = 1;
pub const EXIT_CODE_USAGE_ERROR: u8 = 2;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("input file not found: {}", .0.display())]
    InputNotFound(PathBuf),
    #[error("input file '{}' is {size} bytes, which exceeds the limit of {limit} bytes", path.display())]
    InputTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("'{}' is not a directory", .0.display())]
    InvalidInputDir(PathBuf),
    #[error("output file '{}' already exists; use --overwrite to replace it", .0.display())]
    OutputExists(PathBuf),
    #[error("'{}' is outside the input directory", .0.display())]
    PathOutsideInput(PathBuf),
    #[error("output path would overwrite the input file '{}'", .0.display())]
    OutputEqualsInput(PathBuf),
    #[error("invalid PDF: {0}")]
    InvalidPdf(String),
    #[error("PDF is encrypted and cannot be processed without a password")]
    EncryptedPdf,
    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),
    #[error("image decode error: {0}")]
    ImageDecode(String),
    #[error("encode error: {0}")]
    Encode(String),
    #[error("decompression failed: {0}")]
    Decompress(String),
    #[error("decompressed data exceeds the limit of {limit} bytes")]
    DecompressionLimitExceeded { limit: usize },
    #[error("image of {width}x{height} pixels exceeds the limit of {limit} pixels")]
    ImageTooLarge { width: u32, height: u32, limit: u64 },
    #[error("invalid content stream: {0}")]
    InvalidContentStream(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlob(#[from] globset::Error),
    #[error("JSON serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("PDF library error: {0}")]
    Lopdf(#[from] lopdf::Error),
}

impl AppError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::OutputExists(_) | Self::InvalidInputDir(_) | Self::InvalidGlob(_) => {
                EXIT_CODE_USAGE_ERROR
            }
            _ => EXIT_CODE_RUNTIME_ERROR,
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_is_usage_error_for_output_exists() {
        let error = AppError::OutputExists(PathBuf::from("a.pdf"));
        assert_eq!(error.exit_code(), EXIT_CODE_USAGE_ERROR);
    }

    #[test]
    fn exit_code_is_usage_error_for_invalid_input_dir() {
        let error = AppError::InvalidInputDir(PathBuf::from("nope"));
        assert_eq!(error.exit_code(), EXIT_CODE_USAGE_ERROR);
    }

    #[test]
    fn exit_code_is_runtime_error_for_other_variants() {
        assert_eq!(AppError::EncryptedPdf.exit_code(), EXIT_CODE_RUNTIME_ERROR);
        assert_eq!(
            AppError::Encode("x".into()).exit_code(),
            EXIT_CODE_RUNTIME_ERROR
        );
    }

    #[test]
    fn display_output_exists_mentions_overwrite_flag() {
        let message = AppError::OutputExists(PathBuf::from("a.pdf")).to_string();
        assert_eq!(
            message,
            "output file 'a.pdf' already exists; use --overwrite to replace it"
        );
    }

    #[test]
    fn display_input_too_large_includes_sizes() {
        let error = AppError::InputTooLarge {
            path: PathBuf::from("big.pdf"),
            size: 10,
            limit: 5,
        };
        assert_eq!(
            error.to_string(),
            "input file 'big.pdf' is 10 bytes, which exceeds the limit of 5 bytes"
        );
    }
}
