# Review: media-compressor

Date: 2026-10-26 · Scope: `src/`, `tests/` (working tree at review time, including uncommitted changes)

Severity: 🔴 high (bug / data corruption / security) · 🟡 medium · 🔵 low / suggestion

This review describes the code **before** the refactor tracked in `plan.md`. File locations refer to the old layout. Every finding is addressed by a phase of `plan.md` (see its traceability table).

---

## 1. Security

| # | Sev | Location | Finding | Suggestion |
|---|-----|----------|---------|------------|
| S1 | 🔴 | `compressor/pdf_utils.rs:41` `zlib_decompress` | Decompression has no size limit, so a flate bomb is possible. A PDF of a few KB can expand to GBs and cause an OOM. The same applies to `lzw_decompress` (`stream.rs:164`) and `decode_raw_pixels`. | Use `decoder.take(MAX_STREAM_BYTES).read_to_end()` and fail when the limit is hit. Keep the constant in `pdf_utils.rs`. |
| S2 | 🔴 | `Cargo.toml` `panic = "abort"` + `catch_unwind` in `image_compressor/jpeg.rs:26`, `compressor/image.rs:452` | mozjpeg reports errors through panics. With `panic = "abort"` in release builds `catch_unwind` is useless, so one corrupt image **crashes the whole process**, including a batch (violates "batch must be resilient"). | Remove `panic = "abort"` from the release profile, or isolate the encoder. Add a test with corrupt input in release mode. |
| S3 | 🟡 | `batch/processor.rs:265` `follow_links(true)` | Batch follows symlinks and can read files outside `input_dir`. `output_path_for` falls back with `unwrap_or(file)` (`:285`): when `strip_prefix` fails, `output_dir.join(<absolute source path>)` equals the source path, so the **original file may be overwritten**. | `follow_links(false)` by default (or an opt-in flag). Turn the fallback into an error and never reuse the source path. |
| S4 | 🟡 | `io/reader.rs:18`, `image_compressor/engine.rs:69` | No limit on file size or image dimensions. `std::fs::read` loads the whole file and lopdf parses everything in memory. | Add a max input size check on `metadata.len()` before loading and set explicit `image::Limits`. |
| S5 | 🔵 | `io/writer.rs:21` | Predictable temp path (`<out>.pdf.tmp`) in the same directory, so two parallel runs to the same output collide. | Use `tempfile::NamedTempFile::new_in(parent)` + `persist()` (needs `tempfile` moved to the runtime dependencies). |
| S6 | 🔵 | `io/reader.rs:20` | `path.exists()` followed by `File::open` is a small, redundant TOCTOU window. | Call `File::open` directly and map `NotFound`. |

## 2. Correctness / Data integrity (most important)

| # | Sev | Location | Finding |
|---|-----|----------|---------|
| C1 | 🔴 | `stream.rs:429` `remove_content_stream_comments` | The docstring says only `%` at line start is removed, but the **implementation removes `%` anywhere**, including inside string literals: `(Discount 50%) Tj` becomes `(Discount 50` and the rest of the line is lost, corrupting the content stream. |
| C2 | 🔴 | `stream.rs:470` `compact_whitespace` | Collapses whitespace without a tokenizer: it changes string literals such as `(A   B)` and **corrupts inline image binary data (BI…ID…EI)**. It also treats `0x00` as whitespace. |
| C3 | 🔴 | `stream.rs:399` `is_content_stream` | The heuristic "no /Type and no /Subtype" also matches ICC profiles, sampled functions (FunctionType 0), etc., so binary streams get compacted and corrupted. Safer: collect IDs from each page `/Contents` (plus Form XObjects) explicitly. |
| C4 | 🔴 | `stream.rs:119-131` `process_stream` | Decodes only the **first filter**, then sets `/Filter /FlateDecode` and drops `/DecodeParms`. A chain `[/FlateDecode /X]` loses its second filter and Flate with `/Predictor` loses the predictor, corrupting data. It should skip when the filter array has more than one entry or DecodeParms contains a Predictor. |
| C5 | 🟡 | `stream.rs:240` LZW | The code width grows when `dict.len() >= 1<<w`, which is *late change* (GIF). PDF defaults to `EarlyChange 1`, which needs the width to grow one code earlier, so real LZW streams may decode wrongly. No LZW fixture exists to verify. |
| C6 | 🔴 | `cleanup.rs:116` `remove_non_link_annotations` | Runs at **every level including Low** and removes widgets (form fields), highlights, comments and signatures. AcroForm stays, so `/Fields` points at deleted objects (dangling). Does not match the level table in CLAUDE.md. |
| C7 | 🟡 | `cleanup.rs:41,23` | `remove_named_destinations` and `remove_output_intents` also run at every level, breaking internal links and outlines that use named destinations and dropping PDF/A compliance. Objects are removed directly with `doc.objects.remove` (they may be shared); removing the references and letting `prune_objects` clean up is safer. |
| C8 | 🟡 | `engine.rs:153` | No file-level guard that the output is not larger than the input (CLAUDE.md rule). A larger PDF can still be written. |
| C9 | 🟡 | `image_compressor/png.rs:31`, `compressor/image.rs:520` | `to_rgb8()` **drops the alpha channel** (transparent PNGs get a black background) and reduces 16-bit to 8-bit, with no warning. |
| C10 | 🟡 | `main.rs:40` + `format.rs:35` | GIF/TIFF/BMP with `--format original` are encoded as JPEG but the output path keeps `.gif/.tif/.bmp`, so the extension does not match the content. Batch behaves the same (`output_path_for` keeps the extension). |
| C11 | 🟡 | `batch/processor.rs:418` | `--continue-on-error` is **ignored** (only a warning is logged). Violates CLAUDE.md. Use `try_for_each` or an `AtomicBool` stop flag. |
| C12 | 🟡 | `image_compressor/engine.rs:167` | Image output uses `std::fs::write` (not atomic), violating the atomic-write rule; when output equals input and the process crashes, the original is lost. |
| C13 | 🔵 | `image_compressor/engine.rs:115` | A comment says `--keep-metadata` emits a warning, but the code is empty. `DispatchConfig.keep_metadata` and `strip_metadata` are unused for images. |
| C14 | 🔵 | `engine.rs:156` dry-run | The estimate ignores inline images and metadata; `image_bytes_saved` can be negative and mixes u64 and i64. |

## 3. Clean Code

| # | Sev | Location | Finding / Suggestion |
|---|-----|----------|----------------------|
| CC1 | 🟡 | `compressor/image.rs:432-556` vs `image_compressor/jpeg.rs`, `png.rs` | The mozjpeg RGB/gray encoders and the PNG encoder are **duplicated** in two modules (nearly identical). Merge them into one encoder module used by both. |
| CC2 | 🟡 | `strategy/resolver.rs:17` vs `image_compressor/format.rs:7` | Magic-byte detection is duplicated. `ImageCompressorEngine` re-reads the file and re-detects after the dispatcher already did. Pass the `ImageFormat` from the dispatcher. |
| CC3 | 🟡 | `compressor/inline.rs:493-505` | Local `decompress_content` and `compress_zlib` violate the "`pdf_utils.rs` is the single source" rule. `compress_zlib` silently falls back to raw data without updating `/Filter`. |
| CC4 | 🟡 | All of `compressor/*` | Errors typed as `Result<_, String>` and `AppError::Custom(String)` are used widely, losing error types. Add specific variants (`Decompress`, `Encode`, `Glob`) to `AppError`. |
| CC5 | 🟡 | `main.rs` | `std::process::exit(2)` in the middle of `main` (skips destructors and flushing), `Info` error mapping duplicates the `Display` of `AppError`, and `Info` pretty-printing lives in `main`. Move it to `stats/report.rs` and return errors that carry an exit code. |
| CC6 | 🟡 | `compressor/image.rs:520-552` | Encode PNG, run oxipng, decode again, then raw zlib. The oxipng result is wasted (PNG filtering is not carried over). Either use `/DecodeParms /Predictor 15` with the IDAT data, or deflate directly. |
| CC7 | 🔵 | `strategy/dispatcher.rs:106-127` | Engines are configured by mutating `pub` fields. Cleaner: a constructor from config (`CompressorEngine::from(&DispatchConfig)`) or a builder. |
| CC8 | 🔵 | `main.rs:66` + `image_compressor/engine.rs:54` | Grayscale from the level is computed twice (in `main` and in `ImageCompressorEngine::new`). |
| CC9 | 🔵 | `batch/processor.rs:327` | `rayon::build_global()` inside a library function is a global side effect; a second call with a different `workers` value is silently ignored. Use a local `ThreadPool::install`. |
| CC10 | 🔵 | `batch/processor.rs:347,..` | `Mutex::lock().unwrap()` and `ProgressStyle::...unwrap()` in non-test code. |
| CC11 | 🔵 | `image_compressor/metadata.rs` | `metadata_is_stripped_by_reencoding()` always returns `true` and is unused: dead code. |
| CC12 | 🔵 | `engine.rs` 13 inline steps | `compress()` is about 150 lines and each step repeats the same call-plus-debug pattern. It could be a list of passes. |

## 4. Clean Architecture

| # | Sev | Finding | Suggestion |
|---|-----|---------|------------|
| A1 | 🟡 | The `CompressorStrategy` trait (`strategy/mod.rs:18`) is **not used** by the dispatcher, which hardcodes a `match` over engine types. `type Config = ()` and `can_handle` are unused. A half-finished abstraction. | Either (a) delete the trait and use plain enum dispatch, or (b) iterate `Vec<Box<dyn Strategy>>` and use `can_handle`. |
| A2 | 🟡 | Inverted dependencies: `compressor/engine.rs` and `image_compressor/mod.rs` import `strategy::*` (a higher layer) while `strategy::dispatcher` imports both engines, forming a module cycle. `ImageFormat` / `ImageOutputFormat` (domain) live in `strategy`, and `ImageOutputFormat` derives `clap::ValueEnum` (domain depends on the CLI). | Move domain types to a neutral `domain`/`media` module and keep the clap mapping in `cli/args.rs`. |
| A3 | 🟡 | The batch report (`BatchReport`, `FileResult`, `print_pretty`) lives in `batch/processor.rs`, although CLAUDE.md places it in `stats/report.rs`. Presentation logic (colored println) is mixed with the domain report. | Move it to `stats/` and separate formatters (pretty/json) from data. |
| A4 | 🟡 | Batch builds a hardcoded `DispatchConfig` (`processor.rs:358`), so image options (`--quality`, `--format`, `--max-dimension`) are unavailable in batch; it also duplicates `DispatchConfig::default()`. | `BatchConfig` should hold a `DispatchConfig`. |
| A5 | 🔵 | PDF pipeline: the engine owns I/O (`load_pdf`, `save_pdf`, `fs::metadata`) as well as the transformation. Hard to unit-test without files. | Split a pure `compress_document(&mut Document) -> Stats` from the I/O wrapper. |
| A6 | 🔵 | `CLAUDE.md` does not describe the new `strategy/` and `image_compressor/` modules, and the level table does not mention the cleanup that always runs. | Update the documentation. |

## 5. Unit Tests

| # | Sev | Finding | Suggestion |
|---|-----|---------|------------|
| T1 | 🔴 | There is **not a single** end-to-end happy-path PDF compression test (unit or integration). `inputs/sample.pdf` was removed from the repo. PDF integration tests only cover `--help`, error paths and invalid level (happy paths exist only for JPEG/PNG). | Add a small PDF fixture under `tests/fixtures/` (text, JPEG image, Flate image, form, link). Assert the output reloads in lopdf, the page count is unchanged and the size is not larger. |
| T2 | 🔴 | Bugs C1–C4 slipped through because no test covers string literals with `%`, whitespace in strings, binary inline images, filter chains or Predictor. | One regression test per case in `stream.rs`. |
| T3 | 🟡 | `compressor/image.rs` (975 lines) has only **2 tests**; `inline.rs` (559) has 4; `font.rs` (887) has 5; `cleanup.rs` has 4. Complex functions (`decode_raw_pixels`, `downsample_if_needed`, `subset_fonts`, `parse_inline_image`) are practically untested. | Prioritise unit tests for pure parsing/decoding functions and round-trip property tests (decode∘encode). |
| T4 | 🟡 | Batch: only an empty directory is tested. No tests for success, skip (output exists), per-file errors, `continue_on_error`, recursive output structure or symlinks. | Add tests with mixed valid and invalid files. |
| T5 | 🟡 | No test for the LZW decoder (`lzw_decompress`) with real vectors and none for a decompression bomb or limit. | Add a known vector from the PDF specification. |
| T6 | 🔵 | Many tests are only `*_does_not_panic` or `assert!(result.is_ok())` (report.rs, png effort), which are weak assertions. | Assert on output content (JSON snapshot, specific fields). |
| T7 | 🔵 | Image tests use plain black `new_rgb8` images, so alpha, 16-bit, grayscale input and the "not smaller, keep original" branch are not exercised. | Add images with alpha and gradients; test the keep-original branch. |
| T8 | 🔵 | No CLI integration tests for `batch` with real files, the `info` happy path, the `--overwrite` guard, or that PDF `--dry-run` writes no file. | Add them with `assert_cmd`. |

## 6. `cargo test` / `clippy` / `fmt` results (at review time)

- `cargo test --all-features` ✅ all passed (unit tests plus 24 integration tests).
- `cargo clippy -- -D warnings` ✅ no warnings.
- `cargo fmt --check` ❌ **failed**: diff in `src/compressor/font.rs:627` onward (`extract_char_codes(...)` multi-line). Run `make fmt`. This means `make check` did not pass at that time.

Note: tests were green while bugs C1–C4 and C6 were present, so coverage did not touch the risky paths.

## 7. Recommended fix priority

1. **C1–C4** (content stream corruption): disable `compact_whitespace` and comment stripping until a tokenizer exists, skip filter chains and predictors. Add regression tests (T2).
2. **C6/C7**: gate annotation, named destination and output intent removal by level; do not remove widgets while keeping AcroForm.
3. **S1/S2**: limit decompression; remove `panic = "abort"` so `catch_unwind` works.
4. **C11, C12, C8**: follow the CLAUDE.md rules (continue-on-error, atomic image writes, never bigger).
5. **T1**: PDF fixture plus end-to-end tests, then continue with the architecture refactor (A1–A4) and code deduplication (CC1–CC3).
