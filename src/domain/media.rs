use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ImageFormat {
    #[serde(rename = "JPEG")]
    Jpeg,
    #[serde(rename = "PNG")]
    Png,
    #[serde(rename = "GIF")]
    Gif,
    #[serde(rename = "TIFF")]
    Tiff,
    #[serde(rename = "BMP")]
    Bmp,
    #[serde(rename = "WebP")]
    WebP,
}

impl ImageFormat {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Gif => "GIF",
            Self::Tiff => "TIFF",
            Self::Bmp => "BMP",
            Self::WebP => "WebP",
        }
    }

    pub fn canonical_extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Tiff => "tif",
            Self::Bmp => "bmp",
            Self::WebP => "webp",
        }
    }

    pub fn is_decodable(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png)
    }

    pub fn unsupported_input_message(self) -> String {
        format!(
            "{} input is not supported; supported image formats: JPEG, PNG",
            self.display_name()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Pdf,
    Image(ImageFormat),
    Unknown { leading_bytes: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Original,
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "(u32, u32)")]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

impl From<Dimensions> for (u32, u32) {
    fn from(dimensions: Dimensions) -> Self {
        (dimensions.width, dimensions.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_and_extension_cover_every_format() {
        let expected = [
            (ImageFormat::Jpeg, "JPEG", "jpg"),
            (ImageFormat::Png, "PNG", "png"),
            (ImageFormat::Gif, "GIF", "gif"),
            (ImageFormat::Tiff, "TIFF", "tif"),
            (ImageFormat::Bmp, "BMP", "bmp"),
            (ImageFormat::WebP, "WebP", "webp"),
        ];
        for (format, name, extension) in expected {
            assert_eq!(format.display_name(), name);
            assert_eq!(format.canonical_extension(), extension);
        }
    }

    #[test]
    fn only_jpeg_and_png_are_decodable() {
        assert!(ImageFormat::Jpeg.is_decodable());
        assert!(ImageFormat::Png.is_decodable());
        assert!(!ImageFormat::Gif.is_decodable());
        assert!(!ImageFormat::Tiff.is_decodable());
        assert!(!ImageFormat::Bmp.is_decodable());
        assert!(!ImageFormat::WebP.is_decodable());
    }

    #[test]
    fn unsupported_input_message_names_format_and_supported_set() {
        assert_eq!(
            ImageFormat::Gif.unsupported_input_message(),
            "GIF input is not supported; supported image formats: JPEG, PNG"
        );
    }

    #[test]
    fn image_format_serializes_with_display_name() {
        assert_eq!(
            serde_json::to_string(&ImageFormat::WebP).unwrap(),
            "\"WebP\""
        );
        assert_eq!(
            serde_json::to_string(&ImageFormat::Jpeg).unwrap(),
            "\"JPEG\""
        );
    }

    #[test]
    fn dimensions_serialize_as_width_height_pair() {
        let dimensions = Dimensions {
            width: 640,
            height: 480,
        };
        assert_eq!(serde_json::to_string(&dimensions).unwrap(), "[640,480]");
    }

    #[test]
    fn output_format_defaults_to_original() {
        assert_eq!(OutputFormat::default(), OutputFormat::Original);
    }
}
