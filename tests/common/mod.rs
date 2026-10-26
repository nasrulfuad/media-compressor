#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::path::PathBuf;

use assert_cmd::Command;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
use lopdf::{dictionary, Document, Object, Stream};
use tempfile::TempDir;

pub fn cli() -> Command {
    Command::cargo_bin("media-compressor").expect("media-compressor binary not found")
}

pub fn write_temp(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write temp file");
    path
}

pub fn gradient_image(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgb8(ImageBuffer::from_fn(width, height, |x, y| {
        let value = ((x + y) % 256) as u8;
        Rgb([value, value.wrapping_add(40), value.wrapping_add(90)])
    }))
}

pub fn encode_image(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    image.write_to(&mut cursor, format).expect("encode image");
    cursor.into_inner()
}

pub fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    encode_image(&gradient_image(width, height), ImageFormat::Png)
}

pub fn jpeg_bytes(width: u32, height: u32) -> Vec<u8> {
    encode_image(&gradient_image(width, height), ImageFormat::Jpeg)
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).expect("deflate write");
    encoder.finish().expect("deflate finish")
}

fn serialize(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("serialize document");
    bytes
}

fn document_with_pages(pages: usize, content: &[u8], extra_resources: Object) -> Document {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for _ in 0..pages {
        let content_id =
            document.add_object(Stream::new(lopdf::Dictionary::new(), content.to_vec()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => extra_resources.clone(),
        });
        kids.push(Object::Reference(page_id));
    }
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => pages as i64,
            "Kids" => kids,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    document
}

pub fn minimal_pdf_bytes(pages: usize, content: &[u8]) -> Vec<u8> {
    serialize(document_with_pages(
        pages,
        content,
        Object::Dictionary(Default::default()),
    ))
}

pub fn pdf_with_jpeg_image_bytes() -> Vec<u8> {
    let mut document = document_with_pages(
        1,
        b"q 200 0 0 200 0 0 cm /Im1 Do Q",
        Object::Dictionary(Default::default()),
    );
    let jpeg = jpeg_bytes(64, 64);
    let image_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 64,
            "Height" => 64,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    ));
    let page_id = *document.get_pages().get(&1).expect("page");
    document
        .get_dictionary_mut(page_id)
        .expect("page dict")
        .set(
            "Resources",
            Object::Dictionary(dictionary! {
                "XObject" => dictionary! { "Im1" => image_id },
            }),
        );
    serialize(document)
}

pub fn pdf_with_flate_rgb_image_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut document = document_with_pages(
        1,
        b"q 200 0 0 200 0 0 cm /Im1 Do Q",
        Object::Dictionary(Default::default()),
    );
    let raw = gradient_image(width, height).to_rgb8().into_raw();
    let image_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        deflate(&raw),
    ));
    let page_id = *document.get_pages().get(&1).expect("page");
    document
        .get_dictionary_mut(page_id)
        .expect("page dict")
        .set(
            "Resources",
            Object::Dictionary(dictionary! {
                "XObject" => dictionary! { "Im1" => image_id },
            }),
        );
    serialize(document)
}
