use thiserror::Error;

/// Application-level errors for the PDF compressor.
#[derive(Debug, Error)]
pub enum AppError {
    /// Wraps standard I/O errors.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The file is not a valid PDF document.
    #[error("Invalid PDF: {0}")]
    InvalidPdf(String),

    /// The PDF is password-protected and cannot be processed.
    #[error("PDF is encrypted and cannot be processed without a password")]
    EncryptedPdf,

    /// An image stream inside the PDF could not be decoded.
    #[error("Image decode error: {0}")]
    ImageDecode(String),

    /// An image stream inside the PDF could not be re-encoded.
    #[error("Image encode error: {0}")]
    ImageEncode(String),

    /// Wraps errors originating from the lopdf library.
    #[error("PDF library error: {0}")]
    Lopdf(#[from] lopdf::Error),

    /// Error encountered while walking a directory tree.
    #[error("Directory walk error: {0}")]
    WalkDir(String),

    /// The file format is not supported by any registered strategy.
    #[error("Unsupported file type: {0}")]
    UnsupportedFormat(String),

    /// A catch-all for other runtime errors.
    #[error("{0}")]
    Custom(String),
}

/// Convenience alias used throughout the codebase.
pub type AppResult<T> = Result<T, AppError>;
