use image::{DynamicImage, RgbImage};

pub fn resize_to_max_dimension(image: DynamicImage, max_dimension: u32) -> DynamicImage {
    if image.width() <= max_dimension && image.height() <= max_dimension {
        return image;
    }
    image.resize(
        max_dimension,
        max_dimension,
        image::imageops::FilterType::Lanczos3,
    )
}

pub fn to_grayscale(image: DynamicImage) -> DynamicImage {
    if image.color().has_alpha() {
        DynamicImage::ImageLumaA8(image.to_luma_alpha8())
    } else {
        DynamicImage::ImageLuma8(image.to_luma8())
    }
}

pub fn flatten_onto_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    let mut flattened = RgbImage::new(rgba.width(), rgba.height());
    for (target, source) in flattened.pixels_mut().zip(rgba.pixels()) {
        let alpha = u32::from(source.0[3]);
        let blend =
            |channel: u8| ((u32::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        *target = image::Rgb([blend(source.0[0]), blend(source.0[1]), blend(source.0[2])]);
    }
    flattened
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{gradient_rgb, gradient_rgba_with_transparency};

    #[test]
    fn resize_skips_small_image() {
        let result = resize_to_max_dimension(DynamicImage::new_rgb8(100, 100), 200);
        assert_eq!((result.width(), result.height()), (100, 100));
    }

    #[test]
    fn resize_maintains_aspect_ratio() {
        let result = resize_to_max_dimension(DynamicImage::new_rgb8(400, 200), 200);
        assert_eq!((result.width(), result.height()), (200, 100));
    }

    #[test]
    fn resize_exact_boundary_is_unchanged() {
        let result = resize_to_max_dimension(DynamicImage::new_rgb8(200, 200), 200);
        assert_eq!((result.width(), result.height()), (200, 200));
    }

    #[test]
    fn resize_reduces_tall_image_by_height() {
        let result = resize_to_max_dimension(DynamicImage::new_rgb8(100, 400), 200);
        assert_eq!((result.width(), result.height()), (50, 200));
    }

    #[test]
    fn to_grayscale_produces_single_channel_for_opaque_image() {
        let gray = to_grayscale(gradient_rgb(8, 8));
        assert_eq!(gray.color().channel_count(), 1);
        assert_eq!((gray.width(), gray.height()), (8, 8));
    }

    #[test]
    fn to_grayscale_keeps_alpha_channel() {
        let gray = to_grayscale(gradient_rgba_with_transparency(8, 8));
        assert_eq!(gray.color().channel_count(), 2);
        assert_eq!(gray.to_luma_alpha8().get_pixel(0, 0).0[1], 0);
    }

    #[test]
    fn flatten_onto_white_turns_transparent_pixels_white_and_keeps_opaque_ones() {
        let flattened = flatten_onto_white(&gradient_rgba_with_transparency(8, 8));
        assert_eq!(flattened.get_pixel(0, 0).0, [255, 255, 255]);
        let opaque = gradient_rgba_with_transparency(8, 8).to_rgba8();
        let expected = opaque.get_pixel(7, 0).0;
        assert_eq!(
            flattened.get_pixel(7, 0).0,
            [expected[0], expected[1], expected[2]]
        );
    }

    #[test]
    fn flatten_onto_white_blends_half_transparent_pixel() {
        let mut buffer = image::RgbaImage::new(1, 1);
        buffer.put_pixel(0, 0, image::Rgba([0, 0, 0, 128]));
        let flattened = flatten_onto_white(&DynamicImage::ImageRgba8(buffer));
        assert_eq!(flattened.get_pixel(0, 0).0, [127, 127, 127]);
    }
}
