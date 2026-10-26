use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use rayon::prelude::*;
use tracing::{debug, info, warn};

use crate::detection::{detect_media_type, MAGIC_BYTES_TO_READ};
use crate::domain::options::{FailurePolicy, WriteMode};
use crate::domain::report::{BatchContext, BatchReport, FileOutcome, FileResult};
use crate::error::{AppError, AppResult};
use crate::fs::discovery::{discover_files, DiscoveryRequest};
use crate::fs::reader::read_leading_bytes;

use super::compress_file::compress_file;
use super::output_path::{batch_output_path, default_batch_output_dir};
use super::progress::BatchProgress;
use super::registry::CompressorRegistry;
use super::{BatchRequest, CompressRequest};

const SKIPPED_OUTPUT_EXISTS: &str = "output already exists";
const SKIPPED_AFTER_FAILURE: &str = "aborted after an earlier failure";

struct BatchJob<'a> {
    request: &'a BatchRequest,
    registry: &'a CompressorRegistry,
    input_dir: &'a Path,
    output_dir: &'a Path,
    stop_requested: AtomicBool,
}

fn skipped(path: &Path, reason: &str) -> FileResult {
    FileResult {
        path: path.to_path_buf(),
        outcome: FileOutcome::Skipped {
            reason: reason.to_string(),
        },
    }
}

fn failed(path: &Path, error: &AppError) -> FileResult {
    FileResult {
        path: path.to_path_buf(),
        outcome: FileOutcome::Error {
            error: error.to_string(),
        },
    }
}

fn planned_output_path(job: &BatchJob<'_>, file: &Path) -> AppResult<PathBuf> {
    let leading = read_leading_bytes(file, MAGIC_BYTES_TO_READ)?;
    let media_type = detect_media_type(&leading);
    let compressor = job.registry.find(&media_type)?;
    let extension = compressor.output_extension(file, &media_type, &job.request.settings);
    batch_output_path(file, job.input_dir, job.output_dir, Some(&extension))
}

fn process_file(job: &BatchJob<'_>, file: &Path) -> FileResult {
    if job.stop_requested.load(Ordering::SeqCst) {
        return skipped(file, SKIPPED_AFTER_FAILURE);
    }
    let output = match planned_output_path(job, file) {
        Ok(output) => output,
        Err(error) => return record_failure(job, file, &error),
    };
    if job.request.write_mode == WriteMode::Write && output.exists() {
        return skipped(file, SKIPPED_OUTPUT_EXISTS);
    }
    let request = CompressRequest {
        input: file.to_path_buf(),
        output: Some(output),
        settings: job.request.settings.clone(),
        write_mode: job.request.write_mode,
    };
    match compress_file(&request, job.registry) {
        Ok(report) => {
            debug!(file = %file.display(), ratio = report.sizes.ratio, "File compressed");
            FileResult {
                path: file.to_path_buf(),
                outcome: FileOutcome::Ok {
                    sizes: report.sizes,
                    file_type: report.file_type_label(),
                },
            }
        }
        Err(error) => record_failure(job, file, &error),
    }
}

fn record_failure(job: &BatchJob<'_>, file: &Path, error: &AppError) -> FileResult {
    warn!(file = %file.display(), error = %error, "Compression failed");
    if job.request.failure_policy == FailurePolicy::StopOnFirstError {
        job.stop_requested.store(true, Ordering::SeqCst);
    }
    failed(file, error)
}

fn run_parallel(
    job: &BatchJob<'_>,
    files: &[PathBuf],
    progress: &dyn BatchProgress,
) -> AppResult<Vec<FileResult>> {
    let process_all = || {
        files
            .par_iter()
            .map(|file| {
                let result = process_file(job, file);
                progress.file_finished();
                result
            })
            .collect::<Vec<FileResult>>()
    };
    if job.request.workers == 0 {
        return Ok(process_all());
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(job.request.workers)
        .build()
        .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
    Ok(pool.install(process_all))
}

fn validate_input_dir(input_dir: &Path) -> AppResult<()> {
    if input_dir.is_dir() {
        Ok(())
    } else {
        Err(AppError::InvalidInputDir(input_dir.to_path_buf()))
    }
}

pub fn process_batch(
    request: &BatchRequest,
    registry: &CompressorRegistry,
    progress: &dyn BatchProgress,
) -> AppResult<BatchReport> {
    let started = Instant::now();
    validate_input_dir(&request.input_dir)?;
    let output_dir = request
        .output_dir
        .clone()
        .unwrap_or_else(|| default_batch_output_dir(&request.input_dir));
    let files = discover_files(&DiscoveryRequest {
        root: &request.input_dir,
        pattern: &request.pattern,
        recursive: request.recursive,
        excluded_dir: Some(&output_dir),
    })?;
    info!(
        dir = %request.input_dir.display(),
        pattern = %request.pattern,
        files_found = files.len(),
        "Batch scan complete"
    );
    progress.start(files.len());
    let job = BatchJob {
        request,
        registry,
        input_dir: &request.input_dir,
        output_dir: &output_dir,
        stop_requested: AtomicBool::new(false),
    };
    let mut results = run_parallel(&job, &files, progress)?;
    progress.finish();
    results.sort_by(|left, right| left.path.cmp(&right.path));
    let context = BatchContext {
        input_dir: request.input_dir.clone(),
        output_dir,
        duration_ms: started.elapsed().as_millis() as u64,
        dry_run: request.write_mode == WriteMode::DryRun,
    };
    Ok(BatchReport::from_results(context, results))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use tempfile::TempDir;

    use super::*;
    use crate::app::progress::NoProgress;
    use crate::app::test_settings;
    use crate::test_support::{encode_as, gradient_rgb, PdfFixture};

    const ALL_MEDIA_PATTERN: &str = "*.{pdf,jpg,jpeg,png}";

    #[derive(Default)]
    struct CountingProgress {
        total: AtomicUsize,
        finished: AtomicUsize,
        closed: AtomicUsize,
    }

    impl BatchProgress for CountingProgress {
        fn start(&self, total_files: usize) {
            self.total.store(total_files, Ordering::SeqCst);
        }

        fn file_finished(&self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }

        fn finish(&self) {
            self.closed.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn batch_request(input_dir: &Path, output_dir: &Path) -> BatchRequest {
        BatchRequest {
            input_dir: input_dir.to_path_buf(),
            output_dir: Some(output_dir.to_path_buf()),
            pattern: ALL_MEDIA_PATTERN.to_string(),
            recursive: false,
            workers: 2,
            failure_policy: FailurePolicy::ContinueOnError,
            settings: test_settings(),
            write_mode: WriteMode::Overwrite,
        }
    }

    fn redundant_pdf_bytes() -> Vec<u8> {
        PdfFixture::new()
            .page_with_content(&b"BT   (Hello)   Tj   ET   ".repeat(60))
            .into_bytes()
    }

    fn png_bytes() -> Vec<u8> {
        encode_as(&gradient_rgb(32, 32), image::ImageFormat::Png)
    }

    fn setup() -> (TempDir, PathBuf, PathBuf) {
        let dir = TempDir::new().unwrap();
        let input = dir.path().join("in");
        std::fs::create_dir(&input).unwrap();
        let output = dir.path().join("out");
        (dir, input, output)
    }

    fn run(request: &BatchRequest) -> AppResult<BatchReport> {
        process_batch(request, &CompressorRegistry::standard(), &NoProgress)
    }

    #[test]
    fn process_batch_on_empty_directory_reports_zero_files() {
        let (_dir, input, output) = setup();
        let report = run(&batch_request(&input, &output)).unwrap();
        assert_eq!(report.total_files, 0);
        assert!(report.files.is_empty());
    }

    #[test]
    fn process_batch_compresses_pdf_and_image_with_file_type_labels() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        std::fs::write(input.join("b.png"), png_bytes()).unwrap();
        let report = run(&batch_request(&input, &output)).unwrap();
        assert_eq!(report.total_files, 2);
        assert_eq!(report.succeeded, 2);
        let labels: Vec<String> = report
            .files
            .iter()
            .map(|file| match &file.outcome {
                FileOutcome::Ok { file_type, .. } => file_type.clone(),
                other => panic!("unexpected outcome {other:?}"),
            })
            .collect();
        assert_eq!(labels, vec!["PDF", "PNG"]);
        assert!(output.join("a.pdf").exists());
        assert!(output.join("b.png").exists());
    }

    #[test]
    fn process_batch_continues_after_corrupt_file_when_policy_allows() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        std::fs::write(input.join("broken.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0]).unwrap();
        let report = run(&batch_request(&input, &output)).unwrap();
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 1);
        assert!(output.join("a.pdf").exists());
    }

    #[test]
    fn process_batch_stops_after_first_failure_when_policy_requires_it() {
        let (_dir, input, output) = setup();
        std::fs::write(
            input.join("a_broken.jpg"),
            [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0],
        )
        .unwrap();
        for name in ["b.pdf", "c.pdf", "d.pdf"] {
            std::fs::write(input.join(name), redundant_pdf_bytes()).unwrap();
        }
        let mut request = batch_request(&input, &output);
        request.failure_policy = FailurePolicy::StopOnFirstError;
        request.workers = 1;
        let report = run(&request).unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.skipped, 3);
        assert_eq!(report.succeeded, 0);
    }

    #[test]
    fn process_batch_converts_extension_when_output_format_changes() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("b.png"), png_bytes()).unwrap();
        let mut request = batch_request(&input, &output);
        request.settings.image.output_format = crate::domain::media::OutputFormat::Jpeg;
        let report = run(&request).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(output.join("b.jpg").exists());
        assert!(!output.join("b.png").exists());
    }

    #[test]
    fn process_batch_keeps_input_extension_spelling_without_conversion() {
        let (_dir, input, output) = setup();
        std::fs::write(
            input.join("photo.JPEG"),
            encode_as(&gradient_rgb(32, 32), image::ImageFormat::Jpeg),
        )
        .unwrap();
        let report = run(&batch_request(&input, &output)).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(output.join("photo.JPEG").exists());
    }

    #[test]
    fn process_batch_reports_unsupported_gif_as_failure() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("anim.png"), b"GIF89a\x01\x00\x01\x00").unwrap();
        let report = run(&batch_request(&input, &output)).unwrap();
        assert_eq!(report.failed, 1);
    }

    #[test]
    fn process_batch_skips_existing_output_without_overwrite() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("a.pdf"), b"existing").unwrap();
        let mut request = batch_request(&input, &output);
        request.write_mode = WriteMode::Write;
        let report = run(&request).unwrap();
        assert_eq!(report.skipped, 1);
        assert_eq!(std::fs::read(output.join("a.pdf")).unwrap(), b"existing");
    }

    #[test]
    fn process_batch_dry_run_writes_nothing_and_flags_report() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        let mut request = batch_request(&input, &output);
        request.write_mode = WriteMode::DryRun;
        let report = run(&request).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.succeeded, 1);
        assert!(!output.exists());
    }

    #[test]
    fn process_batch_preserves_relative_structure_when_recursive() {
        let (_dir, input, output) = setup();
        std::fs::create_dir(input.join("sub")).unwrap();
        std::fs::write(input.join("sub").join("deep.pdf"), redundant_pdf_bytes()).unwrap();
        let mut request = batch_request(&input, &output);
        request.recursive = true;
        let report = run(&request).unwrap();
        assert_eq!(report.succeeded, 1);
        assert!(output.join("sub").join("deep.pdf").exists());
    }

    #[test]
    fn process_batch_rejects_missing_input_directory() {
        let (dir, _input, output) = setup();
        let error = run(&batch_request(&dir.path().join("missing"), &output)).unwrap_err();
        assert!(matches!(error, AppError::InvalidInputDir(_)));
    }

    #[test]
    fn process_batch_rejects_invalid_glob() {
        let (_dir, input, output) = setup();
        let mut request = batch_request(&input, &output);
        request.pattern = "[".to_string();
        assert!(matches!(
            run(&request).unwrap_err(),
            AppError::InvalidGlob(_)
        ));
    }

    #[test]
    fn process_batch_reports_progress_events() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        std::fs::write(input.join("b.pdf"), redundant_pdf_bytes()).unwrap();
        let progress = CountingProgress::default();
        process_batch(
            &batch_request(&input, &output),
            &CompressorRegistry::standard(),
            &progress,
        )
        .unwrap();
        assert_eq!(progress.total.load(Ordering::SeqCst), 2);
        assert_eq!(progress.finished.load(Ordering::SeqCst), 2);
        assert_eq!(progress.closed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn process_batch_excludes_output_directory_nested_in_input() {
        let (_dir, input, _) = setup();
        let output = input.join("compressed");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("old.pdf"), redundant_pdf_bytes()).unwrap();
        std::fs::write(input.join("new.pdf"), redundant_pdf_bytes()).unwrap();
        let mut request = batch_request(&input, &output);
        request.recursive = true;
        let report = run(&request).unwrap();
        assert_eq!(report.total_files, 1);
    }

    #[test]
    fn process_batch_with_zero_workers_uses_default_pool() {
        let (_dir, input, output) = setup();
        std::fs::write(input.join("a.pdf"), redundant_pdf_bytes()).unwrap();
        let mut request = batch_request(&input, &output);
        request.workers = 0;
        assert_eq!(run(&request).unwrap().succeeded, 1);
    }

    #[test]
    fn process_batch_is_sorted_by_path() {
        let (_dir, input, output) = setup();
        for name in ["c.pdf", "a.pdf", "b.pdf"] {
            std::fs::write(input.join(name), redundant_pdf_bytes()).unwrap();
        }
        let report = run(&batch_request(&input, &output)).unwrap();
        let names: Vec<String> = report
            .files
            .iter()
            .map(|file| {
                file.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["a.pdf", "b.pdf", "c.pdf"]);
    }
}
