use std::io::Cursor;

use image::DynamicImage;

use crate::error::{AppError, AppResult};

pub const MAX_OXIPNG_PRESET: u8 = 6;
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const CHUNK_HEADER_BYTES: usize = 8;
const CHUNK_CRC_BYTES: usize = 4;
const IHDR_MIN_BYTES: usize = 13;
const COLOR_TYPE_GRAY: u8 = 0;
const COLOR_TYPE_RGB: u8 = 2;
const PDF_PREDICTOR_BIT_DEPTH: u8 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngIdat {
    pub zlib_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
}

impl PngIdat {
    pub fn colors(&self) -> u8 {
        if self.color_type == COLOR_TYPE_GRAY {
            1
        } else {
            3
        }
    }
}

pub fn encode_png(image: &DynamicImage, effort: u8) -> AppResult<Vec<u8>> {
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| AppError::Encode(format!("PNG encode: {error}")))?;
    optimize_png(&encoded.into_inner(), effort)
}

pub fn optimize_png(png_bytes: &[u8], effort: u8) -> AppResult<Vec<u8>> {
    let preset = effort.min(MAX_OXIPNG_PRESET);
    oxipng::optimize_from_memory(png_bytes, &oxipng::Options::from_preset(preset))
        .map_err(|error| AppError::Encode(format!("oxipng optimization: {error}")))
}

fn invalid_png(message: &str) -> AppError {
    AppError::Encode(format!("invalid PNG: {message}"))
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn parse_ihdr(data: &[u8]) -> AppResult<(u32, u32, u8, u8)> {
    if data.len() < IHDR_MIN_BYTES {
        return Err(invalid_png("truncated IHDR"));
    }
    let interlace = data[12];
    if interlace != 0 {
        return Err(invalid_png("interlaced images are not supported"));
    }
    Ok((
        read_u32(&data[0..4]),
        read_u32(&data[4..8]),
        data[8],
        data[9],
    ))
}

pub fn extract_idat(png_bytes: &[u8]) -> AppResult<PngIdat> {
    if !png_bytes.starts_with(&PNG_SIGNATURE) {
        return Err(invalid_png("missing signature"));
    }
    let mut position = PNG_SIGNATURE.len();
    let mut header: Option<(u32, u32, u8, u8)> = None;
    let mut zlib_data = Vec::new();
    while position + CHUNK_HEADER_BYTES <= png_bytes.len() {
        let length = read_u32(&png_bytes[position..position + 4]) as usize;
        let chunk_type = &png_bytes[position + 4..position + CHUNK_HEADER_BYTES];
        let data_start = position + CHUNK_HEADER_BYTES;
        let data_end = data_start
            .checked_add(length)
            .filter(|end| end + CHUNK_CRC_BYTES <= png_bytes.len())
            .ok_or_else(|| invalid_png("truncated chunk"))?;
        let data = &png_bytes[data_start..data_end];
        match chunk_type {
            b"IHDR" => header = Some(parse_ihdr(data)?),
            b"IDAT" => zlib_data.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        position = data_end + CHUNK_CRC_BYTES;
    }
    let (width, height, bit_depth, color_type) =
        header.ok_or_else(|| invalid_png("missing IHDR"))?;
    if color_type != COLOR_TYPE_GRAY && color_type != COLOR_TYPE_RGB {
        return Err(invalid_png("unsupported color type for PDF predictor"));
    }
    if zlib_data.is_empty() {
        return Err(invalid_png("missing IDAT"));
    }
    Ok(PngIdat {
        zlib_data,
        width,
        height,
        bit_depth,
        color_type,
    })
}

pub fn pdf_safe_oxipng_options(effort: u8) -> oxipng::Options {
    let mut options = oxipng::Options::from_preset(effort.min(MAX_OXIPNG_PRESET));
    options.bit_depth_reduction = false;
    options.color_type_reduction = false;
    options.palette_reduction = false;
    options.grayscale_reduction = false;
    options.interlace = Some(oxipng::Interlacing::None);
    options
}

pub fn encode_png_for_pdf_predictor(image: &DynamicImage, effort: u8) -> AppResult<PngIdat> {
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|error| AppError::Encode(format!("PNG encode: {error}")))?;
    let optimized =
        oxipng::optimize_from_memory(&encoded.into_inner(), &pdf_safe_oxipng_options(effort))
            .map_err(|error| AppError::Encode(format!("oxipng optimization: {error}")))?;
    let idat = extract_idat(&optimized)?;
    if idat.bit_depth != PDF_PREDICTOR_BIT_DEPTH {
        return Err(invalid_png("unexpected bit depth for PDF predictor"));
    }
    Ok(idat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{gradient_rgb, gradient_rgba_with_transparency};

    const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn encode_png_produces_png_signature_and_round_trips_pixels() {
        let image = gradient_rgb(16, 16);
        let bytes = encode_png(&image, 2).unwrap();
        assert_eq!(&bytes[..8], &PNG_SIGNATURE);
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.to_rgb8(), image.to_rgb8());
    }

    #[test]
    fn encode_png_keeps_grayscale_color_type() {
        let gray = DynamicImage::ImageLuma8(gradient_rgb(8, 8).to_luma8());
        let bytes = encode_png(&gray, 2).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!(decoded.color().channel_count(), 1);
    }

    #[test]
    fn encode_png_keeps_alpha_channel() {
        let image = gradient_rgba_with_transparency(8, 8);
        let bytes = encode_png(&image, 2).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0[3], 0);
        assert_eq!(decoded.get_pixel(7, 0).0[3], 255);
    }

    #[test]
    fn encode_png_clamps_effort_above_maximum() {
        let bytes = encode_png(&gradient_rgb(8, 8), 255).unwrap();
        assert_eq!(&bytes[..8], &PNG_SIGNATURE);
    }

    #[test]
    fn optimize_png_never_returns_invalid_png_for_garbage() {
        let error = optimize_png(b"not a png", 2).unwrap_err();
        assert!(matches!(error, AppError::Encode(_)));
    }

    fn png_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    }

    fn ihdr(width: u32, height: u32, color_type: u8, interlace: u8) -> Vec<u8> {
        let mut data = width.to_be_bytes().to_vec();
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&[8, color_type, 0, 0, interlace]);
        data
    }

    fn assemble(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        for chunk in chunks {
            png.extend_from_slice(chunk);
        }
        png
    }

    #[test]
    fn extract_idat_reads_dimensions_and_concatenates_chunks() {
        let png = assemble(&[
            png_chunk(b"IHDR", &ihdr(7, 3, 2, 0)),
            png_chunk(b"tEXt", b"ignored"),
            png_chunk(b"IDAT", b"abc"),
            png_chunk(b"IDAT", b"def"),
            png_chunk(b"IEND", b""),
        ]);
        let idat = extract_idat(&png).unwrap();
        assert_eq!(idat.zlib_data, b"abcdef");
        assert_eq!(
            (idat.width, idat.height, idat.bit_depth, idat.color_type),
            (7, 3, 8, 2)
        );
        assert_eq!(idat.colors(), 3);
    }

    #[test]
    fn extract_idat_rejects_interlaced_png() {
        let png = assemble(&[
            png_chunk(b"IHDR", &ihdr(2, 2, 2, 1)),
            png_chunk(b"IDAT", b"x"),
            png_chunk(b"IEND", b""),
        ]);
        assert!(extract_idat(&png).is_err());
    }

    #[test]
    fn extract_idat_rejects_color_types_without_predictor_support() {
        for color_type in [3u8, 4, 6] {
            let png = assemble(&[
                png_chunk(b"IHDR", &ihdr(2, 2, color_type, 0)),
                png_chunk(b"IDAT", b"x"),
                png_chunk(b"IEND", b""),
            ]);
            assert!(extract_idat(&png).is_err(), "color type {color_type}");
        }
    }

    #[test]
    fn extract_idat_rejects_missing_signature_truncation_and_missing_data() {
        assert!(extract_idat(b"not a png").is_err());
        let truncated = assemble(&[png_chunk(b"IHDR", &ihdr(2, 2, 2, 0))[..10].to_vec()]);
        assert!(extract_idat(&truncated).is_err());
        let without_idat = assemble(&[
            png_chunk(b"IHDR", &ihdr(2, 2, 2, 0)),
            png_chunk(b"IEND", b""),
        ]);
        assert!(extract_idat(&without_idat).is_err());
        let without_header = assemble(&[png_chunk(b"IDAT", b"x"), png_chunk(b"IEND", b"")]);
        assert!(extract_idat(&without_header).is_err());
    }

    #[test]
    fn extract_idat_reports_gray_as_one_color() {
        let png = assemble(&[
            png_chunk(b"IHDR", &ihdr(2, 2, 0, 0)),
            png_chunk(b"IDAT", b"x"),
            png_chunk(b"IEND", b""),
        ]);
        assert_eq!(extract_idat(&png).unwrap().colors(), 1);
    }

    #[test]
    fn pdf_safe_oxipng_options_disable_every_reduction() {
        let options = pdf_safe_oxipng_options(4);
        assert!(!options.bit_depth_reduction);
        assert!(!options.color_type_reduction);
        assert!(!options.palette_reduction);
        assert!(!options.grayscale_reduction);
        assert_eq!(options.interlace, Some(oxipng::Interlacing::None));
    }

    #[test]
    fn encode_png_for_pdf_predictor_keeps_rgb_and_gray_layouts() {
        let rgb = encode_png_for_pdf_predictor(&gradient_rgb(16, 8), 2).unwrap();
        assert_eq!(
            (rgb.width, rgb.height, rgb.bit_depth, rgb.colors()),
            (16, 8, 8, 3)
        );
        let gray_image = DynamicImage::ImageLuma8(gradient_rgb(16, 8).to_luma8());
        let gray = encode_png_for_pdf_predictor(&gray_image, 2).unwrap();
        assert_eq!(gray.colors(), 1);
    }
}
