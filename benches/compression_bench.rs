//! Compression benchmarks using criterion.
//!
//! Requires `inputs/sample.pdf` in the project root.  If absent, benchmarks are
//! skipped with a warning rather than failing the build.
//!
//! Run with: `cargo bench`
//! HTML report: `target/criterion/report/index.html`

use std::path::PathBuf;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode};
use tempfile::TempDir;

use media_compressor::compressor::engine::CompressorEngine;
use media_compressor::compressor::level::CompressionLevel;

/// Locate the sample PDF used for benchmarks.
fn sample_pdf() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("inputs/sample.pdf"),
        PathBuf::from("tests/fixtures/with_images.pdf"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Configure a benchmark group for PDF compression workloads.
///
/// PDF compression is CPU-intensive (seconds per iteration), so we use:
/// - `sample_size(10)` — fewer iterations to keep total time reasonable
/// - `measurement_time(60s)` — enough wall time for 10 samples of multi-second ops
/// - `warm_up_time(5s)` — warm up caches and JIT before measuring
/// - `SamplingMode::Flat` — each iteration measured independently
fn configure_group(group: &mut criterion::BenchmarkGroup<criterion::measurement::WallTime>) {
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));
    group.warm_up_time(Duration::from_secs(5));
    group.sampling_mode(SamplingMode::Flat);
}

/// Benchmark compression across all four levels using a real PDF.
fn bench_compression_levels(c: &mut Criterion) {
    let input = match sample_pdf() {
        Some(p) => p,
        None => {
            eprintln!(
                "WARNING: No sample PDF found (inputs/sample.pdf or tests/fixtures/with_images.pdf). \
                 Skipping compression benchmarks."
            );
            return;
        }
    };

    let file_size = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);
    let mut group = c.benchmark_group("compress_levels");
    configure_group(&mut group);

    let levels = [
        CompressionLevel::Low,
        CompressionLevel::Medium,
        CompressionLevel::High,
        CompressionLevel::Extreme,
    ];

    for level in &levels {
        group.bench_with_input(
            BenchmarkId::new(level.to_string(), format!("{:.0}KB", file_size / 1024)),
            &input,
            |b, input_path| {
                b.iter(|| {
                    let tmp = TempDir::new().unwrap();
                    let output = tmp.path().join("out.pdf");
                    let engine = CompressorEngine::new(*level);
                    engine.compress(input_path, &output).unwrap();
                });
            },
        );
    }

    group.finish();
}

/// Benchmark dry-run mode (no I/O write) to measure pure processing overhead.
fn bench_dry_run(c: &mut Criterion) {
    let input = match sample_pdf() {
        Some(p) => p,
        None => return,
    };

    let mut group = c.benchmark_group("dry_run");
    configure_group(&mut group);

    group.bench_function("medium_dry_run", |b| {
        b.iter(|| {
            let tmp = TempDir::new().unwrap();
            let output = tmp.path().join("out.pdf");
            let mut engine = CompressorEngine::new(CompressionLevel::Medium);
            engine.dry_run = true;
            engine.compress(&input, &output).unwrap();
        });
    });

    group.finish();
}

/// Benchmark individual phases by comparing with/without image compression.
fn bench_stream_only(c: &mut Criterion) {
    let input = match sample_pdf() {
        Some(p) => p,
        None => return,
    };

    let mut group = c.benchmark_group("stream_only");
    configure_group(&mut group);

    // Low level with no DPI target = minimal image work, mostly stream compression.
    group.bench_function("low_no_image_resize", |b| {
        b.iter(|| {
            let tmp = TempDir::new().unwrap();
            let output = tmp.path().join("out.pdf");
            let engine = CompressorEngine::new(CompressionLevel::Low);
            engine.compress(&input, &output).unwrap();
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_compression_levels,
    bench_dry_run,
    bench_stream_only
);
criterion_main!(benches);
