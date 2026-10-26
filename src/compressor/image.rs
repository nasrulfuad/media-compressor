use image::DynamicImage;
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::HashMap;
use tracing::{debug, warn};

use crate::compressor::level::CompressionLevel;
use crate::compressor::pdf_utils::{
    get_primary_filter, zlib_compress_best, zlib_decompress, COMPLEX_IMAGE_STDDEV,
    COMPLEX_QUALITY_BOOST, FLAT_IMAGE_STDDEV, FLAT_QUALITY_REDUCTION, MAX_QUALITY, MIN_IMAGE_SIZE,
    ORPHAN_IMAGE_MIN_DIM,
};
use crate::error::AppResult;

// ── Public API ────────────────────────────────────────────────────────────────

/// Per-image context derived from page MediaBox information.
///
/// Values are in PDF points (1 pt = 1/72 inch).
#[derive(Debug, Clone)]
pub struct PageDimensions {
    /// MediaBox width in points.
    pub width_pts: f64,
    /// MediaBox height in points.
    pub height_pts: f64,
}

/// Compress all image XObjects in the document.
///
/// `page_dpi_map` maps image `ObjectId`s to the [`PageDimensions`] of the page
/// that uses them, enabling accurate DPI calculation. Images not present in the
/// map are treated as orphans and processed with a pixel-count heuristic.
///
/// Returns the total bytes saved (may be negative when some images expand after
/// re-encoding, though the engine only replaces smaller results).
pub fn compress_images(
    doc: &mut lopdf::Document,
    quality: u8,
    target_dpi: Option<u32>,
    level: CompressionLevel,
    page_dpi_map: &HashMap<ObjectId, PageDimensions>,
) -> AppResult<i64> {
    // Collect IDs first to avoid simultaneous mutable/immutable borrows.
    let ids: Vec<lopdf::ObjectId> = doc.objects.keys().copied().collect();

    let mut total_saved: i64 = 0;

    let grayscale = level.convert_to_grayscale();

    for id in ids {
        let obj = match doc.objects.get_mut(&id) {
            Some(o) => o,
            None => continue,
        };

        let stream = match obj {
            Object::Stream(s) => s,
            _ => continue,
        };

        if !is_image_stream(&stream.dict) {
            continue;
        }

        let page_dims = page_dpi_map.get(&id);

        match compress_image_stream(stream, quality, target_dpi, page_dims, grayscale) {
            Ok(Some(saved)) => {
                total_saved += saved;
                debug!(object_id = ?id, bytes_saved = saved, "Image re-compressed");
            }
            Ok(None) => {
                debug!(object_id = ?id, "Image skipped (no benefit or unsupported format)");
            }
            Err(e) => {
                warn!(object_id = ?id, error = %e, "Failed to compress image, skipping");
            }
        }
    }

    Ok(total_saved)
}

/// Build a map from image `ObjectId` → [`PageDimensions`] of the page that
/// references the image.
///
/// The map is used by [`compress_images`] to calculate accurate per-image DPI
/// values rather than relying on a hardcoded page-size assumption.
pub fn build_page_dpi_map(doc: &lopdf::Document) -> HashMap<ObjectId, PageDimensions> {
    let mut map: HashMap<ObjectId, PageDimensions> = HashMap::new();

    for (_page_num, page_id) in doc.get_pages() {
        // Extract the MediaBox for this page.
        let dims = match get_page_media_box(doc, page_id) {
            Some(d) => d,
            None => {
                debug!(page_id = ?page_id, "No MediaBox found for page; skipping");
                continue;
            }
        };

        // Walk the page's Resources/XObject dictionary looking for image refs.
        let resources = match doc.get_page_resources(page_id) {
            Ok((Some(res_dict), _)) => res_dict.clone(),
            _ => continue,
        };

        let xobject_dict = match resources.get(b"XObject") {
            Ok(Object::Dictionary(d)) => d.clone(),
            Ok(Object::Reference(ref_id)) => match doc.objects.get(ref_id) {
                Some(Object::Dictionary(d)) => d.clone(),
                _ => continue,
            },
            _ => continue,
        };

        for (_, val) in xobject_dict.iter() {
            let ref_id = match val {
                Object::Reference(id) => *id,
                _ => continue,
            };

            // Only map image XObjects.
            let is_image = doc
                .objects
                .get(&ref_id)
                .and_then(|o| o.as_stream().ok())
                .map(|s| {
                    matches!(
                        s.dict.get(b"Subtype"),
                        Ok(Object::Name(n)) if n == b"Image"
                    )
                })
                .unwrap_or(false);

            if is_image {
                // First page wins if the same image appears on multiple pages;
                // using the first encountered page is a reasonable approximation.
                map.entry(ref_id).or_insert_with(|| dims.clone());
            }
        }
    }

    map
}

// ── Stream classification ─────────────────────────────────────────────────────

fn is_image_stream(dict: &Dictionary) -> bool {
    matches!(
        dict.get(b"Subtype"),
        Ok(Object::Name(name)) if name == b"Image"
    )
}

// ── Dimension helpers ─────────────────────────────────────────────────────────

fn get_image_dimensions(dict: &Dictionary) -> Option<(u32, u32)> {
    let width = dict.get(b"Width").ok().and_then(|o| match o {
        Object::Integer(n) => Some(*n as u32),
        Object::Real(r) => Some(*r as u32),
        _ => None,
    })?;
    let height = dict.get(b"Height").ok().and_then(|o| match o {
        Object::Integer(n) => Some(*n as u32),
        Object::Real(r) => Some(*r as u32),
        _ => None,
    })?;
    Some((width, height))
}

fn get_bits_per_component(dict: &Dictionary) -> u8 {
    dict.get(b"BitsPerComponent")
        .ok()
        .and_then(|o| match o {
            Object::Integer(n) => Some(*n as u8),
            _ => None,
        })
        .unwrap_or(8)
}

fn get_color_space(dict: &Dictionary) -> String {
    match dict.get(b"ColorSpace") {
        Ok(Object::Name(name)) => String::from_utf8_lossy(name).into_owned(),
        Ok(Object::Array(arr)) => {
            if let Some(Object::Name(name)) = arr.first() {
                String::from_utf8_lossy(name).into_owned()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Extract the MediaBox of a page as `(width_pts, height_pts)`.
fn get_page_media_box(doc: &lopdf::Document, page_id: ObjectId) -> Option<PageDimensions> {
    let dict = doc.get_dictionary(page_id).ok()?;
    let array = match dict.get(b"MediaBox") {
        Ok(Object::Array(arr)) => arr.clone(),
        Ok(Object::Reference(ref_id)) => match doc.objects.get(ref_id) {
            Some(Object::Array(arr)) => arr.clone(),
            _ => return None,
        },
        _ => return None,
    };

    if array.len() < 4 {
        return None;
    }

    let as_f64 = |o: &Object| match o {
        Object::Integer(n) => Some(*n as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    };

    let x0 = as_f64(&array[0])?;
    let y0 = as_f64(&array[1])?;
    let x1 = as_f64(&array[2])?;
    let y1 = as_f64(&array[3])?;

    Some(PageDimensions {
        width_pts: (x1 - x0).abs(),
        height_pts: (y1 - y0).abs(),
    })
}

// ── Core compression logic ────────────────────────────────────────────────────

/// Estimate the perceptual complexity of an image as the standard deviation of
/// pixel values across all channels.
///
/// A low stddev (< ~20) indicates a flat/uniform image such as a solid
/// background or a simple diagram — these compress well at lower quality.
/// A high stddev (> ~60) indicates a photographic or highly-detailed image
/// where quality reduction is more visible.
///
/// The value is used in [`compress_image_stream`] to adjust the JPEG quality
/// target up or down by a small amount before encoding.
fn estimate_complexity(img: &DynamicImage) -> f64 {
    let rgb = img.to_rgb8();
    let pixels: Vec<f64> = rgb
        .pixels()
        .flat_map(|p| p.0.iter().map(|&v| v as f64))
        .collect();
    if pixels.is_empty() {
        return 0.0;
    }
    let mean = pixels.iter().sum::<f64>() / pixels.len() as f64;
    let variance = pixels.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / pixels.len() as f64;
    variance.sqrt() // stddev as complexity metric
}

/// Whether the re-encoded image data is JPEG or optimized PNG (FlateDecode).
enum ImageFormat {
    Jpeg,
    Png,
}

/// Attempt to compress a single image stream.
///
/// Returns `Ok(Some(bytes_saved))` when the stream was replaced with a
/// smaller version, `Ok(None)` when skipped, and `Err` for unexpected
/// internal failures.
fn compress_image_stream(
    stream: &mut Stream,
    quality: u8,
    target_dpi: Option<u32>,
    page_dims: Option<&PageDimensions>,
    grayscale: bool,
) -> AppResult<Option<i64>> {
    let original_size = stream.content.len();

    // Skip trivially small streams.
    if original_size < MIN_IMAGE_SIZE {
        return Ok(None);
    }

    let filter = get_primary_filter(&stream.dict);
    let filter_str = filter.as_deref().unwrap_or("");

    // Dispatch based on the encoding stored in the PDF stream.
    let result = match filter_str {
        "DCTDecode" => compress_dct_stream(stream, quality, target_dpi, page_dims, grayscale),
        "FlateDecode" | "" => {
            compress_raw_stream(stream, quality, target_dpi, page_dims, grayscale)
        }
        other => {
            // JPXDecode (JPEG2000), CCITTFaxDecode, JBIG2Decode, etc.
            debug!(filter = other, "Unsupported image filter, skipping");
            return Ok(None);
        }
    };

    match result {
        Ok((new_data, format)) => {
            if new_data.len() >= original_size {
                // Re-encoded version is not smaller; keep the original.
                return Ok(None);
            }
            let saved = original_size as i64 - new_data.len() as i64;
            match format {
                ImageFormat::Jpeg => apply_jpeg_data(stream, new_data, grayscale),
                ImageFormat::Png => apply_png_data(stream, new_data, grayscale),
            }
            Ok(Some(saved))
        }
        Err(e) => {
            warn!(error = %e, "Image encode failed, leaving stream untouched");
            Ok(None)
        }
    }
}

/// Apply a complexity-based quality adjustment to the base quality value.
///
/// - stddev < 20 (flat/simple image): reduce quality by 15 to save more space.
/// - stddev > 60 (complex/photographic image): bump quality by 5, capped at 95.
/// - Otherwise: no adjustment.
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

/// Handle DCTDecode (JPEG) streams: decode → complexity analysis → optional downsample → re-encode.
fn compress_dct_stream(
    stream: &Stream,
    quality: u8,
    target_dpi: Option<u32>,
    page_dims: Option<&PageDimensions>,
    grayscale: bool,
) -> Result<(Vec<u8>, ImageFormat), String> {
    // For DCTDecode the raw content IS the JPEG bytes.
    let img = image::load_from_memory(&stream.content)
        .map_err(|e| format!("JPEG decode failed: {}", e))?;

    let (w, h) = match get_image_dimensions(&stream.dict) {
        Some(dims) => dims,
        None => (img.width(), img.height()),
    };

    // Adjust quality based on image complexity (stddev of pixel values).
    let stddev = estimate_complexity(&img);
    let adjusted_quality = adjust_quality_for_complexity(quality, stddev);
    debug!(
        stddev = stddev as u32,
        base_quality = quality,
        adjusted_quality,
        "DCT image complexity adjustment"
    );

    let img = downsample_if_needed(img, w, h, target_dpi, page_dims);
    encode_jpeg(&img, adjusted_quality, grayscale).map(|d| (d, ImageFormat::Jpeg))
}

/// Handle raw-pixel streams (FlateDecode or uncompressed image data).
///
/// Tries both JPEG and oxipng-optimized PNG encoding, keeping the smaller
/// result.  Low-complexity images (diagrams, text, line art) often compress
/// better as PNG.
fn compress_raw_stream(
    stream: &Stream,
    quality: u8,
    target_dpi: Option<u32>,
    page_dims: Option<&PageDimensions>,
    grayscale: bool,
) -> Result<(Vec<u8>, ImageFormat), String> {
    let (width, height) = match get_image_dimensions(&stream.dict) {
        Some(d) => d,
        None => return Err("Missing Width/Height in image stream dict".to_string()),
    };
    let bpc = get_bits_per_component(&stream.dict);
    let cs = get_color_space(&stream.dict);

    // Decompress FlateDecode content to raw pixels first.
    let raw_pixels = if get_primary_filter(&stream.dict).as_deref() == Some("FlateDecode") {
        zlib_decompress(&stream.content)?
    } else {
        stream.content.clone()
    };

    let img = decode_raw_pixels(&raw_pixels, width, height, &cs, bpc)?;

    // Adjust quality based on image complexity (stddev of pixel values).
    let stddev = estimate_complexity(&img);
    let adjusted_quality = adjust_quality_for_complexity(quality, stddev);
    debug!(
        stddev = stddev as u32,
        base_quality = quality,
        adjusted_quality,
        "Raw image complexity adjustment"
    );

    let img = downsample_if_needed(img, width, height, target_dpi, page_dims);

    // Try both JPEG and optimized PNG, pick the smaller result.
    let jpeg_result = encode_jpeg(&img, adjusted_quality, grayscale);
    let png_result = encode_png_optimized(&img, grayscale);

    match (jpeg_result, png_result) {
        (Ok(jpeg), Ok(png)) => {
            if png.len() < jpeg.len() {
                debug!(
                    jpeg_size = jpeg.len(),
                    png_size = png.len(),
                    "PNG wins over JPEG for this image"
                );
                Ok((png, ImageFormat::Png))
            } else {
                Ok((jpeg, ImageFormat::Jpeg))
            }
        }
        (Ok(jpeg), Err(_)) => Ok((jpeg, ImageFormat::Jpeg)),
        (Err(_), Ok(png)) => Ok((png, ImageFormat::Png)),
        (Err(e), Err(_)) => Err(e),
    }
}

// ── Image codec helpers ───────────────────────────────────────────────────────

/// Encode a `DynamicImage` as JPEG using mozjpeg for superior compression.
///
/// When `grayscale` is `true`, the image is converted to grayscale before
/// encoding (used for `CompressionLevel::Extreme`).
fn encode_jpeg(img: &DynamicImage, quality: u8, grayscale: bool) -> Result<Vec<u8>, String> {
    // JPEG does not support transparency; always convert first.
    if grayscale {
        encode_jpeg_mozjpeg_gray(img, quality)
    } else {
        encode_jpeg_mozjpeg_rgb(img, quality)
    }
}

/// Encode as progressive RGB JPEG via mozjpeg.
///
/// mozjpeg 0.10 API: `start_compress(writer)` returns `CompressStarted<W>`;
/// scanlines and finishing are called on that handle.
fn encode_jpeg_mozjpeg_rgb(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let (width, height) = (rgb.width() as usize, rgb.height() as usize);
    let pixels: &[u8] = rgb.as_raw();

    // mozjpeg's C internals use longjmp for error handling; catch_unwind keeps
    // the Rust stack safe.
    std::panic::catch_unwind(|| {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        comp.set_size(width, height);
        comp.set_quality(quality as f32);
        comp.set_progressive_mode();

        let mut started = comp
            .start_compress(Vec::new())
            .map_err(|e| format!("mozjpeg start_compress failed: {e}"))?;

        started
            .write_scanlines(pixels)
            .map_err(|e| format!("mozjpeg write_scanlines failed: {e}"))?;

        started
            .finish()
            .map_err(|e| format!("mozjpeg finish_compress failed: {e}"))
    })
    .map_err(|_| "mozjpeg: panic during RGB encoding".to_string())?
}

/// Encode as progressive grayscale JPEG via mozjpeg.
fn encode_jpeg_mozjpeg_gray(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let gray = img.to_luma8();
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    let pixels: &[u8] = gray.as_raw();

    std::panic::catch_unwind(|| {
        let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_GRAYSCALE);
        comp.set_size(width, height);
        comp.set_quality(quality as f32);
        comp.set_progressive_mode();

        let mut started = comp
            .start_compress(Vec::new())
            .map_err(|e| format!("mozjpeg start_compress (gray) failed: {e}"))?;

        started
            .write_scanlines(pixels)
            .map_err(|e| format!("mozjpeg write_scanlines (gray) failed: {e}"))?;

        started
            .finish()
            .map_err(|e| format!("mozjpeg finish_compress (gray) failed: {e}"))
    })
    .map_err(|_| "mozjpeg: panic during grayscale encoding".to_string())?
}

/// Encode as optimized PNG via oxipng, then extract the compressed data for
/// PDF FlateDecode usage.
///
/// Useful for low-complexity images (diagrams, text, line art) where lossless
/// PNG can beat lossy JPEG in size.
fn encode_png_optimized(img: &DynamicImage, grayscale: bool) -> Result<Vec<u8>, String> {
    use std::io::Cursor;

    // Encode as in-memory PNG.
    let mut png_buf: Vec<u8> = Vec::new();
    {
        let cursor = Cursor::new(&mut png_buf);
        if grayscale {
            let gray = img.to_luma8();
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                gray.as_raw(),
                gray.width(),
                gray.height(),
                image::ExtendedColorType::L8,
            )
            .map_err(|e| format!("PNG encode failed: {e}"))?;
        } else {
            let rgb = img.to_rgb8();
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| format!("PNG encode failed: {e}"))?;
        }
    }

    // Optimize with oxipng — try multiple filter & compression strategies.
    let optimized = oxipng::optimize_from_memory(&png_buf, &oxipng::Options::from_preset(4))
        .map_err(|e| format!("oxipng optimization failed: {e}"))?;

    // PDF uses FlateDecode for raw pixel data, not PNG format.  Re-decode
    // the optimized PNG back to raw pixels, then zlib-compress them.  The
    // oxipng step often reduces the pixel data variance (via better PNG
    // filter selection), which helps zlib achieve a better ratio.
    let opt_img = image::load_from_memory(&optimized)
        .map_err(|e| format!("Failed to re-decode optimized PNG: {e}"))?;

    let raw = if grayscale {
        opt_img.to_luma8().into_raw()
    } else {
        opt_img.to_rgb8().into_raw()
    };

    zlib_compress_best(&raw)
}

/// Reconstruct a `DynamicImage` from a raw (uncompressed) pixel buffer.
fn decode_raw_pixels(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &str,
    bits_per_component: u8,
) -> Result<DynamicImage, String> {
    match (color_space, bits_per_component) {
        ("DeviceRGB", 8) => image::RgbImage::from_raw(width, height, data.to_vec())
            .map(DynamicImage::ImageRgb8)
            .ok_or_else(|| {
                format!(
                    "Buffer too small for {}x{} RGB image ({} bytes)",
                    width,
                    height,
                    data.len()
                )
            }),
        ("DeviceGray", 8) => image::GrayImage::from_raw(width, height, data.to_vec())
            .map(DynamicImage::ImageLuma8)
            .ok_or_else(|| {
                format!(
                    "Buffer too small for {}x{} grayscale image ({} bytes)",
                    width,
                    height,
                    data.len()
                )
            }),
        ("DeviceCMYK", 8) => {
            // Convert CMYK → RGB (naïve inversion)
            if data.len() < (width * height * 4) as usize {
                return Err(format!(
                    "Buffer too small for {}x{} CMYK image ({} bytes)",
                    width,
                    height,
                    data.len()
                ));
            }
            let rgb_data: Vec<u8> = data
                .chunks_exact(4)
                .take((width * height) as usize)
                .flat_map(|ch| {
                    let (c, m, y, k) = (ch[0], ch[1], ch[2], ch[3]);
                    let k_f = k as f32 / 255.0;
                    let r = ((255 - c) as f32 * (1.0 - k_f)).round() as u8;
                    let g = ((255 - m) as f32 * (1.0 - k_f)).round() as u8;
                    let b = ((255 - y) as f32 * (1.0 - k_f)).round() as u8;
                    [r, g, b]
                })
                .collect();
            image::RgbImage::from_raw(width, height, rgb_data)
                .map(DynamicImage::ImageRgb8)
                .ok_or_else(|| "Failed to build RGB image from CMYK conversion".to_string())
        }
        ("ICCBased", 8) => {
            // ICCBased profiles are very common in modern PDFs. The actual
            // number of components is determined by /N in the ICC profile
            // stream, but we don't have access to the document here. Infer
            // from the raw data size instead — this is reliable because the
            // pixel buffer must exactly match width × height × components.
            let total_pixels = (width as usize) * (height as usize);
            if total_pixels == 0 {
                return Err("Zero-dimension ICCBased image".to_string());
            }
            let components = data.len() / total_pixels;
            debug!(
                width,
                height,
                data_len = data.len(),
                inferred_components = components,
                "Inferring ICCBased color space components"
            );
            match components {
                1 => decode_raw_pixels(data, width, height, "DeviceGray", 8),
                3 => decode_raw_pixels(data, width, height, "DeviceRGB", 8),
                4 => decode_raw_pixels(data, width, height, "DeviceCMYK", 8),
                _ => Err(format!(
                    "ICCBased image with unsupported component count {} ({}x{}, {} bytes)",
                    components,
                    width,
                    height,
                    data.len()
                )),
            }
        }
        _ => Err(format!(
            "Unsupported color space '{}' with {} bits per component",
            color_space, bits_per_component
        )),
    }
}

// ── DPI downsampling ──────────────────────────────────────────────────────────

/// Resize `img` proportionally when its actual DPI exceeds `target_dpi`.
///
/// When `page_dims` is available the actual DPI is computed as:
///   `actual_dpi = pixel_width / (page_width_pts / 72.0)`
///
/// For orphan images (no page context), a pixel-count threshold of 300×300 is
/// used: images smaller than that are returned unchanged; larger ones are
/// downsampled so the longest side fits within `target_dpi * (longest_side / 72)`
/// pixels.
fn downsample_if_needed(
    img: DynamicImage,
    width: u32,
    height: u32,
    target_dpi: Option<u32>,
    page_dims: Option<&PageDimensions>,
) -> DynamicImage {
    let dpi = match target_dpi {
        Some(d) => d,
        None => return img,
    };

    match page_dims {
        Some(dims) => {
            // Calculate actual DPI from the page MediaBox.
            let page_width_inch = dims.width_pts / 72.0;
            let page_height_inch = dims.height_pts / 72.0;

            // Use the dominant axis (the one that implies the higher DPI).
            let actual_dpi_x = width as f64 / page_width_inch;
            let actual_dpi_y = height as f64 / page_height_inch;
            let actual_dpi = actual_dpi_x.max(actual_dpi_y);

            if actual_dpi <= dpi as f64 {
                // Already within target; no downsampling needed.
                return img;
            }

            // Scale down so actual DPI matches target.
            let scale = dpi as f64 / actual_dpi;
            let new_w = ((width as f64 * scale).round() as u32).max(1);
            let new_h = ((height as f64 * scale).round() as u32).max(1);

            debug!(
                original_w = width,
                original_h = height,
                new_w,
                new_h,
                actual_dpi = actual_dpi as u32,
                target_dpi = dpi,
                "Downsampling image (page-aware)"
            );

            img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        }
        None => {
            // Orphan image: skip if smaller than minimum dimension.
            if width < ORPHAN_IMAGE_MIN_DIM && height < ORPHAN_IMAGE_MIN_DIM {
                return img;
            }

            // Pixel-budget heuristic: target_dpi pixels per inch on a notional
            // 1-inch square.  Scale proportionally if the image exceeds this.
            let max_pixels = (dpi as u64) * (dpi as u64) * 4; // 2-inch square budget
            let current_pixels = width as u64 * height as u64;

            if current_pixels <= max_pixels {
                return img;
            }

            let scale = (max_pixels as f64 / current_pixels as f64).sqrt();
            let new_w = ((width as f64 * scale).round() as u32).max(1);
            let new_h = ((height as f64 * scale).round() as u32).max(1);

            debug!(
                original_w = width,
                original_h = height,
                new_w,
                new_h,
                target_dpi = dpi,
                "Downsampling orphan image (pixel heuristic)"
            );

            img.resize(new_w, new_h, image::imageops::FilterType::Lanczos3)
        }
    }
}

// ── Stream mutation ───────────────────────────────────────────────────────────

/// Replace the stream's content with JPEG-encoded data and update the dict.
fn apply_jpeg_data(stream: &mut Stream, jpeg_bytes: Vec<u8>, grayscale: bool) {
    let len = jpeg_bytes.len() as i64;
    stream.content = jpeg_bytes;
    stream.dict.set("Length", Object::Integer(len));
    stream
        .dict
        .set("Filter", Object::Name(b"DCTDecode".to_vec()));
    // Remove DecodeParms — plain DCT needs none.
    stream.dict.remove(b"DecodeParms");

    // Update colour space to match the encoder output.
    let cs = if grayscale {
        b"DeviceGray".as_ref()
    } else {
        b"DeviceRGB".as_ref()
    };
    stream.dict.set("ColorSpace", Object::Name(cs.to_vec()));
}

/// Replace the stream's content with zlib-compressed raw pixel data (PNG path).
fn apply_png_data(stream: &mut Stream, zlib_bytes: Vec<u8>, grayscale: bool) {
    let len = zlib_bytes.len() as i64;
    stream.content = zlib_bytes;
    stream.dict.set("Length", Object::Integer(len));
    stream
        .dict
        .set("Filter", Object::Name(b"FlateDecode".to_vec()));
    // Remove DecodeParms if any — raw zlib without predictors.
    stream.dict.remove(b"DecodeParms");

    let cs = if grayscale {
        b"DeviceGray".as_ref()
    } else {
        b"DeviceRGB".as_ref()
    };
    stream.dict.set("ColorSpace", Object::Name(cs.to_vec()));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::Dictionary;

    fn make_image_dict(
        width: u32,
        height: u32,
        filter: Option<&str>,
        color_space: Option<&str>,
    ) -> Dictionary {
        let mut dict = Dictionary::new();
        dict.set("Subtype", Object::Name(b"Image".to_vec()));
        dict.set("Type", Object::Name(b"XObject".to_vec()));
        dict.set("Width", Object::Integer(width as i64));
        dict.set("Height", Object::Integer(height as i64));
        dict.set("BitsPerComponent", Object::Integer(8));
        if let Some(f) = filter {
            dict.set("Filter", Object::Name(f.as_bytes().to_vec()));
        }
        if let Some(cs) = color_space {
            dict.set("ColorSpace", Object::Name(cs.as_bytes().to_vec()));
        }
        dict
    }

    #[test]
    fn is_image_stream_detects_image_subtype() {
        let dict = make_image_dict(10, 10, None, None);
        assert!(is_image_stream(&dict));
    }

    #[test]
    fn is_image_stream_rejects_non_image() {
        let mut dict = Dictionary::new();
        dict.set("Subtype", Object::Name(b"Form".to_vec()));
        assert!(!is_image_stream(&dict));
    }

    #[test]
    fn get_image_dimensions_extracts_width_height() {
        let dict = make_image_dict(320, 240, None, None);
        assert_eq!(get_image_dimensions(&dict), Some((320, 240)));
    }

    #[test]
    fn decode_raw_pixels_rgb8_succeeds() {
        let w = 2u32;
        let h = 2u32;
        let pixels: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128];
        let img = decode_raw_pixels(&pixels, w, h, "DeviceRGB", 8).unwrap();
        assert_eq!(img.width(), w);
        assert_eq!(img.height(), h);
    }

    #[test]
    fn decode_raw_pixels_gray8_succeeds() {
        let w = 4u32;
        let h = 1u32;
        let pixels: Vec<u8> = vec![0, 64, 128, 255];
        let img = decode_raw_pixels(&pixels, w, h, "DeviceGray", 8).unwrap();
        assert_eq!(img.width(), w);
    }

    #[test]
    fn decode_raw_pixels_unsupported_fails() {
        let pixels = vec![0u8; 12];
        let err = decode_raw_pixels(&pixels, 2, 2, "DeviceN", 8);
        assert!(err.is_err());
    }

    #[test]
    fn downsample_skips_image_within_dpi_budget() {
        // A 100×100 image on a 72-pt (1-inch) page would be 100 DPI, which is
        // below a 150 DPI target — no downsampling expected.
        let img = DynamicImage::new_rgb8(100, 100);
        let dims = PageDimensions {
            width_pts: 72.0,
            height_pts: 72.0,
        };
        let result = downsample_if_needed(img.clone(), 100, 100, Some(150), Some(&dims));
        assert_eq!(result.width(), 100, "Should not downsample within budget");
    }

    #[test]
    fn downsample_reduces_image_exceeding_dpi() {
        // A 600×600 image on a 72-pt (1-inch) page is 600 DPI; target is 150.
        let img = DynamicImage::new_rgb8(600, 600);
        let dims = PageDimensions {
            width_pts: 72.0,
            height_pts: 72.0,
        };
        let result = downsample_if_needed(img, 600, 600, Some(150), Some(&dims));
        assert!(
            result.width() < 600,
            "Should downsample high-DPI image: got {}",
            result.width()
        );
    }

    #[test]
    fn downsample_orphan_skips_tiny_images() {
        let img = DynamicImage::new_rgb8(100, 100);
        let result = downsample_if_needed(img.clone(), 100, 100, Some(300), None);
        assert_eq!(
            result.width(),
            100,
            "Tiny orphan images must not be downsampled"
        );
    }

    #[test]
    fn encode_jpeg_rgb_produces_valid_jpeg() {
        let img = DynamicImage::new_rgb8(16, 16);
        let result = encode_jpeg(&img, 75, false).unwrap();
        assert!(!result.is_empty());
        assert_eq!(
            &result[..2],
            &[0xFF, 0xD8],
            "Must start with JPEG SOI marker"
        );
    }

    #[test]
    fn encode_jpeg_grayscale_produces_valid_jpeg() {
        let img = DynamicImage::new_rgb8(16, 16);
        let result = encode_jpeg(&img, 75, true).unwrap();
        assert!(!result.is_empty());
        assert_eq!(
            &result[..2],
            &[0xFF, 0xD8],
            "Must start with JPEG SOI marker"
        );
    }

    #[test]
    fn estimate_complexity_flat_image_has_low_stddev() {
        // All pixels identical → zero variance.
        use image::{Rgb, RgbImage};
        let mut buf = RgbImage::new(8, 8);
        for p in buf.pixels_mut() {
            *p = Rgb([128u8, 128, 128]);
        }
        let img = DynamicImage::ImageRgb8(buf);
        let stddev = estimate_complexity(&img);
        assert!(
            stddev < 1.0,
            "Flat image should have near-zero stddev, got {stddev}"
        );
    }

    #[test]
    fn estimate_complexity_noisy_image_has_high_stddev() {
        use image::{Rgb, RgbImage};
        let mut buf = RgbImage::new(8, 8);
        for (i, p) in buf.pixels_mut().enumerate() {
            let v = if i % 2 == 0 { 0u8 } else { 255u8 };
            *p = Rgb([v, v, v]);
        }
        let img = DynamicImage::ImageRgb8(buf);
        let stddev = estimate_complexity(&img);
        assert!(
            stddev > 60.0,
            "High-contrast image should have high stddev, got {stddev}"
        );
    }

    #[test]
    fn adjust_quality_reduces_for_flat_image() {
        let q = adjust_quality_for_complexity(75, 10.0);
        assert_eq!(q, 60, "Flat image (stddev<20) should reduce quality by 15");
    }

    #[test]
    fn adjust_quality_increases_for_complex_image() {
        let q = adjust_quality_for_complexity(75, 80.0);
        assert_eq!(
            q, 80,
            "Complex image (stddev>60) should increase quality by 5"
        );
    }

    #[test]
    fn adjust_quality_caps_at_95() {
        let q = adjust_quality_for_complexity(92, 80.0);
        assert_eq!(q, 95, "Quality bump must be capped at 95");
    }

    #[test]
    fn adjust_quality_unchanged_for_medium_complexity() {
        let q = adjust_quality_for_complexity(75, 40.0);
        assert_eq!(q, 75, "Mid-complexity images should keep base quality");
    }
}
