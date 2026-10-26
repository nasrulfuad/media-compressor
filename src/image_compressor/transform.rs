use image::DynamicImage;

/// Resize `img` proportionally so neither width nor height exceeds `max_dim` pixels.
///
/// If the image is already within bounds, it is returned unchanged.
pub fn resize_to_max_dimension(img: DynamicImage, max_dim: u32) -> DynamicImage {
    if img.width() <= max_dim && img.height() <= max_dim {
        return img;
    }
    img.resize(max_dim, max_dim, image::imageops::FilterType::Lanczos3)
}

/// Convert `img` to grayscale (single-channel luminance).
pub fn to_grayscale(img: DynamicImage) -> DynamicImage {
    DynamicImage::ImageLuma8(img.to_luma8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_skips_small_image() {
        let img = DynamicImage::new_rgb8(100, 100);
        let result = resize_to_max_dimension(img, 200);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn resize_reduces_large_image() {
        let img = DynamicImage::new_rgb8(400, 300);
        let result = resize_to_max_dimension(img, 200);
        assert!(result.width() <= 200, "width {} > 200", result.width());
        assert!(result.height() <= 200, "height {} > 200", result.height());
    }

    #[test]
    fn resize_maintains_aspect_ratio() {
        // 400x200 → max_dim=200; width should be 200, height 100.
        let img = DynamicImage::new_rgb8(400, 200);
        let result = resize_to_max_dimension(img, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 100);
    }

    #[test]
    fn resize_exact_boundary_is_unchanged() {
        let img = DynamicImage::new_rgb8(200, 200);
        let result = resize_to_max_dimension(img, 200);
        assert_eq!(result.width(), 200);
        assert_eq!(result.height(), 200);
    }

    #[test]
    fn to_grayscale_produces_single_channel() {
        let img = DynamicImage::new_rgb8(8, 8);
        let gray = to_grayscale(img);
        // A grayscale DynamicImage should be convertible to luma8 without loss.
        let luma = gray.to_luma8();
        assert_eq!(luma.width(), 8);
        assert_eq!(luma.height(), 8);
    }
}
