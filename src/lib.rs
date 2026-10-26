/// Batch PDF compression — walk folders, filter by glob, compress in parallel.
pub mod batch;

/// Strategy pattern — file-type detection, dispatch, and compressor contracts.
pub mod strategy;

/// Image compression pipeline (JPEG via mozjpeg, PNG via oxipng, transforms).
pub mod image_compressor;

/// Command-line interface definitions and argument parsing.
pub mod cli;

/// PDF compression engine, levels, and stream processing.
pub mod compressor;

/// PDF I/O — reading and writing documents to disk.
pub mod io;

/// Compression statistics and report formatting.
pub mod stats;

/// Application error types.
pub mod error;

/// Shared utility functions.
pub mod utils;
