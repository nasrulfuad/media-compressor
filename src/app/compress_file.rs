use std::path::{Path, PathBuf};
use std::time::Instant;

use tracing::info;

use crate::detection::detect_media_type;
use crate::domain::media::MediaType;
use crate::domain::options::WriteMode;
use crate::domain::report::{FileReport, SizeStats};
use crate::error::{AppError, AppResult};
use crate::fs::atomic_writer::write_atomically;
use crate::fs::reader::read_bounded;

use super::media_compressor::{CompressedMedia, MediaCompressor};
use super::output_path::default_output_path;
use super::registry::CompressorRegistry;
use super::CompressRequest;

fn resolve_output_path(
    request: &CompressRequest,
    compressor: &dyn MediaCompressor,
    media_type: &MediaType,
) -> PathBuf {
    request.output.clone().unwrap_or_else(|| {
        let extension = compressor.output_extension(&request.input, media_type, &request.settings);
        default_output_path(&request.input, &extension)
    })
}

fn ensure_output_writable(output: &Path, write_mode: WriteMode) -> AppResult<()> {
    if write_mode == WriteMode::Write && output.exists() {
        return Err(AppError::OutputExists(output.to_path_buf()));
    }
    Ok(())
}

fn write_unless_dry_run(output: &Path, bytes: &[u8], write_mode: WriteMode) -> AppResult<()> {
    if write_mode == WriteMode::DryRun {
        return Ok(());
    }
    write_atomically(output, bytes)
}

struct CompletedRun {
    output_path: PathBuf,
    input_size: usize,
    compressed: CompressedMedia,
    started: Instant,
}

fn build_report(request: &CompressRequest, run: CompletedRun) -> FileReport {
    FileReport {
        input_path: request.input.clone(),
        output_path: run.output_path,
        sizes: SizeStats::new(run.input_size as u64, run.compressed.bytes.len() as u64),
        kept_original: run.compressed.kept_original,
        duration_ms: run.started.elapsed().as_millis() as u64,
        dry_run: request.write_mode == WriteMode::DryRun,
        details: run.compressed.details,
    }
}

pub fn compress_file(
    request: &CompressRequest,
    registry: &CompressorRegistry,
) -> AppResult<FileReport> {
    let started = Instant::now();
    let input = read_bounded(&request.input, request.settings.pdf.limits.max_input_bytes)?;
    let media_type = detect_media_type(&input);
    let compressor = registry.find(&media_type)?;
    let output_path = resolve_output_path(request, compressor, &media_type);
    ensure_output_writable(&output_path, request.write_mode)?;
    info!(
        input = %request.input.display(),
        output = %output_path.display(),
        compressor = compressor.name(),
        "Starting compression"
    );
    let compressed = compressor.compress(&input, &media_type, &request.settings)?;
    write_unless_dry_run(&output_path, &compressed.bytes, request.write_mode)?;
    Ok(build_report(
        request,
        CompletedRun {
            output_path,
            input_size: input.len(),
            compressed,
            started,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::app::media_compressor::CompressionSettings;
    use crate::app::test_settings;
    use crate::domain::report::MediaDetails;
    use crate::test_support::{encode_as, gradient_rgb, PdfFixture};

    struct RecordingCompressor {
        calls: Arc<AtomicUsize>,
    }

    impl MediaCompressor for RecordingCompressor {
        fn name(&self) -> &'static str {
            "Recording"
        }

        fn supports(&self, _media_type: &MediaType) -> bool {
            true
        }

        fn output_extension(&self, _: &Path, _: &MediaType, _: &CompressionSettings) -> String {
            "rec".to_string()
        }

        fn compress(
            &self,
            _input: &[u8],
            _: &MediaType,
            _: &CompressionSettings,
        ) -> AppResult<CompressedMedia> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CompressedMedia {
                bytes: b"fixed".to_vec(),
                kept_original: false,
                details: MediaDetails::Pdf { passes: Vec::new() },
            })
        }
    }

    fn request(input: &Path, write_mode: WriteMode) -> CompressRequest {
        CompressRequest {
            input: input.to_path_buf(),
            output: None,
            settings: test_settings(),
            write_mode,
        }
    }

    fn redundant_pdf_bytes() -> Vec<u8> {
        PdfFixture::new()
            .page_with_content(&b"BT   (Hello)   Tj   ET   ".repeat(60))
            .into_bytes()
    }

    #[test]
    fn compress_file_uses_strategy_from_registry() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("anything.bin");
        std::fs::write(&input, b"whatever bytes").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = CompressorRegistry::new(vec![Box::new(RecordingCompressor {
            calls: calls.clone(),
        })]);
        let report = compress_file(&request(&input, WriteMode::Write), &registry).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            report.output_path,
            dir.path().join("anything_compressed.rec")
        );
        assert_eq!(std::fs::read(&report.output_path).unwrap(), b"fixed");
        assert_eq!(report.sizes.original_size, 14);
        assert_eq!(report.sizes.compressed_size, 5);
    }

    #[test]
    fn compress_file_writes_pdf_next_to_input_with_default_name() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        let report = compress_file(
            &request(&input, WriteMode::Write),
            &CompressorRegistry::standard(),
        )
        .unwrap();
        assert_eq!(report.output_path, dir.path().join("doc_compressed.pdf"));
        assert!(report.sizes.compressed_size < report.sizes.original_size);
        assert_eq!(
            std::fs::metadata(&report.output_path).unwrap().len(),
            report.sizes.compressed_size
        );
        assert!(!report.dry_run);
    }

    #[test]
    fn dry_run_reports_same_size_as_real_run() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        let registry = CompressorRegistry::standard();
        let dry = compress_file(&request(&input, WriteMode::DryRun), &registry).unwrap();
        let real = compress_file(&request(&input, WriteMode::Write), &registry).unwrap();
        assert_eq!(dry.sizes.compressed_size, real.sizes.compressed_size);
        assert!(dry.dry_run);
    }

    #[test]
    fn dry_run_writes_no_file() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        let report = compress_file(
            &request(&input, WriteMode::DryRun),
            &CompressorRegistry::standard(),
        )
        .unwrap();
        assert!(!report.output_path.exists());
    }

    #[test]
    fn compress_file_refuses_to_replace_existing_output_without_overwrite() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        std::fs::write(dir.path().join("doc_compressed.pdf"), b"keep me").unwrap();
        let error = compress_file(
            &request(&input, WriteMode::Write),
            &CompressorRegistry::standard(),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::OutputExists(_)));
        assert_eq!(
            std::fs::read(dir.path().join("doc_compressed.pdf")).unwrap(),
            b"keep me"
        );
    }

    #[test]
    fn compress_file_overwrites_existing_output_when_requested() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        std::fs::write(dir.path().join("doc_compressed.pdf"), b"old").unwrap();
        let report = compress_file(
            &request(&input, WriteMode::Overwrite),
            &CompressorRegistry::standard(),
        )
        .unwrap();
        assert!(std::fs::metadata(&report.output_path).unwrap().len() > 3);
    }

    #[test]
    fn compress_file_honors_explicit_output_path() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("photo.png");
        std::fs::write(
            &input,
            encode_as(&gradient_rgb(32, 32), image::ImageFormat::Png),
        )
        .unwrap();
        let mut explicit = request(&input, WriteMode::Write);
        explicit.output = Some(dir.path().join("nested").join("out.png"));
        let report = compress_file(&explicit, &CompressorRegistry::standard()).unwrap();
        assert_eq!(
            report.output_path,
            dir.path().join("nested").join("out.png")
        );
        assert!(report.output_path.exists());
        assert!(matches!(report.details, MediaDetails::Image { .. }));
    }

    #[test]
    fn compress_file_rejects_unknown_file_type_before_writing() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("data.bin");
        std::fs::write(&input, [0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        let error = compress_file(
            &request(&input, WriteMode::Write),
            &CompressorRegistry::standard(),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::UnsupportedFormat(_)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn compress_file_reports_missing_input() {
        let error = compress_file(
            &request(Path::new("/nonexistent/in.pdf"), WriteMode::Write),
            &CompressorRegistry::standard(),
        )
        .unwrap_err();
        assert!(matches!(error, AppError::InputNotFound(_)));
    }

    #[test]
    fn compress_file_enforces_max_input_size() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("doc.pdf");
        std::fs::write(&input, redundant_pdf_bytes()).unwrap();
        let mut limited = request(&input, WriteMode::Write);
        limited.settings.pdf.limits.max_input_bytes = 10;
        let error = compress_file(&limited, &CompressorRegistry::standard()).unwrap_err();
        assert!(matches!(error, AppError::InputTooLarge { limit: 10, .. }));
    }

    #[test]
    fn compress_file_rejects_undecodable_image_formats() {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("anim.gif");
        std::fs::write(&input, b"GIF89a\x01\x00\x01\x00").unwrap();
        let error = compress_file(
            &request(&input, WriteMode::Write),
            &CompressorRegistry::standard(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("GIF input is not supported"));
    }
}
