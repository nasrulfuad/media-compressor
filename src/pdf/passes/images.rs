use std::collections::HashMap;

use image::DynamicImage;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use tracing::{debug, warn};

use crate::codec::jpeg::{encode_jpeg, JpegSettings, ScanMode};
use crate::codec::png::encode_png_for_pdf_predictor;
use crate::codec::zlib::deflate;
use crate::domain::options::{ColorMode, ImageEncodingMode, PdfOptions};
use crate::error::{AppError, AppResult};
use crate::pdf::filters::{decode_plain_stream, replace_with_flate_bytes, StreamEncoding};
use crate::pdf::object_graph::{is_image_stream_dict, resolve_dict};

const MIN_IMAGE_SIZE: usize = 128;
const ORPHAN_IMAGE_MIN_DIM: u32 = 300;
const FLAT_IMAGE_STDDEV: f64 = 20.0;
const COMPLEX_IMAGE_STDDEV: f64 = 60.0;
const FLAT_QUALITY_REDUCTION: u8 = 15;
const COMPLEX_QUALITY_BOOST: u8 = 5;
const MAX_QUALITY: u8 = 95;
const PDF_POINTS_PER_INCH: f64 = 72.0;
const ORPHAN_BUDGET_SQUARE_INCHES: u64 = 4;
const FLATE_IMAGE_DEFLATE_LEVEL: u32 = 9;
const SUPPORTED_BITS_PER_COMPONENT: u8 = 8;
const DEFAULT_BITS_PER_COMPONENT: u8 = 8;
const PNG_OPTIMUM_PREDICTOR: i64 = 15;

#[derive(Debug, Clone)]
pub struct PageDimensions {
    pub width_pts: f64,
    pub height_pts: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReencodedCodec {
    Jpeg,
    Flate,
    FlateWithPngPredictor { colors: u8 },
}

struct ReencodedImage {
    bytes: Vec<u8>,
    codec: ReencodedCodec,
    width: u32,
    height: u32,
    color_mode: ColorMode,
}

fn dict_u32(dict: &Dictionary, key: &[u8]) -> Option<u32> {
    match dict.get(key).ok()? {
        Object::Integer(value) => u32::try_from(*value).ok(),
        Object::Real(value) if *value >= 0.0 => Some(*value as u32),
        _ => None,
    }
}

fn image_dimensions(dict: &Dictionary) -> Option<(u32, u32)> {
    Some((dict_u32(dict, b"Width")?, dict_u32(dict, b"Height")?))
}

fn bits_per_component(dict: &Dictionary) -> u8 {
    dict_u32(dict, b"BitsPerComponent")
        .and_then(|bits| u8::try_from(bits).ok())
        .unwrap_or(DEFAULT_BITS_PER_COMPONENT)
}

fn color_space_name(dict: &Dictionary) -> String {
    match dict.get(b"ColorSpace") {
        Ok(Object::Name(name)) => String::from_utf8_lossy(name).into_owned(),
        Ok(Object::Array(items)) => match items.first() {
            Some(Object::Name(name)) => String::from_utf8_lossy(name).into_owned(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn is_safe_to_reencode(dict: &Dictionary) -> bool {
    let is_mask = matches!(dict.get(b"ImageMask"), Ok(Object::Boolean(true)));
    !is_mask && !dict.has(b"Mask") && !dict.has(b"Decode")
}

fn media_box_dimensions(document: &Document, page_id: ObjectId) -> Option<PageDimensions> {
    let page = document.get_dictionary(page_id).ok()?;
    let media_box = resolve_array_value(document, page.get(b"MediaBox").ok()?)?;
    if media_box.len() < 4 {
        return None;
    }
    let number = |object: &Object| match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    };
    let (x0, y0) = (number(&media_box[0])?, number(&media_box[1])?);
    let (x1, y1) = (number(&media_box[2])?, number(&media_box[3])?);
    Some(PageDimensions {
        width_pts: (x1 - x0).abs(),
        height_pts: (y1 - y0).abs(),
    })
}

fn resolve_array_value<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Vec<Object>> {
    crate::pdf::object_graph::resolve_array(document, object)
}

pub fn build_page_dpi_map(document: &Document) -> HashMap<ObjectId, PageDimensions> {
    let mut map = HashMap::new();
    for page_id in document.page_iter() {
        let Some(dimensions) = media_box_dimensions(document, page_id) else {
            debug!(page_id = ?page_id, "No MediaBox found for page; skipping");
            continue;
        };
        let Some(resources) = crate::pdf::object_graph::inherited_page_resources(document, page_id)
        else {
            continue;
        };
        let Some(xobjects) = resources
            .get(b"XObject")
            .ok()
            .and_then(|value| resolve_dict(document, value))
        else {
            continue;
        };
        for (_, value) in xobjects.iter() {
            if let Object::Reference(image_id) = value {
                let is_image = matches!(
                    document.objects.get(image_id),
                    Some(Object::Stream(stream)) if is_image_stream_dict(&stream.dict)
                );
                if is_image {
                    map.entry(*image_id).or_insert_with(|| dimensions.clone());
                }
            }
        }
    }
    map
}

fn estimate_complexity(image: &DynamicImage) -> f64 {
    let rgb = image.to_rgb8();
    let samples = rgb.as_raw();
    if samples.is_empty() {
        return 0.0;
    }
    let count = samples.len() as f64;
    let mean = samples.iter().map(|value| f64::from(*value)).sum::<f64>() / count;
    let variance = samples
        .iter()
        .map(|value| (f64::from(*value) - mean).powi(2))
        .sum::<f64>()
        / count;
    variance.sqrt()
}

fn adjust_quality_for_complexity(base_quality: u8, stddev: f64) -> u8 {
    if stddev < FLAT_IMAGE_STDDEV {
        base_quality.saturating_sub(FLAT_QUALITY_REDUCTION)
    } else if stddev > COMPLEX_IMAGE_STDDEV {
        base_quality
            .saturating_add(COMPLEX_QUALITY_BOOST)
            .min(MAX_QUALITY)
    } else {
        base_quality
    }
}

fn scaled_dimensions(width: u32, height: u32, scale: f64) -> (u32, u32) {
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

fn page_aware_scale(
    width: u32,
    height: u32,
    target_dpi: u32,
    page: &PageDimensions,
) -> Option<f64> {
    let width_inches = page.width_pts / PDF_POINTS_PER_INCH;
    let height_inches = page.height_pts / PDF_POINTS_PER_INCH;
    let actual_dpi = (f64::from(width) / width_inches).max(f64::from(height) / height_inches);
    (actual_dpi > f64::from(target_dpi)).then(|| f64::from(target_dpi) / actual_dpi)
}

fn orphan_scale(width: u32, height: u32, target_dpi: u32) -> Option<f64> {
    if width < ORPHAN_IMAGE_MIN_DIM && height < ORPHAN_IMAGE_MIN_DIM {
        return None;
    }
    let budget = u64::from(target_dpi) * u64::from(target_dpi) * ORPHAN_BUDGET_SQUARE_INCHES;
    let pixels = u64::from(width) * u64::from(height);
    (pixels > budget).then(|| (budget as f64 / pixels as f64).sqrt())
}

fn downsample_if_needed(
    image: DynamicImage,
    width: u32,
    height: u32,
    target_dpi: Option<u32>,
    page: Option<&PageDimensions>,
) -> DynamicImage {
    let Some(dpi) = target_dpi else {
        return image;
    };
    let scale = match page {
        Some(page) => page_aware_scale(width, height, dpi, page),
        None => orphan_scale(width, height, dpi),
    };
    let Some(scale) = scale else {
        return image;
    };
    let (new_width, new_height) = scaled_dimensions(width, height, scale);
    debug!(
        width,
        height,
        new_width,
        new_height,
        target_dpi = dpi,
        "Downsampling image"
    );
    image.resize(new_width, new_height, image::imageops::FilterType::Lanczos3)
}

fn check_pixel_limit(width: u32, height: u32, options: &PdfOptions) -> AppResult<()> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > options.limits.max_image_pixels {
        return Err(AppError::ImageTooLarge {
            width,
            height,
            limit: options.limits.max_image_pixels,
        });
    }
    Ok(())
}

fn expected_component_count(color_space: &str, data_length: usize, pixels: usize) -> Option<usize> {
    match color_space {
        "DeviceGray" => Some(1),
        "DeviceRGB" => Some(3),
        "DeviceCMYK" => Some(4),
        "ICCBased" if pixels > 0 && data_length % pixels == 0 => Some(data_length / pixels),
        _ => None,
    }
}

fn cmyk_to_rgb(cyan: u8, magenta: u8, yellow: u8, black: u8) -> [u8; 3] {
    let key = f32::from(black) / 255.0;
    let channel = |ink: u8| ((255 - ink) as f32 * (1.0 - key)).round() as u8;
    [channel(cyan), channel(magenta), channel(yellow)]
}

fn decode_raw_pixels(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &str,
    bits: u8,
) -> AppResult<DynamicImage> {
    let unsupported = || {
        AppError::ImageDecode(format!(
            "unsupported color space '{color_space}' with {bits} bits per component"
        ))
    };
    if bits != SUPPORTED_BITS_PER_COMPONENT {
        return Err(unsupported());
    }
    let pixels = width as usize * height as usize;
    let components =
        expected_component_count(color_space, data.len(), pixels).ok_or_else(unsupported)?;
    if pixels == 0 || data.len() != pixels * components {
        return Err(AppError::ImageDecode(format!(
            "expected {} bytes for {width}x{height} image, found {}",
            pixels * components,
            data.len()
        )));
    }
    let mismatch = || AppError::ImageDecode("pixel buffer does not match dimensions".to_string());
    match components {
        1 => image::GrayImage::from_raw(width, height, data.to_vec())
            .map(DynamicImage::ImageLuma8)
            .ok_or_else(mismatch),
        3 => image::RgbImage::from_raw(width, height, data.to_vec())
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(mismatch),
        4 => {
            let rgb: Vec<u8> = data
                .chunks_exact(4)
                .flat_map(|ink| cmyk_to_rgb(ink[0], ink[1], ink[2], ink[3]))
                .collect();
            image::RgbImage::from_raw(width, height, rgb)
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(mismatch)
        }
        _ => Err(unsupported()),
    }
}

fn encode_flate_pixels(image: &DynamicImage, color_mode: ColorMode) -> AppResult<Vec<u8>> {
    let raw = match color_mode {
        ColorMode::Grayscale => image.to_luma8().into_raw(),
        ColorMode::Preserve => image.to_rgb8().into_raw(),
    };
    deflate(&raw, FLATE_IMAGE_DEFLATE_LEVEL)
}

fn encode_jpeg_candidate(
    image: &DynamicImage,
    quality: u8,
    color_mode: ColorMode,
) -> AppResult<Vec<u8>> {
    encode_jpeg(
        image,
        JpegSettings {
            quality,
            color_mode,
            scan_mode: ScanMode::Progressive,
        },
    )
}

struct SourceImage {
    image: DynamicImage,
    width: u32,
    height: u32,
}

fn reencode_dct(
    stream: &Stream,
    options: &PdfOptions,
    page: Option<&PageDimensions>,
) -> AppResult<Option<ReencodedImage>> {
    if options.image_encoding == ImageEncodingMode::Lossless {
        return Ok(None);
    }
    let decoded = image::load_from_memory(&stream.content)
        .map_err(|error| AppError::ImageDecode(format!("JPEG decode failed: {error}")))?;
    check_pixel_limit(decoded.width(), decoded.height(), options)?;
    let (width, height) =
        image_dimensions(&stream.dict).unwrap_or((decoded.width(), decoded.height()));
    let source = SourceImage {
        image: decoded,
        width,
        height,
    };
    let quality =
        adjust_quality_for_complexity(options.jpeg_quality, estimate_complexity(&source.image));
    let resized = downsample_if_needed(
        source.image,
        source.width,
        source.height,
        options.target_dpi,
        page,
    );
    let bytes = encode_jpeg_candidate(&resized, quality, options.color_mode)?;
    Ok(Some(ReencodedImage {
        bytes,
        codec: ReencodedCodec::Jpeg,
        width: resized.width(),
        height: resized.height(),
        color_mode: options.color_mode,
    }))
}

fn smallest_candidate(candidates: Vec<AppResult<ReencodedImage>>) -> AppResult<ReencodedImage> {
    let mut best: Option<ReencodedImage> = None;
    let mut first_error: Option<AppError> = None;
    for candidate in candidates {
        match candidate {
            Ok(image) => {
                if best
                    .as_ref()
                    .map_or(true, |current| image.bytes.len() < current.bytes.len())
                {
                    best = Some(image);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    best.ok_or_else(|| {
        first_error.unwrap_or_else(|| AppError::Encode("no image candidate".to_string()))
    })
}

fn predictor_candidate(
    image: &DynamicImage,
    options: &PdfOptions,
    describe: &dyn Fn(Vec<u8>, ReencodedCodec) -> ReencodedImage,
) -> AppResult<ReencodedImage> {
    let source = match options.color_mode {
        ColorMode::Grayscale => DynamicImage::ImageLuma8(image.to_luma8()),
        ColorMode::Preserve => DynamicImage::ImageRgb8(image.to_rgb8()),
    };
    let idat = encode_png_for_pdf_predictor(&source, options.level.preset().png_effort)?;
    let colors = idat.colors();
    Ok(describe(
        idat.zlib_data,
        ReencodedCodec::FlateWithPngPredictor { colors },
    ))
}

fn reencode_raw(
    stream: &Stream,
    options: &PdfOptions,
    page: Option<&PageDimensions>,
) -> AppResult<Option<ReencodedImage>> {
    let (width, height) = image_dimensions(&stream.dict)
        .ok_or_else(|| AppError::ImageDecode("missing Width/Height".to_string()))?;
    check_pixel_limit(width, height, options)?;
    let raw = decode_plain_stream(stream, options.limits.max_stream_bytes)?;
    let color_space = color_space_name(&stream.dict);
    let decoded = decode_raw_pixels(
        &raw,
        width,
        height,
        &color_space,
        bits_per_component(&stream.dict),
    )?;
    let lossless = options.image_encoding == ImageEncodingMode::Lossless;
    let target_dpi = if lossless { None } else { options.target_dpi };
    let quality =
        adjust_quality_for_complexity(options.jpeg_quality, estimate_complexity(&decoded));
    let resized = downsample_if_needed(decoded, width, height, target_dpi, page);
    let describe = |bytes: Vec<u8>, codec: ReencodedCodec| ReencodedImage {
        bytes,
        codec,
        width: resized.width(),
        height: resized.height(),
        color_mode: options.color_mode,
    };
    let flate = encode_flate_pixels(&resized, options.color_mode)
        .map(|bytes| describe(bytes, ReencodedCodec::Flate));
    let predictor = predictor_candidate(&resized, options, &describe);
    let mut candidates = vec![flate, predictor];
    if !lossless {
        candidates.push(
            encode_jpeg_candidate(&resized, quality, options.color_mode)
                .map(|bytes| describe(bytes, ReencodedCodec::Jpeg)),
        );
    }
    smallest_candidate(candidates).map(Some)
}

fn predictor_parms(colors: u8, columns: u32) -> Dictionary {
    let mut parms = Dictionary::new();
    parms.set("Predictor", Object::Integer(PNG_OPTIMUM_PREDICTOR));
    parms.set("Colors", Object::Integer(i64::from(colors)));
    parms.set(
        "BitsPerComponent",
        Object::Integer(i64::from(SUPPORTED_BITS_PER_COMPONENT)),
    );
    parms.set("Columns", Object::Integer(i64::from(columns)));
    parms
}

fn apply_reencoded(stream: &mut Stream, image: ReencodedImage) {
    match image.codec {
        ReencodedCodec::Jpeg => {
            let length = image.bytes.len() as i64;
            stream.content = image.bytes;
            stream.dict.set("Length", Object::Integer(length));
            stream
                .dict
                .set("Filter", Object::Name(b"DCTDecode".to_vec()));
            stream.dict.remove(b"DecodeParms");
        }
        ReencodedCodec::Flate => replace_with_flate_bytes(stream, image.bytes),
        ReencodedCodec::FlateWithPngPredictor { colors } => {
            replace_with_flate_bytes(stream, image.bytes);
            stream.dict.set(
                "DecodeParms",
                Object::Dictionary(predictor_parms(colors, image.width)),
            );
        }
    }
    let color_space: &[u8] = match image.color_mode {
        ColorMode::Grayscale => b"DeviceGray",
        ColorMode::Preserve => b"DeviceRGB",
    };
    stream
        .dict
        .set("ColorSpace", Object::Name(color_space.to_vec()));
    stream.dict.set(
        "BitsPerComponent",
        Object::Integer(i64::from(SUPPORTED_BITS_PER_COMPONENT)),
    );
    stream
        .dict
        .set("Width", Object::Integer(i64::from(image.width)));
    stream
        .dict
        .set("Height", Object::Integer(i64::from(image.height)));
}

fn compress_image_stream(
    stream: &mut Stream,
    options: &PdfOptions,
    page: Option<&PageDimensions>,
) -> AppResult<Option<i64>> {
    let original_size = stream.content.len();
    if original_size < MIN_IMAGE_SIZE || !is_safe_to_reencode(&stream.dict) {
        return Ok(None);
    }
    let encoding = StreamEncoding::from_dict(&stream.dict);
    let candidate = if encoding.is_dct_only() {
        reencode_dct(stream, options, page)?
    } else if encoding.is_flate_or_unfiltered() && !encoding.has_predictor {
        reencode_raw(stream, options, page)?
    } else {
        debug!(filters = ?encoding.filters, "Image left untouched: unsupported filter chain or predictor");
        return Ok(None);
    };
    let Some(image) = candidate else {
        return Ok(None);
    };
    if image.bytes.len() >= original_size {
        return Ok(None);
    }
    let saved = (original_size - image.bytes.len()) as i64;
    apply_reencoded(stream, image);
    Ok(Some(saved))
}

pub fn compress_images(
    document: &mut Document,
    options: &PdfOptions,
    page_dpi_map: &HashMap<ObjectId, PageDimensions>,
) -> AppResult<i64> {
    let ids: Vec<ObjectId> = document.objects.keys().copied().collect();
    let mut total_saved = 0i64;
    for id in ids {
        let Some(Object::Stream(stream)) = document.objects.get_mut(&id) else {
            continue;
        };
        if !is_image_stream_dict(&stream.dict) {
            continue;
        }
        match compress_image_stream(stream, options, page_dpi_map.get(&id)) {
            Ok(Some(saved)) => {
                total_saved += saved;
                debug!(object_id = ?id, bytes_saved = saved, "Image re-compressed");
            }
            Ok(None) => debug!(object_id = ?id, "Image skipped (no benefit or unsupported format)"),
            Err(error) => {
                warn!(object_id = ?id, error = %error, "Failed to compress image, skipping")
            }
        }
    }
    Ok(total_saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::level::CompressionLevel;
    use crate::test_support::{decoded_image_pixels, FixtureImage, PdfFixture};

    fn options(level: CompressionLevel) -> PdfOptions {
        PdfOptions::for_level(level)
    }

    fn image_stream(document: &Document) -> (ObjectId, Stream) {
        document
            .objects
            .iter()
            .find_map(|(id, object)| match object {
                Object::Stream(stream) if is_image_stream_dict(&stream.dict) => {
                    Some((*id, stream.clone()))
                }
                _ => None,
            })
            .expect("image stream")
    }

    fn noisy_rgb_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut state: u32 = 0x1234_5678;
        (0..width * height * 3)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((state >> 24) % 16) as i64 - 8;
                ((i64::from(index / 3 % 256) + noise).clamp(0, 255)) as u8
            })
            .collect()
    }

    fn photographic_document(width: u32, height: u32) -> Document {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb { width, height })
            .build();
        let (id, mut stream) = image_stream(&document);
        let compressed = deflate(&noisy_rgb_bytes(width, height), 9).unwrap();
        replace_with_flate_bytes(&mut stream, compressed);
        document.objects.insert(id, Object::Stream(stream));
        document
    }

    fn compress(document: &mut Document, options: &PdfOptions) -> i64 {
        let map = build_page_dpi_map(document);
        compress_images(document, options, &map).unwrap()
    }

    fn make_image_dict(
        width: u32,
        height: u32,
        filter: Option<&str>,
        color_space: Option<&str>,
    ) -> Dictionary {
        let mut dict = Dictionary::new();
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Width", Object::Integer(i64::from(width)));
        dict.set("Height", Object::Integer(i64::from(height)));
        dict.set("BitsPerComponent", Object::Integer(8));
        if let Some(filter) = filter {
            dict.set("Filter", Object::Name(filter.as_bytes().to_vec()));
        }
        if let Some(space) = color_space {
            dict.set("ColorSpace", Object::Name(space.as_bytes().to_vec()));
        }
        dict
    }

    #[test]
    fn compress_images_recompresses_flate_rgb_image_smaller_and_decodable() {
        let mut document = photographic_document(128, 128);
        let (id, before) = image_stream(&document);
        let saved = compress(&mut document, &options(CompressionLevel::High));
        let (_, after) = image_stream(&document);
        assert!(saved > 0, "saved {saved}");
        assert!(after.content.len() < before.content.len());
        assert_eq!(saved, (before.content.len() - after.content.len()) as i64);
        let _ = id;
        let filter = after
            .dict
            .get(b"Filter")
            .unwrap()
            .as_name()
            .unwrap()
            .to_vec();
        assert!(filter == b"DCTDecode" || filter == b"FlateDecode");
        if filter == b"DCTDecode" {
            let decoded = image::load_from_memory(&after.content).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (128, 128));
        }
    }

    #[test]
    fn compress_images_skips_flate_image_with_png_predictor() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgbWithPngPredictor {
                width: 64,
                height: 64,
            })
            .build();
        let (_, before) = image_stream(&document);
        assert_eq!(compress(&mut document, &options(CompressionLevel::High)), 0);
        let (_, after) = image_stream(&document);
        assert_eq!(after, before);
    }

    #[test]
    fn compress_images_skips_filter_chain_image() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FilterChain {
                filters: vec!["ASCIIHexDecode", "FlateDecode"],
            })
            .build();
        let (id, before) = image_stream(&document);
        let mut padded = before.clone();
        padded.content = vec![1u8; 400];
        document.objects.insert(id, Object::Stream(padded.clone()));
        assert_eq!(compress(&mut document, &options(CompressionLevel::High)), 0);
        assert_eq!(image_stream(&document).1, padded);
    }

    #[test]
    fn compress_images_skips_image_with_color_key_mask_or_decode() {
        for key in ["Mask", "Decode"] {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .with_image(FixtureImage::FlateRgb {
                    width: 128,
                    height: 128,
                })
                .build();
            let (id, mut stream) = image_stream(&document);
            stream.dict.set(
                key,
                Object::Array(vec![Object::Integer(0), Object::Integer(255)]),
            );
            document.objects.insert(id, Object::Stream(stream.clone()));
            assert_eq!(
                compress(&mut document, &options(CompressionLevel::High)),
                0,
                "{key}"
            );
            assert_eq!(image_stream(&document).1, stream);
        }
    }

    #[test]
    fn compress_images_updates_width_and_height_when_downsampling() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 600,
                height: 600,
            })
            .build();
        let mut settings = options(CompressionLevel::Extreme);
        settings.target_dpi = Some(36);
        compress(&mut document, &settings);
        let (_, after) = image_stream(&document);
        let (width, height) = image_dimensions(&after.dict).unwrap();
        assert!(width < 600 && height < 600, "{width}x{height}");
        let raw_length = match after.dict.get(b"Filter").unwrap().as_name().unwrap() {
            b"FlateDecode" => decoded_image_pixels(&after).len(),
            _ => {
                let decoded = image::load_from_memory(&after.content).unwrap();
                assert_eq!((decoded.width(), decoded.height()), (width, height));
                width as usize * height as usize
            }
        };
        assert_eq!(raw_length, width as usize * height as usize);
    }

    #[test]
    fn compress_images_applies_grayscale_color_space_at_extreme_level() {
        let mut document = photographic_document(128, 128);
        compress(&mut document, &options(CompressionLevel::Extreme));
        let (_, after) = image_stream(&document);
        assert_eq!(
            after.dict.get(b"ColorSpace").unwrap(),
            &Object::Name(b"DeviceGray".to_vec())
        );
    }

    #[test]
    fn compress_images_lossless_mode_keeps_dct_image_untouched() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::Jpeg {
                width: 128,
                height: 128,
            })
            .build();
        let (_, before) = image_stream(&document);
        let mut settings = options(CompressionLevel::Extreme);
        settings.image_encoding = ImageEncodingMode::Lossless;
        assert_eq!(compress(&mut document, &settings), 0);
        assert_eq!(image_stream(&document).1, before);
    }

    #[test]
    fn compress_images_lossless_mode_never_uses_jpeg_for_raw_images() {
        let mut document = photographic_document(128, 128);
        let mut settings = options(CompressionLevel::Medium);
        settings.image_encoding = ImageEncodingMode::Lossless;
        compress(&mut document, &settings);
        let (_, after) = image_stream(&document);
        assert_eq!(
            after.dict.get(b"Filter").unwrap(),
            &Object::Name(b"FlateDecode".to_vec())
        );
        assert_eq!(decoded_image_pixels(&after).len(), 128 * 128 * 3);
    }

    #[test]
    fn compress_images_uses_configured_quality_for_jpeg() {
        let build = |quality: u8| {
            let mut document = PdfFixture::new()
                .page_with_content(b"q Q")
                .with_image(FixtureImage::Jpeg {
                    width: 128,
                    height: 128,
                })
                .build();
            let mut settings = options(CompressionLevel::Low);
            settings.jpeg_quality = quality;
            settings.target_dpi = None;
            compress(&mut document, &settings);
            image_stream(&document).1.content.len()
        };
        assert!(build(30) <= build(90));
    }

    #[test]
    fn compress_images_rejects_declared_dimensions_above_pixel_limit() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 64,
                height: 64,
            })
            .build();
        let (_, before) = image_stream(&document);
        let mut settings = options(CompressionLevel::High);
        settings.limits.max_image_pixels = 100;
        assert_eq!(compress(&mut document, &settings), 0);
        assert_eq!(image_stream(&document).1, before);
    }

    #[test]
    fn compress_images_never_grows_an_image() {
        let mut document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::Jpeg {
                width: 128,
                height: 128,
            })
            .build();
        let (_, before) = image_stream(&document);
        let saved = compress(&mut document, &options(CompressionLevel::Low));
        let (_, after) = image_stream(&document);
        assert!(saved >= 0);
        assert!(after.content.len() <= before.content.len());
    }

    #[test]
    fn image_dimensions_reads_width_and_height() {
        let dict = make_image_dict(320, 240, None, None);
        assert_eq!(image_dimensions(&dict), Some((320, 240)));
        assert_eq!(image_dimensions(&Dictionary::new()), None);
    }

    #[test]
    fn color_space_name_reads_name_and_array_head() {
        let mut dict = make_image_dict(1, 1, None, Some("DeviceRGB"));
        assert_eq!(color_space_name(&dict), "DeviceRGB");
        dict.set(
            "ColorSpace",
            Object::Array(vec![Object::Name(b"ICCBased".to_vec()), Object::Null]),
        );
        assert_eq!(color_space_name(&dict), "ICCBased");
        dict.remove(b"ColorSpace");
        assert_eq!(color_space_name(&dict), "");
    }

    #[test]
    fn decode_raw_pixels_rgb8_succeeds() {
        let pixels: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let image = decode_raw_pixels(&pixels, 2, 2, "DeviceRGB", 8).unwrap();
        assert_eq!((image.width(), image.height()), (2, 2));
        assert_eq!(image.to_rgb8().get_pixel(1, 0).0, [0, 255, 0]);
    }

    #[test]
    fn decode_raw_pixels_gray8_succeeds() {
        let image = decode_raw_pixels(&[0, 64, 128, 255], 4, 1, "DeviceGray", 8).unwrap();
        assert_eq!(image.to_luma8().get_pixel(3, 0).0, [255]);
    }

    #[test]
    fn decode_raw_pixels_cmyk_converts_to_rgb() {
        let image =
            decode_raw_pixels(&[0, 0, 0, 0, 255, 255, 255, 0], 2, 1, "DeviceCMYK", 8).unwrap();
        let rgb = image.to_rgb8();
        assert_eq!(rgb.get_pixel(0, 0).0, [255, 255, 255]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0, 0, 0]);
    }

    #[test]
    fn decode_raw_pixels_infers_icc_based_components_from_length() {
        let image = decode_raw_pixels(&[1, 2, 3, 4, 5, 6], 2, 1, "ICCBased", 8).unwrap();
        assert_eq!(image.to_rgb8().get_pixel(1, 0).0, [4, 5, 6]);
    }

    #[test]
    fn decode_raw_pixels_rejects_length_mismatch_and_unsupported_inputs() {
        assert!(decode_raw_pixels(&[0u8; 11], 2, 2, "DeviceRGB", 8).is_err());
        assert!(decode_raw_pixels(&[0u8; 13], 2, 2, "DeviceRGB", 8).is_err());
        assert!(decode_raw_pixels(&[0u8; 12], 2, 2, "DeviceN", 8).is_err());
        assert!(decode_raw_pixels(&[0u8; 12], 2, 2, "DeviceRGB", 16).is_err());
        assert!(decode_raw_pixels(&[], 0, 0, "DeviceRGB", 8).is_err());
    }

    #[test]
    fn downsample_skips_image_within_dpi_budget() {
        let dims = PageDimensions {
            width_pts: 72.0,
            height_pts: 72.0,
        };
        let result = downsample_if_needed(
            DynamicImage::new_rgb8(100, 100),
            100,
            100,
            Some(150),
            Some(&dims),
        );
        assert_eq!(result.width(), 100);
    }

    #[test]
    fn downsample_reduces_image_exceeding_dpi() {
        let dims = PageDimensions {
            width_pts: 72.0,
            height_pts: 72.0,
        };
        let result = downsample_if_needed(
            DynamicImage::new_rgb8(600, 600),
            600,
            600,
            Some(150),
            Some(&dims),
        );
        assert_eq!((result.width(), result.height()), (150, 150));
    }

    #[test]
    fn downsample_orphan_skips_tiny_images() {
        let result =
            downsample_if_needed(DynamicImage::new_rgb8(100, 100), 100, 100, Some(300), None);
        assert_eq!(result.width(), 100);
    }

    #[test]
    fn downsample_orphan_reduces_large_images_above_pixel_budget() {
        let result = downsample_if_needed(
            DynamicImage::new_rgb8(2000, 2000),
            2000,
            2000,
            Some(150),
            None,
        );
        assert!(result.width() < 2000);
    }

    #[test]
    fn downsample_without_target_dpi_is_identity() {
        let result =
            downsample_if_needed(DynamicImage::new_rgb8(2000, 2000), 2000, 2000, None, None);
        assert_eq!(result.width(), 2000);
    }

    #[test]
    fn estimate_complexity_flat_image_has_low_stddev() {
        let mut buffer = image::RgbImage::new(8, 8);
        for pixel in buffer.pixels_mut() {
            *pixel = image::Rgb([128u8, 128, 128]);
        }
        assert!(estimate_complexity(&DynamicImage::ImageRgb8(buffer)) < 1.0);
    }

    #[test]
    fn estimate_complexity_noisy_image_has_high_stddev() {
        let mut buffer = image::RgbImage::new(8, 8);
        for (index, pixel) in buffer.pixels_mut().enumerate() {
            let value = if index % 2 == 0 { 0u8 } else { 255u8 };
            *pixel = image::Rgb([value, value, value]);
        }
        assert!(estimate_complexity(&DynamicImage::ImageRgb8(buffer)) > 60.0);
    }

    #[test]
    fn adjust_quality_follows_complexity_bands() {
        assert_eq!(adjust_quality_for_complexity(75, 10.0), 60);
        assert_eq!(adjust_quality_for_complexity(75, 80.0), 80);
        assert_eq!(adjust_quality_for_complexity(92, 80.0), 95);
        assert_eq!(adjust_quality_for_complexity(75, 40.0), 75);
        assert_eq!(adjust_quality_for_complexity(5, 10.0), 0);
    }

    #[test]
    fn build_page_dpi_map_maps_page_images_to_media_box() {
        let document = PdfFixture::new()
            .page_with_content(b"q Q")
            .with_image(FixtureImage::FlateRgb {
                width: 8,
                height: 8,
            })
            .build();
        let map = build_page_dpi_map(&document);
        let (id, _) = image_stream(&document);
        let dims = map.get(&id).expect("image mapped");
        assert_eq!((dims.width_pts, dims.height_pts), (612.0, 792.0));
    }

    #[test]
    fn build_page_dpi_map_ignores_images_without_resources() {
        let document = PdfFixture::new().page_with_content(b"q Q").build();
        assert!(build_page_dpi_map(&document).is_empty());
    }

    fn lossless_result(width: u32, height: u32) -> (Vec<u8>, Stream) {
        let mut document = photographic_document(width, height);
        let mut settings = options(CompressionLevel::Medium);
        settings.image_encoding = ImageEncodingMode::Lossless;
        compress(&mut document, &settings);
        (noisy_rgb_bytes(width, height), image_stream(&document).1)
    }

    #[test]
    fn lossless_result_decodes_back_to_original_pixels_including_predictor_output() {
        let (original, stream) = lossless_result(96, 96);
        assert_eq!(decoded_image_pixels(&stream), original);
    }

    #[test]
    fn images_pass_prefers_smallest_candidate() {
        let (original, stream) = lossless_result(96, 96);
        let plain_deflate = deflate(&original, 9).unwrap();
        assert!(stream.content.len() <= plain_deflate.len());
    }

    #[test]
    fn predictor_candidate_sets_png_decode_parms() {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 64, |x, y| {
            image::Rgb([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8])
        }));
        let options = options(CompressionLevel::Medium);
        let describe = |bytes: Vec<u8>, codec: ReencodedCodec| ReencodedImage {
            bytes,
            codec,
            width: 64,
            height: 64,
            color_mode: ColorMode::Preserve,
        };
        let candidate = predictor_candidate(&image, &options, &describe).unwrap();
        assert_eq!(
            candidate.codec,
            ReencodedCodec::FlateWithPngPredictor { colors: 3 }
        );
        let mut stream = Stream::new(Dictionary::new(), Vec::new());
        apply_reencoded(&mut stream, candidate);
        let parms = stream.dict.get(b"DecodeParms").unwrap().as_dict().unwrap();
        assert_eq!(parms.get(b"Predictor").unwrap(), &Object::Integer(15));
        assert_eq!(parms.get(b"Colors").unwrap(), &Object::Integer(3));
        assert_eq!(parms.get(b"Columns").unwrap(), &Object::Integer(64));
        assert_eq!(decoded_image_pixels(&stream).len(), 64 * 64 * 3);
    }

    #[test]
    fn smallest_candidate_returns_smallest_and_first_error_when_all_fail() {
        let make = |size: usize| {
            Ok(ReencodedImage {
                bytes: vec![0; size],
                codec: ReencodedCodec::Flate,
                width: 1,
                height: 1,
                color_mode: ColorMode::Preserve,
            })
        };
        let best = smallest_candidate(vec![make(9), make(3), make(5)]).unwrap();
        assert_eq!(best.bytes.len(), 3);
        let failing = vec![
            Err(AppError::Encode("first".into())),
            Err(AppError::Encode("second".into())),
        ];
        assert!(
            matches!(smallest_candidate(failing), Err(AppError::Encode(message)) if message == "first")
        );
        assert!(smallest_candidate(Vec::new()).is_err());
    }
}
