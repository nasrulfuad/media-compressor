use std::path::Path;

use crate::domain::media::{ImageFormat, OutputFormat};
use crate::domain::options::ImageEncodingMode;

pub fn resolve_output_format(
    requested: OutputFormat,
    input: ImageFormat,
    encoding: ImageEncodingMode,
) -> ImageFormat {
    match requested {
        OutputFormat::Jpeg => ImageFormat::Jpeg,
        OutputFormat::Png => ImageFormat::Png,
        OutputFormat::Original => match (input, encoding) {
            (ImageFormat::Jpeg, ImageEncodingMode::Lossless) => ImageFormat::Png,
            (ImageFormat::Png, _) => ImageFormat::Png,
            _ => ImageFormat::Jpeg,
        },
    }
}

pub fn output_extension(
    input_path: &Path,
    input_format: ImageFormat,
    output_format: ImageFormat,
) -> String {
    if output_format == input_format {
        if let Some(extension) = input_path.extension() {
            return extension.to_string_lossy().into_owned();
        }
    }
    output_format.canonical_extension().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(requested: OutputFormat, input: ImageFormat) -> ImageFormat {
        resolve_output_format(requested, input, ImageEncodingMode::Lossy)
    }

    #[test]
    fn resolve_original_keeps_jpeg_and_png() {
        assert_eq!(
            resolve(OutputFormat::Original, ImageFormat::Jpeg),
            ImageFormat::Jpeg
        );
        assert_eq!(
            resolve(OutputFormat::Original, ImageFormat::Png),
            ImageFormat::Png
        );
    }

    #[test]
    fn resolve_explicit_format_wins_over_input() {
        assert_eq!(
            resolve(OutputFormat::Jpeg, ImageFormat::Png),
            ImageFormat::Jpeg
        );
        assert_eq!(
            resolve(OutputFormat::Png, ImageFormat::Jpeg),
            ImageFormat::Png
        );
    }

    #[test]
    fn resolve_original_jpeg_in_lossless_mode_becomes_png() {
        let format = resolve_output_format(
            OutputFormat::Original,
            ImageFormat::Jpeg,
            ImageEncodingMode::Lossless,
        );
        assert_eq!(format, ImageFormat::Png);
    }

    #[test]
    fn resolve_explicit_jpeg_stays_jpeg_in_lossless_mode() {
        let format = resolve_output_format(
            OutputFormat::Jpeg,
            ImageFormat::Png,
            ImageEncodingMode::Lossless,
        );
        assert_eq!(format, ImageFormat::Jpeg);
    }

    #[test]
    fn resolve_original_for_other_formats_falls_back_to_jpeg() {
        for input in [
            ImageFormat::Gif,
            ImageFormat::Tiff,
            ImageFormat::Bmp,
            ImageFormat::WebP,
        ] {
            assert_eq!(resolve(OutputFormat::Original, input), ImageFormat::Jpeg);
        }
    }

    #[test]
    fn output_extension_keeps_input_extension_when_format_is_unchanged() {
        let extension = output_extension(
            Path::new("a/photo.JPEG"),
            ImageFormat::Jpeg,
            ImageFormat::Jpeg,
        );
        assert_eq!(extension, "JPEG");
    }

    #[test]
    fn output_extension_uses_canonical_extension_after_conversion() {
        let extension =
            output_extension(Path::new("photo.png"), ImageFormat::Png, ImageFormat::Jpeg);
        assert_eq!(extension, "jpg");
    }

    #[test]
    fn output_extension_uses_canonical_extension_when_input_has_none() {
        let extension = output_extension(Path::new("photo"), ImageFormat::Png, ImageFormat::Png);
        assert_eq!(extension, "png");
    }
}
