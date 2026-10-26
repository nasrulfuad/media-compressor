use std::io::Write;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode};
use flate2::write::ZlibEncoder;
use flate2::Compression;
use lopdf::{dictionary, Document, Object, Stream};

use media_compressor::domain::level::CompressionLevel;
use media_compressor::domain::options::PdfOptions;
use media_compressor::pdf::engine::compress_pdf_bytes;

const IMAGE_SIDE: u32 = 1024;
const CONTENT_REPEATS: usize = 400;
const SAMPLE_COUNT: usize = 10;
const MEASUREMENT_SECONDS: u64 = 60;
const WARM_UP_SECONDS: u64 = 5;

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data).expect("deflate write");
    encoder.finish().expect("deflate finish")
}

fn gradient_rgb_bytes(side: u32) -> Vec<u8> {
    (0..side * side * 3)
        .map(|index| {
            let pixel = index / 3;
            ((pixel % side + pixel / side) % 256) as u8
        })
        .collect()
}

fn image_stream(side: u32) -> Stream {
    Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(side),
            "Height" => i64::from(side),
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        deflate(&gradient_rgb_bytes(side)),
    )
}

fn sample_pdf_bytes() -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let image_id = document.add_object(image_stream(IMAGE_SIDE));
    let content = b"q 400 0 0 400 50 50 cm /Im1 Do Q BT /F1 12 Tf (Sample text) Tj ET\n"
        .repeat(CONTENT_REPEATS);
    let content_id = document.add_object(Stream::new(lopdf::Dictionary::new(), content));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content_id,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image_id } },
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("serialize sample pdf");
    bytes
}

fn configure_group(group: &mut criterion::BenchmarkGroup<criterion::measurement::WallTime>) {
    group.sample_size(SAMPLE_COUNT);
    group.measurement_time(Duration::from_secs(MEASUREMENT_SECONDS));
    group.warm_up_time(Duration::from_secs(WARM_UP_SECONDS));
    group.sampling_mode(SamplingMode::Flat);
}

fn bench_compression_levels(criterion: &mut Criterion) {
    let input = sample_pdf_bytes();
    let mut group = criterion.benchmark_group("compress_levels");
    configure_group(&mut group);
    let levels = [
        CompressionLevel::Low,
        CompressionLevel::Medium,
        CompressionLevel::High,
        CompressionLevel::Extreme,
    ];
    for level in levels {
        let options = PdfOptions::for_level(level);
        group.bench_with_input(
            BenchmarkId::new(level.to_string(), format!("{}KB", input.len() / 1024)),
            &input,
            |bencher, bytes| {
                bencher.iter(|| compress_pdf_bytes(bytes, &options).expect("compress sample pdf"));
            },
        );
    }
    group.finish();
}

fn bench_streams_only(criterion: &mut Criterion) {
    let input = sample_pdf_bytes();
    let mut options = PdfOptions::for_level(CompressionLevel::Low);
    options.target_dpi = None;
    let mut group = criterion.benchmark_group("streams_only");
    configure_group(&mut group);
    group.bench_function("low_without_resize", |bencher| {
        bencher.iter(|| compress_pdf_bytes(&input, &options).expect("compress sample pdf"));
    });
    group.finish();
}

criterion_group!(benches, bench_compression_levels, bench_streams_only);
criterion_main!(benches);
