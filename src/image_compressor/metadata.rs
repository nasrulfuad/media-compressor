//! EXIF / metadata handling for the image compression pipeline.
//!
//! ## Design note
//!
//! Re-encoding an image via mozjpeg or oxipng naturally strips all EXIF and XMP
//! metadata because the encoders only write raw pixel data, not metadata chunks.
//! As a result, metadata stripping is the default behaviour and requires no
//! explicit implementation for Phase B/C.
//!
//! When `keep_metadata` is `true`, a warning is emitted (see engine.rs) because
//! metadata preservation from the input file is not yet implemented.  Phase F
//! will add auto-orient-from-EXIF support, at which point this module will grow
//! a proper EXIF reader/writer.

/// Returns `true` when metadata should be stripped in the output.
///
/// Since re-encoding always strips metadata, this function is used only to
/// decide whether to warn the user that `--keep-metadata` is not yet honoured.
pub fn metadata_is_stripped_by_reencoding() -> bool {
    true
}
