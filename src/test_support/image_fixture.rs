use std::io::Cursor;

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb, Rgba};

const CHANNEL_MODULUS: u32 = 256;

fn gradient_value(x: u32, y: u32) -> u8 {
    ((x + y) % CHANNEL_MODULUS) as u8
}

pub fn gradient_rgb(width: u32, height: u32) -> DynamicImage {
    let buffer = ImageBuffer::from_fn(width, height, |x, y| {
        let value = gradient_value(x, y);
        Rgb([value, value.wrapping_add(40), value.wrapping_add(90)])
    });
    DynamicImage::ImageRgb8(buffer)
}

pub fn gradient_rgba_with_transparency(width: u32, height: u32) -> DynamicImage {
    let buffer = ImageBuffer::from_fn(width, height, |x, y| {
        let value = gradient_value(x, y);
        let alpha = if x < width / 2 { 0 } else { 255 };
        Rgba([value, value.wrapping_add(40), value.wrapping_add(90), alpha])
    });
    DynamicImage::ImageRgba8(buffer)
}

pub fn gradient_rgb16(width: u32, height: u32) -> DynamicImage {
    let buffer: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_fn(width, height, |x, y| {
        let value = u16::from(gradient_value(x, y)) * 257;
        Rgb([value, value / 2, value / 3])
    });
    DynamicImage::ImageRgb16(buffer)
}

pub fn encode_as(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    image
        .write_to(&mut cursor, format)
        .expect("fixture image must encode");
    cursor.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradient_rgb_is_not_uniform() {
        let image = gradient_rgb(16, 16).to_rgb8();
        assert_ne!(image.get_pixel(0, 0), image.get_pixel(5, 3));
    }

    #[test]
    fn gradient_rgba_has_transparent_left_half() {
        let image = gradient_rgba_with_transparency(8, 8).to_rgba8();
        assert_eq!(image.get_pixel(0, 0).0[3], 0);
        assert_eq!(image.get_pixel(7, 0).0[3], 255);
    }

    #[test]
    fn gradient_rgb16_uses_sixteen_bit_channels() {
        let image = gradient_rgb16(4, 4);
        assert!(matches!(image, DynamicImage::ImageRgb16(_)));
    }

    #[test]
    fn encode_as_png_starts_with_png_signature() {
        let bytes = encode_as(&gradient_rgb(4, 4), ImageFormat::Png);
        assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn encode_as_jpeg_starts_with_soi_marker() {
        let bytes = encode_as(&gradient_rgb(8, 8), ImageFormat::Jpeg);
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }
}
