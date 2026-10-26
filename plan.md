# Refactor Plan: media-compressor

Goal: apply clean architecture, full unit test coverage, no comments in Rust source, clean code, English only, and fix every finding in `summary.md` plus the new findings in section 2.

This plan is written for an executor model. Follow it literally. When something in the codebase contradicts the plan, or a step cannot be done as written, **stop and ask the user**. Do not improvise a different design.

Status: **P0–P6 implemented** on branch `refactor/clean-architecture`. P4 (identity CID font subsetting with usage walker, CIDToGIDMap rebuild, subset tags), P5 (bounded decode, `--max-input-size`, symlink-safe discovery, atomic writes, batch failure tests) and P6 (pure engines, batch CLI parity, extension from resolved format, NoProgress for `--json`/`--quiet`, PNG predictor 15 for Flate images, `anyhow` removed) are done. CLI help already lives in `#[arg(help = ...)]` attributes (part of P7.1). Open: P7 (comments left in `tests/integration_test.rs`, `summary.md` translation, readability sweep, `no-comments` in `make check`) and P8 (integration test split, coverage gate, `CLAUDE.md`/README updates). The D9 font test is `#[ignore]` until `tests/fixtures/fonts/test-font.ttf` exists; it passed locally with a temporary system font.

---

## 0. Execution Protocol

### 0.1 Before the first task

1. Confirm with the user that every decision in section 1 is approved. Do not start before that.
2. Commit the current working tree as-is on the current branch (`docs/claude-md-rules`):
   ```bash
   git add -A
   git commit -m "chore: snapshot image compression work before refactor"
   ```
3. Create the work branch:
   ```bash
   git checkout -b refactor/clean-architecture
   ```
4. Install the coverage tool once (dev tool, not a crate dependency):
   ```bash
   cargo install cargo-llvm-cov --locked
   rustup component add llvm-tools-preview
   ```

### 0.2 Per task loop

For every task `Px.y`:
1. Read every file listed under **Files** before editing it.
2. Write the listed tests first. Run them and confirm the new ones fail for the documented reason (for bug fixes) or do not compile yet (for new modules).
3. Implement the change.
4. Run `make check`. Every task must end green. Until P7.4 wires them in, `make check` = test + lint + fmt-check.
5. Commit with the message given in the task. End every commit message with the attribution trailer your harness requires.

### 0.3 Global rules for all code written in this plan

- **No comments in `.rs` files**: no `//`, `/* */`, `///`, `//!`. Names, small functions and constants carry the meaning. Old files may still have comments until P7; never add new ones.
- **English only** for identifiers, strings, log messages, test names, docs, commit messages.
- No new crates. The only dependency change allowed is D2 (moving `tempfile`).
- No `unwrap()`, `expect()`, `panic!()` outside `#[cfg(test)]` code.
- Functions are at most ~40 lines and take at most 4 parameters. Group more parameters into a struct.
- Never use two `bool` parameters on one function. Use enums (`ColorMode`, `ScanMode`, `WriteMode`, `FailurePolicy`).
- Logging uses `tracing` with structured fields: `warn!(object_id = ?id, error = %e, "message")`.
- Test names describe behavior: `compress_streams_keeps_percent_inside_string_literal`.
- Tests assert concrete values. `is_ok()` alone, or "does not panic", is not an assertion.
- Comment-free code style examples:
  ```rust
  const PDF_POINTS_PER_INCH: f64 = 72.0;

  fn effective_dpi(pixel_width: u32, page_width_points: f64) -> f64 {
      f64::from(pixel_width) / (page_width_points / PDF_POINTS_PER_INCH)
  }
  ```

### 0.4 Commands

| Purpose | Command |
|---------|---------|
| All checks | `make check` |
| One test | `cargo test --lib pdf::content_lexer::tests::tokenize_keeps_percent_inside_literal_string` |
| Integration tests only | `cargo test --test integration_test` |
| Coverage | `make coverage` (added in P0.5) |
| No-comment gate | `make no-comments` (added in P0.5) |

---

## 1. Decisions (must be approved before starting)

| # | Decision | Default |
|---|----------|---------|
| D1 | Remove `panic = "abort"` from `[profile.release]` so `catch_unwind` around mozjpeg works (S2). | Yes |
| D2 | Move `tempfile = "3.10"` from `[dev-dependencies]` to `[dependencies]` for atomic writes (S5). | Yes |
| D3 | Adopt the per-level cleanup table in P3.1 (C6, C7). Low/Medium stop deleting annotations, forms and named destinations. | Yes |
| D4 | Remove the `--strip-metadata` CLI flag. It does nothing for images; PDFs follow `--level` / `--keep-metadata`. | Yes |
| D5 | Coverage gate: `cargo llvm-cov --lib --fail-under-lines 90`. | Yes |
| D6 | Snapshot commit + new branch as in 0.1. | Yes |
| D7 | All `///` docs are deleted. `cargo doc` output becomes empty. CLI help moves into `#[arg(help = ...)]` attributes (see N15). | Yes |
| D8 | Only JPEG and PNG can be decoded (the `image` crate is built with `jpeg` and `png` features only). GIF/TIFF/BMP/WebP inputs are rejected early with `UnsupportedFormat`, and the batch default pattern becomes `*.{pdf,jpg,jpeg,png}` (case-insensitive). No new image features. | Yes |
| D9 | Font subsetting end-to-end tests need a real TrueType file. The user provides an OFL-licensed `.ttf` under 200 KB at `tests/fixtures/fonts/test-font.ttf`. If absent, those tests are marked `#[ignore = "requires tests/fixtures/fonts/test-font.ttf"]`. | User provides file |
| D10 | Remove `webp` from `--format`. It only ever returned an error. | Yes |
| D11 | Move inline-image conversion to run right after cleanup (before dedup and the image pass). Today converted images are never compressed (N10). CLAUDE.md pipeline section is updated in P8. | Yes |
| D12 | Media types are separated with the **Strategy + Registry** pattern (section 3.4): one `MediaCompressor` strategy per media type (PDF, image), selected from a `CompressorRegistry` by the detected `MediaType`. This replaces the unused `CompressorStrategy` trait and the hardcoded `match` in the dispatcher (A1). | Yes |

---

## 2. New Findings (found while planning; not in `summary.md`)

| # | Sev | Location | Finding |
|---|-----|----------|---------|
| N1 | 🟡 | `strategy/dispatcher.rs:105` | `--quality` help says it applies to PDF images, but it is never passed to the PDF engine. |
| N2 | 🟡 | `strategy/dispatcher.rs:105` | `--lossless` help says "for PDF: skip lossy image recompression", but the PDF engine never receives it. |
| N3 | 🔵 | `strategy/dispatcher.rs:105` | `--grayscale` is not passed to the PDF engine; only `--level extreme` turns PDF images gray. |
| N4 | 🟡 | `Cargo.toml` image features | GIF/TIFF/BMP/WebP are detected but cannot be decoded (only `jpeg`,`png` features). Every such file fails; the batch default pattern still includes them. |
| N5 | 🔴 | `compressor/image.rs:383` | Flate images with `/DecodeParms /Predictor > 1` are inflated and treated as raw pixels. The predictor bytes are read as pixels, so the re-encoded image is garbage and replaces the original. |
| N6 | 🔴 | `compressor/dedup.rs:37` | Dedup key is stream content only. Two streams with identical bytes but different dictionaries (`/Width`, `/ColorSpace`, `/Filter`, `/DecodeParms`, `/SMask`) are merged, so one of them renders wrong. |
| N7 | 🔴 | `compressor/inline.rs:333-339` | `unsafe` raw pointer to the page dictionary while `doc.objects` is read again: aliasing a `&mut` is undefined behavior. |
| N8 | 🔴 | `compressor/inline.rs:111,493` | Content streams with filters other than Flate are scanned as encoded bytes; failed inflate falls back to raw bytes; the stream is then rewritten with `/Filter /FlateDecode`, losing the original filter. |
| N9 | 🔴 | `compressor/inline.rs:326-386` | Creating `/Resources` on a page hides resources inherited from `/Pages` (fonts disappear). When `/Resources` is an indirect reference holding a direct `/XObject` dictionary, new XObjects are never registered, so the inserted `Do` references a missing XObject. |
| N10 | 🟡 | `compressor/engine.rs:146` | Inline-image conversion runs after the image pass (new XObjects are never compressed) and after dedup (a content stream shared by two pages is rewritten for one page only). |
| N11 | 🔴 | `compressor/font.rs:335-368` | Subsetting reads the stored (usually Flate-compressed) font bytes, writes the raw subset under the old `/Filter`, and removes `/Length1`, which `FontFile2` requires. |
| N12 | 🔴 | `compressor/font.rs:343` | `GlyphRemapper` renumbers glyphs, but neither content streams nor `/CIDToGIDMap` are updated, so text renders with wrong glyphs. Simple TrueType fonts map char codes through `cmap`, not 1:1 to glyph IDs. |
| N13 | 🔴 | `compressor/font.rs:465` | Character collection uses `lopdf::Content::decode`, whose parser silently stops at the first `%` comment or unknown token (verified in lopdf 0.34 `parser.rs:304`, no end-of-input check). Glyphs used later in the stream, in annotation appearances or in AcroForm `/DR` fonts are dropped from the subset. |
| N14 | 🟡 | `compressor/metadata.rs:31` | XMP streams are deleted while the catalog `/Metadata` still references them (dangling reference). |
| N15 | 🟡 | `cli/args.rs` | All CLI help text comes from `///` doc comments. Deleting comments (rule) deletes `--help` output unless the text moves to `#[command(about = ...)]` / `#[arg(help = ...)]`. |
| N16 | 🔵 | `benches/compression_bench.rs:21` | Benchmark looks for `inputs/sample.pdf`, which was moved to `samples/`. |

Consequence of N13 for the plan: **lopdf's content parser must not be used anywhere**. This plan adds an in-house content-stream lexer (P2.1) used by stream minification, inline-image conversion and font character collection.

---

## 3. Target Architecture

### 3.1 Dependency rule

Arrows mean "may import from". Nothing imports in the opposite direction.

```
main.rs ─► cli ─► app ─► { pdf, raster, fs, detection } ─► codec ─► domain
                   │                                        ▲
                   └─► (progress trait only)                │
main.rs ─► presentation ────────────────────────────────────┘
error.rs is imported by everything except presentation.
```

- `domain` imports nothing from this crate except `error`.
- `pdf`, `raster` never import `fs`, `app`, `cli`, `presentation`, `clap`, `colored`, `indicatif`, or call `println!`/`eprintln!`.
- `presentation` imports only `domain`, `colored`, `humansize`, `serde_json`.
- `cli` is the only module that imports `clap`.

### 3.2 Final file tree

```
src/
  main.rs
  lib.rs
  error.rs
  domain/
    mod.rs
    media.rs
    level.rs
    options.rs
    report.rs
  detection/
    mod.rs
  codec/
    mod.rs
    zlib.rs
    lzw.rs
    jpeg.rs
    png.rs
  pdf/
    mod.rs
    engine.rs
    pipeline.rs
    filters.rs
    content_lexer.rs
    content_minifier.rs
    object_graph.rs
    passes/
      mod.rs
      cleanup.rs
      inline_images.rs
      dedup.rs
      streams.rs
      images.rs
      merge_content.rs
      fonts.rs
      metadata.rs
  raster/
    mod.rs
    engine.rs
    transform.rs
    format_resolution.rs
  fs/
    mod.rs
    reader.rs
    atomic_writer.rs
    discovery.rs
  app/
    mod.rs
    media_compressor.rs
    registry.rs
    pdf_compressor.rs
    image_compressor.rs
    output_path.rs
    compress_file.rs
    batch.rs
    info.rs
    progress.rs
  presentation/
    mod.rs
    pretty.rs
    json.rs
  cli/
    mod.rs
    args.rs
    mapping.rs
  test_support/
    mod.rs
    pdf_fixture.rs
    image_fixture.rs
tests/
  common/mod.rs
  cli_compress_pdf.rs
  cli_compress_image.rs
  cli_batch.rs
  cli_info.rs
  cli_usage.rs
benches/
  compression_bench.rs
scripts/
  check-no-comments.sh
```

`src/lib.rs` (final):
```rust
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod app;
pub mod cli;
pub mod codec;
pub mod detection;
pub mod domain;
pub mod error;
pub mod fs;
pub mod raster;
pub mod pdf;
pub mod presentation;

#[cfg(test)]
pub(crate) mod test_support;
```

### 3.3 Old → new location map

| Old | New | Notes |
|-----|-----|-------|
| `compressor/level.rs` | `domain/level.rs` | clap derive removed; `LevelPreset` added |
| `strategy/mod.rs` `MediaType`, `ImageFormat`, `ImageOutputFormat` | `domain/media.rs` | `ImageOutputFormat` → `OutputFormat { Original, Jpeg, Png }` |
| `strategy/mod.rs` `CompressorStrategy` | `app/media_compressor.rs` `MediaCompressor` | redesigned, actually used (A1, D12) |
| `strategy/resolver.rs`, `image_compressor/format.rs::detect_format_from_bytes` | `detection/mod.rs` | one table (CC2) |
| `strategy/dispatcher.rs` `match MediaType` | `app/registry.rs` `CompressorRegistry::find` | no hardcoded match (D12) |
| `strategy/dispatcher.rs` engine setup | `app/pdf_compressor.rs`, `app/image_compressor.rs` | one strategy per media type |
| `strategy/dispatcher.rs` read/write flow | `app/compress_file.rs` | media-agnostic |
| `image_compressor/format.rs` (rest) | `raster/format_resolution.rs` | |
| `image_compressor/jpeg.rs`, `compressor/image.rs` mozjpeg fns | `codec/jpeg.rs` | one copy (CC1) |
| `image_compressor/png.rs`, `compressor/image.rs::encode_png_optimized` | `codec/png.rs` | one copy (CC1) |
| `image_compressor/transform.rs` | `raster/transform.rs` | |
| `image_compressor/engine.rs` | `raster/engine.rs` | pure, no I/O |
| `image_compressor/metadata.rs` | deleted | CC11 |
| `compressor/pdf_utils.rs` zlib fns | `codec/zlib.rs` | bounded inflate |
| `compressor/pdf_utils.rs` `get_primary_filter` | `pdf/filters.rs` | full chain |
| `compressor/pdf_utils.rs` image constants | `pdf/passes/images.rs` | private consts |
| `compressor/pdf_utils.rs` `estimate_object_size` | deleted | sizes come from real serialization |
| `compressor/stream.rs` LZW | `codec/lzw.rs` | |
| `compressor/stream.rs` compress_streams | `pdf/passes/streams.rs` | |
| `compressor/stream.rs` merge | `pdf/passes/merge_content.rs` | |
| `compressor/stream.rs` comment/whitespace fns | deleted | replaced by `content_minifier.rs` |
| `compressor/cleanup.rs` | `pdf/passes/cleanup.rs` | rewritten in P3 |
| `compressor/dedup.rs` | `pdf/passes/dedup.rs` | key fixed in P2 |
| `compressor/font.rs` `deduplicate_fonts` | deleted | generic dedup covers it |
| `compressor/font.rs` `subset_fonts` | `pdf/passes/fonts.rs` | rewritten in P4 |
| `compressor/inline.rs` | `pdf/passes/inline_images.rs` | rewritten in P2 |
| `compressor/metadata.rs` | `pdf/passes/metadata.rs` | |
| `compressor/image.rs` | `pdf/passes/images.rs` | |
| `compressor/engine.rs` | `pdf/engine.rs` + `pdf/pipeline.rs` | pure |
| `io/reader.rs` | `fs/reader.rs` | |
| `io/writer.rs` | `fs/atomic_writer.rs` | bytes in, used by PDF and image |
| `batch/processor.rs` discovery | `fs/discovery.rs` | |
| `batch/processor.rs` processing | `app/batch.rs` | |
| `batch/processor.rs` `BatchReport` etc. | `domain/report.rs` | |
| `batch/processor.rs` `print_pretty` | `presentation/pretty.rs` | |
| `stats/report.rs` structs | `domain/report.rs` | |
| `stats/report.rs` printing/json | `presentation/` | |
| `utils.rs` | `app/output_path.rs` | |
| `main.rs` logic | `app/*`, `presentation/*` | main stays thin |

### 3.4 Media separation: Strategy + Registry (D12)

**Today.** The code does tell PDF and images apart, but only half uses a pattern:
- `strategy/resolver.rs` `FileTypeResolver::detect` reads the magic bytes and returns `MediaType::{Pdf, Image(_), Unknown}`. This part is fine.
- `strategy/dispatcher.rs:104` then picks the engine with a hardcoded `match`. Adding a media type means editing the `match`, `DispatchConfig` (which mixes PDF and image fields), the batch loop's report extraction (`batch/processor.rs:376`), and `main.rs`.
- `strategy/mod.rs` declares a `CompressorStrategy` trait and both engines implement it, but nothing calls it. Its `can_handle(path)` re-reads the file from disk, and `type Config = ()` carries no configuration.
- The image engine detects the format a second time (`image_compressor/format.rs`), and `info` silently assumes PDF.

**Target.** Strategy pattern for "how to compress this media type", Registry for "which strategy handles this `MediaType`". `app::compress_file` and `app::batch` only talk to the trait, so adding a media type (for example SVG) means one new strategy file plus one line in `CompressorRegistry::standard()`, with no change to the read/detect/write flow (open/closed principle).

```
                 ┌──────────────────────────┐
bytes ─► detect_media_type ─► MediaType ─►  │ CompressorRegistry::find │
                 └────────────┬─────────────┘
                              │ &dyn MediaCompressor
             ┌────────────────┴────────────────┐
             ▼                                 ▼
      PdfCompressor                     ImageCompressor
  (pdf::engine::compress_pdf_bytes)  (raster::engine::compress_image_bytes)
```

`src/app/media_compressor.rs`:
```rust
use std::path::Path;

use crate::domain::media::MediaType;
use crate::domain::options::{ImageOptions, PdfOptions};
use crate::domain::report::MediaDetails;
use crate::error::AppResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionSettings {
    pub pdf: PdfOptions,
    pub image: ImageOptions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedMedia {
    pub bytes: Vec<u8>,
    pub kept_original: bool,
    pub details: MediaDetails,
}

pub trait MediaCompressor: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, media_type: &MediaType) -> bool;
    fn output_extension(
        &self,
        input_path: &Path,
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> String;
    fn compress(
        &self,
        input: &[u8],
        media_type: &MediaType,
        settings: &CompressionSettings,
    ) -> AppResult<CompressedMedia>;
}
```

`src/app/registry.rs`:
```rust
pub struct CompressorRegistry {
    compressors: Vec<Box<dyn MediaCompressor>>,
}

impl CompressorRegistry {
    pub fn new(compressors: Vec<Box<dyn MediaCompressor>>) -> Self;
    pub fn standard() -> Self {
        Self::new(vec![Box::new(PdfCompressor), Box::new(ImageCompressor)])
    }
    pub fn find(&self, media_type: &MediaType) -> AppResult<&dyn MediaCompressor>;
}
```
- `find` returns the first compressor whose `supports` is true, else `AppError::UnsupportedFormat` with the leading bytes in hex (current message format) for `MediaType::Unknown`, or `"<format> input is not supported"` for undecodable image formats (D8).

`src/app/pdf_compressor.rs`:
- `PdfCompressor` is a unit struct.
- `supports`: `matches!(media_type, MediaType::Pdf)`.
- `output_extension`: the input's own extension, or `pdf` when it has none.
- `compress`: `pdf::engine::compress_pdf_bytes(input, &settings.pdf)` → `CompressedMedia { details: MediaDetails::Pdf { passes }, .. }`.

`src/app/image_compressor.rs`:
- `ImageCompressor` is a unit struct.
- `supports`: `matches!(media_type, MediaType::Image(format) if format.is_decodable())`.
- `output_extension`: `raster::format_resolution::output_extension(input_path, input_format, resolved_output_format(input_format, &settings.image))` (P6.3).
- `compress`: `raster::engine::compress_image_bytes(input, format, &settings.image)` → `MediaDetails::Image { input_format, output_format, original_dimensions, output_dimensions }`.

`src/domain/report.rs` changes: `PdfReport` and `ImageReport` are replaced by one report plus a details enum, keeping the JSON keys flat and identical to today:
```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MediaDetails {
    Pdf {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        passes: Vec<PassStat>,
    },
    Image {
        input_format: ImageFormat,
        output_format: ImageFormat,
        original_dimensions: Dimensions,
        output_dimensions: Dimensions,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileReport {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    #[serde(flatten)]
    pub sizes: SizeStats,
    pub kept_original: bool,
    pub duration_ms: u64,
    pub dry_run: bool,
    #[serde(flatten)]
    pub details: MediaDetails,
}

impl FileReport {
    pub fn file_type_label(&self) -> String;
}
```
- `file_type_label` returns `"PDF"` for `Pdf` and the input format's display name for `Image` (replaces the `match` in `batch/processor.rs:376`).
- `presentation::pretty::render_file_report` matches on `MediaDetails` once to print the media-specific lines. That is the only other place that knows about media types, and it is presentation.

`src/app/compress_file.rs` is media-agnostic:
```rust
pub fn compress_file(
    request: &CompressRequest,
    registry: &CompressorRegistry,
) -> AppResult<FileReport> {
    let started = Instant::now();
    let input = read_bounded(&request.input, request.settings.pdf.limits.max_input_bytes)?;
    let media_type = detect_media_type(&input);
    let compressor = registry.find(&media_type)?;
    let output_path = resolve_output_path(request, compressor, &media_type);
    ensure_output_writable(&output_path, &request.input, request.write_mode)?;
    let compressed = compressor.compress(&input, &media_type, &request.settings)?;
    write_unless_dry_run(&output_path, &compressed.bytes, request.write_mode)?;
    Ok(build_report(request, output_path, &input, compressed, started))
}
```
- `CompressRequest` and `BatchRequest` hold `settings: CompressionSettings` instead of separate `pdf` / `image` fields (update section 4.4 accordingly).
- `info` stays PDF-only on purpose; it returns `AppError::UnsupportedFormat("info supports PDF files only")` when `detect_media_type` is not `Pdf`.

Tests (`app/registry.rs`, `app/*_compressor.rs`):
- `registry_finds_pdf_compressor_for_pdf`
- `registry_finds_image_compressor_for_jpeg_and_png`
- `registry_rejects_unknown_media_with_hex_bytes_in_message`
- `registry_rejects_undecodable_image_formats`
- `registry_uses_first_matching_compressor` (custom registry with two fake strategies)
- `compress_file_uses_strategy_from_registry` (fake `MediaCompressor` that records calls and returns fixed bytes; proves `compress_file` has no media-specific branches)
- `pdf_compressor_output_extension_keeps_input_extension`
- `image_compressor_output_extension_follows_resolved_format`
- `file_report_file_type_label_for_pdf_and_image`

---

## 4. Core Type Definitions

Use these exact definitions. They are referenced by every phase.

### 4.1 `src/error.rs`

```rust
use std::path::PathBuf;

use thiserror::Error;

pub const EXIT_CODE_RUNTIME_ERROR: u8 = 1;
pub const EXIT_CODE_USAGE_ERROR: u8 = 2;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("input file not found: {}", .0.display())]
    InputNotFound(PathBuf),
    #[error("input file '{}' is {size} bytes, which exceeds the limit of {limit} bytes", path.display())]
    InputTooLarge { path: PathBuf, size: u64, limit: u64 },
    #[error("'{}' is not a directory", .0.display())]
    InvalidInputDir(PathBuf),
    #[error("output file '{}' already exists; use --overwrite to replace it", .0.display())]
    OutputExists(PathBuf),
    #[error("'{}' is outside the input directory", .0.display())]
    PathOutsideInput(PathBuf),
    #[error("output path would overwrite the input file '{}'", .0.display())]
    OutputEqualsInput(PathBuf),
    #[error("invalid PDF: {0}")]
    InvalidPdf(String),
    #[error("PDF is encrypted and cannot be processed without a password")]
    EncryptedPdf,
    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),
    #[error("image decode error: {0}")]
    ImageDecode(String),
    #[error("encode error: {0}")]
    Encode(String),
    #[error("decompression failed: {0}")]
    Decompress(String),
    #[error("decompressed data exceeds the limit of {limit} bytes")]
    DecompressionLimitExceeded { limit: usize },
    #[error("image of {width}x{height} pixels exceeds the limit of {limit} pixels")]
    ImageTooLarge { width: u32, height: u32, limit: u64 },
    #[error("invalid content stream: {0}")]
    InvalidContentStream(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlob(#[from] globset::Error),
    #[error("JSON serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("PDF library error: {0}")]
    Lopdf(#[from] lopdf::Error),
}

impl AppError {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::OutputExists(_) | Self::InvalidInputDir(_) | Self::InvalidGlob(_) => {
                EXIT_CODE_USAGE_ERROR
            }
            _ => EXIT_CODE_RUNTIME_ERROR,
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
```

`AppError::Custom` and `AppError::WalkDir` are deleted in P6. Until then they may remain for compilation.

### 4.2 `src/domain/media.rs`

```rust
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ImageFormat {
    #[serde(rename = "JPEG")]
    Jpeg,
    #[serde(rename = "PNG")]
    Png,
    #[serde(rename = "GIF")]
    Gif,
    #[serde(rename = "TIFF")]
    Tiff,
    #[serde(rename = "BMP")]
    Bmp,
    #[serde(rename = "WebP")]
    WebP,
}

impl ImageFormat {
    pub fn display_name(self) -> &'static str { ... }
    pub fn canonical_extension(self) -> &'static str { ... }
    pub fn is_decodable(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaType {
    Pdf,
    Image(ImageFormat),
    Unknown { leading_bytes: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Original,
    Jpeg,
    Png,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}
```

`canonical_extension`: Jpeg→`jpg`, Png→`png`, Gif→`gif`, Tiff→`tif`, Bmp→`bmp`, WebP→`webp`. `display_name`: `JPEG`, `PNG`, `GIF`, `TIFF`, `BMP`, `WebP`.

`Dimensions` serializes as an object `{"width":..,"height":..}`. The old JSON used a `[w, h]` tuple; to keep JSON stable add `#[serde(into = "(u32, u32)")]` with `impl From<Dimensions> for (u32, u32)`.

### 4.3 `src/domain/level.rs`

```rust
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompressionLevel {
    Low,
    #[default]
    Medium,
    High,
    Extreme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPolicy {
    pub remove_markup_annotations: bool,
    pub remove_forms: bool,
    pub remove_struct_tree: bool,
    pub remove_embedded_files: bool,
    pub remove_output_intents: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelPreset {
    pub jpeg_quality: u8,
    pub target_dpi: Option<u32>,
    pub deflate_level: u32,
    pub png_effort: u8,
    pub strip_metadata: bool,
    pub grayscale: bool,
    pub cleanup: CleanupPolicy,
}

impl CompressionLevel {
    pub fn preset(self) -> LevelPreset { ... }
}

impl fmt::Display for CompressionLevel { ... }
```

Preset values (keep current behavior except cleanup, which follows P3.1):

| Field | Low | Medium | High | Extreme |
|-------|-----|--------|------|---------|
| `jpeg_quality` | 90 | 75 | 60 | 40 |
| `target_dpi` | None | Some(200) | Some(150) | Some(96) |
| `deflate_level` | 6 | 9 | 9 | 9 |
| `png_effort` | 2 | 4 | 5 | 6 |
| `strip_metadata` | false | false | true | true |
| `grayscale` | false | false | false | true |
| `cleanup.remove_markup_annotations` | false | false | true | true |
| `cleanup.remove_forms` | false | false | false | true |
| `cleanup.remove_struct_tree` | false | false | true | true |
| `cleanup.remove_embedded_files` | false | false | true | true |
| `cleanup.remove_output_intents` | false | false | false | true |

### 4.4 `src/domain/options.rs`

```rust
use std::path::PathBuf;

use super::level::CompressionLevel;
use super::media::OutputFormat;

pub const DEFAULT_MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_MAX_STREAM_BYTES: usize = 256 * 1024 * 1024;
pub const DEFAULT_MAX_IMAGE_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_stream_bytes: usize,
    pub max_image_pixels: u64,
}

impl Default for Limits { ... }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Preserve,
    Grayscale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncodingMode {
    Lossy,
    Lossless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    FollowLevel,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontOptimization {
    Disabled,
    Subset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOptions {
    pub level: CompressionLevel,
    pub jpeg_quality: u8,
    pub target_dpi: Option<u32>,
    pub color_mode: ColorMode,
    pub image_encoding: ImageEncodingMode,
    pub metadata: MetadataPolicy,
    pub fonts: FontOptimization,
    pub limits: Limits,
}

impl PdfOptions {
    pub fn for_level(level: CompressionLevel) -> Self { ... }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOptions {
    pub level: CompressionLevel,
    pub quality: u8,
    pub output_format: OutputFormat,
    pub max_dimension: Option<u32>,
    pub color_mode: ColorMode,
    pub encoding: ImageEncodingMode,
    pub metadata: MetadataPolicy,
    pub limits: Limits,
}

impl ImageOptions {
    pub fn for_level(level: CompressionLevel) -> Self { ... }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Write,
    Overwrite,
    DryRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailurePolicy {
    StopOnFirstError,
    ContinueOnError,
}

```

`CompressRequest` and `BatchRequest` live in `src/app/mod.rs` (not `domain`), because they carry `CompressionSettings` from `app/media_compressor.rs` (section 3.4):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressRequest {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub settings: CompressionSettings,
    pub write_mode: WriteMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRequest {
    pub input_dir: PathBuf,
    pub output_dir: Option<PathBuf>,
    pub pattern: String,
    pub recursive: bool,
    pub workers: usize,
    pub failure_policy: FailurePolicy,
    pub settings: CompressionSettings,
    pub write_mode: WriteMode,
}
```

`for_level` fills every field from `level.preset()`: `color_mode = Grayscale` when `preset.grayscale`, `image_encoding = Lossy`, `metadata = FollowLevel`, `fonts = Disabled`, `limits = Limits::default()`.

### 4.5 `src/domain/report.rs`

JSON field names must stay identical to today's output. P0.4 snapshots them first.

```rust
use std::path::PathBuf;

use serde::Serialize;

use super::media::{Dimensions, ImageFormat};

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SizeStats {
    pub original_size: u64,
    pub compressed_size: u64,
    pub bytes_saved: i64,
    pub ratio: f64,
}

impl SizeStats {
    pub fn new(original_size: u64, compressed_size: u64) -> Self { ... }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PassStat {
    pub pass: &'static str,
    pub bytes_saved: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MediaDetails {
    Pdf {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        passes: Vec<PassStat>,
    },
    Image {
        input_format: ImageFormat,
        output_format: ImageFormat,
        original_dimensions: Dimensions,
        output_dimensions: Dimensions,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileReport {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    #[serde(flatten)]
    pub sizes: SizeStats,
    pub kept_original: bool,
    pub duration_ms: u64,
    pub dry_run: bool,
    #[serde(flatten)]
    pub details: MediaDetails,
}

impl FileReport {
    pub fn file_type_label(&self) -> String { ... }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status")]
pub enum FileOutcome {
    #[serde(rename = "ok")]
    Ok {
        #[serde(flatten)]
        sizes: SizeStats,
        file_type: String,
    },
    #[serde(rename = "skipped")]
    Skipped { reason: String },
    #[serde(rename = "error")]
    Error { error: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub outcome: FileOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchReport {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub total_files: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub total_original_bytes: u64,
    pub total_compressed_bytes: u64,
    pub total_bytes_saved: i64,
    pub overall_ratio: f64,
    pub duration_ms: u64,
    pub dry_run: bool,
    pub files: Vec<FileResult>,
}

impl BatchReport {
    pub fn from_results(context: BatchContext, files: Vec<FileResult>) -> Self { ... }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchContext {
    pub input_dir: PathBuf,
    pub output_dir: PathBuf,
    pub duration_ms: u64,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InfoReport {
    pub path: String,
    pub file_size_bytes: u64,
    pub page_count: usize,
    pub pdf_version: String,
}
```

`SizeStats::new`: `bytes_saved = original as i64 - compressed as i64`; `ratio = if original > 0 { (1.0 - compressed / original) * 100.0 } else { 0.0 }`.

New JSON keys (`kept_original`, `passes`) are additive. `passes` is skipped when empty: `#[serde(skip_serializing_if = "Vec::is_empty")]`.

---

## 5. Phases

| Phase | Content | Closes |
|-------|---------|--------|
| P0 | Baseline and safety net | fmt failure, N16, groundwork T1 |
| P1 | Structural move to the new layout, no behavior change | A1, A2, CC2, CC11 (partly CC1, CC3) |
| P2 | Content lexer, streams, filters, dedup, inline images | C1, C2, C3, C4, C5, N5, N6, N7, N8, N9, N10, CC3, T2, T5 |
| P3 | Cleanup and metadata per level | C6, C7, N14 |
| P4 | Font subsetting rewrite | N11, N12, N13 |
| P5 | Security hardening | S1, S2, S3, S4, S5, S6 |
| P6 | App layer, presentation, entry point, correctness | C8–C14, N1–N4, A3, A4, A5, CC1, CC4–CC10, CC12, D8, D10 |
| P7 | No comments, English, readability | rules, N15 |
| P8 | Test completion, coverage gate, docs | T1, T3, T4, T6, T7, T8, A6 |

---

### P0. Baseline and Safety Net

#### P0.1 Format
- Run `make fmt`. Commit: `style: apply rustfmt`.

#### P0.2 Cargo.toml decisions
- **Files:** `Cargo.toml`
- Steps:
  1. Delete the line `panic = "abort"` under `[profile.release]` (D1).
  2. Move `tempfile = "3.10"` from `[dev-dependencies]` to `[dependencies]` (D2).
  3. `make check`.
- Commit: `build: allow unwinding in release and promote tempfile to runtime dependency`.

#### P0.3 Test-support fixtures
- **Files:** new `src/test_support/mod.rs`, `src/test_support/pdf_fixture.rs`, `src/test_support/image_fixture.rs`; `src/lib.rs` (add `#[cfg(test)] pub(crate) mod test_support;`).
- `PdfFixture` is a builder that produces a `lopdf::Document` in memory. Required API:
  ```rust
  pub struct PdfFixture { document: Document, pages_id: ObjectId, page_ids: Vec<ObjectId> }

  impl PdfFixture {
      pub fn new() -> Self;
      pub fn page_with_content(self, content: &[u8]) -> Self;
      pub fn page_with_content_streams(self, contents: &[&[u8]]) -> Self;
      pub fn with_flate_content(self) -> Self;
      pub fn with_inherited_font_resource(self) -> Self;
      pub fn with_image(self, image: FixtureImage) -> Self;
      pub fn with_annotation(self, subtype: &str) -> Self;
      pub fn with_acroform_widget(self) -> Self;
      pub fn with_signature_field(self) -> Self;
      pub fn with_named_destination(self, name: &str) -> Self;
      pub fn with_output_intent(self) -> Self;
      pub fn with_struct_tree(self) -> Self;
      pub fn with_embedded_file(self) -> Self;
      pub fn with_thumbnail(self) -> Self;
      pub fn with_javascript_open_action(self) -> Self;
      pub fn with_info_and_xmp_metadata(self) -> Self;
      pub fn with_icc_color_space_stream(self) -> Self;
      pub fn with_sampled_function_stream(self) -> Self;
      pub fn with_form_xobject(self, content: &[u8]) -> Self;
      pub fn build(self) -> Document;
      pub fn to_bytes(self) -> Vec<u8>;
  }

  pub enum FixtureImage {
      Jpeg { width: u32, height: u32 },
      FlateRgb { width: u32, height: u32 },
      FlateRgbWithPngPredictor { width: u32, height: u32 },
      FilterChain { filters: Vec<&'static str> },
  }

  pub fn assert_no_dangling_references(document: &Document);
  pub fn decoded_page_content(document: &Document, page_number: u32) -> Vec<u8>;
  ```
  - Images use a gradient pattern (pixel value = `(x + y) % 256`), never all-zero.
  - `to_bytes` calls `document.save_to(&mut Vec<u8>)`.
  - `assert_no_dangling_references` walks every object recursively (dictionaries, arrays, stream dicts, trailer) and panics with the list of `ObjectId`s that are referenced but missing from `document.objects`.
- `image_fixture.rs`: `gradient_rgb(w,h)`, `gradient_rgba_with_transparency(w,h)`, `gradient_rgb16(w,h)`, `encode_as(image, image::ImageFormat) -> Vec<u8>`.
- Tests inside `test_support` (yes, the helpers are tested): `pdf_fixture_builds_loadable_document`, `pdf_fixture_round_trips_through_bytes`, `assert_no_dangling_references_detects_missing_object` (`#[should_panic]`).
- Commit: `test: add in-memory PDF and image fixture builders`.

#### P0.4 Characterization tests (lock today's good behavior)
- **Files:** new `tests/common/mod.rs` content (replace), `tests/cli_compress_pdf.rs`.
- `tests/common/mod.rs` exposes a small copy of the fixture builder for integration tests (integration tests cannot use `pub(crate)` items): `minimal_pdf_bytes(pages: usize, content: &[u8]) -> Vec<u8>`, `pdf_with_jpeg_image_bytes() -> Vec<u8>`, `write_temp(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf`.
- Tests:
  - `compress_pdf_json_report_has_stable_keys`: run `compress <pdf> --json --dry-run`, parse JSON, assert the key set is exactly `input_path, output_path, original_size, compressed_size, bytes_saved, ratio, duration_ms, dry_run` (later phases may only add `kept_original`, `passes`; update this test when they do).
  - `compress_image_json_report_has_stable_keys`: same for a PNG, keys `input_path, output_path, input_format, output_format, original_size, compressed_size, bytes_saved, ratio, original_dimensions, output_dimensions, duration_ms, dry_run`.
  - `batch_json_report_has_stable_keys`.
  - `compress_pdf_preserves_page_count_at_every_level` (loop over 4 levels, load output with `lopdf::Document::load`).
- Commit: `test: lock JSON report schema and PDF page count behavior`.

#### P0.5 Quality gates
- **Files:** new `scripts/check-no-comments.sh`, `Makefile`.
- `scripts/check-no-comments.sh`:
  ```bash
  #!/usr/bin/env bash
  set -euo pipefail
  comment_at_line_start='^[[:space:]]*(//|/\*|\*/)'
  trailing_comment='[^[:space:]:"][[:space:]]+//([[:space:]]|/|!|$)'
  if matches=$(grep -rnE --include='*.rs' "${comment_at_line_start}|${trailing_comment}" src tests benches); then
      echo "Comments are not allowed in Rust source files:"
      echo "${matches}"
      exit 1
  fi
  ```
  `chmod +x scripts/check-no-comments.sh`.
- `Makefile` additions (do **not** add them to `check` yet):
  ```make
  no-comments:
  	./scripts/check-no-comments.sh

  coverage:
  	cargo llvm-cov --lib --fail-under-lines 90 --ignore-filename-regex 'test_support'
  ```
  Add both to `.PHONY`.
- Fix N16 in `benches/compression_bench.rs`: build the benchmark input with an in-crate public helper is not possible (test_support is test-only), so generate a PDF in the bench itself with lopdf (one page, one 1024×1024 Flate RGB gradient image, one JPEG image). Delete the `inputs/sample.pdf` lookup.
- Commit: `build: add no-comments and coverage targets; make bench self-contained`.

---

### P1. Structural Move (no behavior change)

Goal: the layout of section 3.2 exists, old modules are gone, all existing tests pass unchanged in meaning. Do moves with `git mv` first, then edit, and commit each step separately so history follows files.

#### P1.1 `domain` module
- **Files:** new `src/domain/{mod,media,level,options,report}.rs`; `git mv src/compressor/level.rs src/domain/level.rs`.
- Steps:
  1. Implement section 4.2–4.5 types. `report.rs` holds data only. Move `print_pretty`/`to_json` bodies unchanged into `src/presentation/pretty.rs` and `src/presentation/json.rs` as free functions:
     ```rust
     pub fn render_file_report(report: &FileReport) -> String;
     pub fn render_batch_report(report: &BatchReport) -> String;
     pub fn render_info_report(report: &InfoReport) -> String;
     pub fn to_pretty_json<T: Serialize>(value: &T) -> AppResult<String>;
     ```
     Renderers return `String` (build with `writeln!` on a `String`). `main` prints. This makes them unit-testable.
  2. Remove `clap::ValueEnum` from `CompressionLevel`. In `cli/args.rs` add `LevelArg` and `FormatArg` `ValueEnum` enums with `From<LevelArg> for CompressionLevel` and `From<FormatArg> for OutputFormat`.
  3. Update all imports. Delete `src/stats/`.
- Tests moved and kept: level tests, report JSON tests (rewrite them to assert exact values: e.g. `assert_eq!(json["bytes_saved"], 500_000)`).
- Commit: `refactor: introduce domain module and move reports to presentation`.

#### P1.2 `codec` module
- **Files:** new `src/codec/{mod,zlib,lzw,jpeg,png}.rs`.
- Steps:
  1. `zlib.rs`: move `zlib_compress`, `zlib_decompress` from `pdf_utils.rs`, renamed `deflate(data, level: u32) -> AppResult<Vec<u8>>` and `inflate(data) -> AppResult<Vec<u8>>` returning `AppError::Encode`/`AppError::Decompress`. Delete `zlib_compress_best` and `zlib_decompress_or_raw` callers by passing `9` / handling the error explicitly at each call site (the lenient fallback is removed; P2.5 handles inline images properly).
  2. `lzw.rs`: move `lzw_decompress` as `pub fn lzw_decode(data: &[u8]) -> AppResult<Vec<u8>>` (fixed in P2.3).
  3. `jpeg.rs`: one implementation, merged from `image_compressor/jpeg.rs` and the mozjpeg functions in `compressor/image.rs`:
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
     pub enum ScanMode { Progressive, Baseline }

     pub struct JpegSettings { pub quality: u8, pub color_mode: ColorMode, pub scan_mode: ScanMode }

     pub fn encode_jpeg(image: &DynamicImage, settings: JpegSettings) -> AppResult<Vec<u8>>;
     ```
     Wrap the mozjpeg calls in `std::panic::catch_unwind(AssertUnwindSafe(|| ...))` and map a panic to `AppError::Encode("mozjpeg panicked")`.
  4. `png.rs`: one implementation:
     ```rust
     pub const MAX_OXIPNG_PRESET: u8 = 6;
     pub fn encode_png(image: &DynamicImage, effort: u8) -> AppResult<Vec<u8>>;
     ```
     Keep today's behavior (RGB/L8 encode, then `oxipng::optimize_from_memory`). Alpha handling changes in P6.4.
  5. Both `compressor/image.rs` and `image_compressor/*` call `codec::*`. Delete the duplicate functions.
- Tests: move existing codec tests; add `encode_jpeg_baseline_and_progressive_both_start_with_soi`, `deflate_then_inflate_round_trips`.
- Commit: `refactor: extract shared codec module`.

#### P1.3 `detection` module
- **Files:** new `src/detection/mod.rs`; delete `src/strategy/resolver.rs`, `image_compressor/format.rs::detect_format_from_bytes`.
- API:
  ```rust
  pub const MAGIC_BYTES_TO_READ: usize = 16;
  pub fn detect_media_type(leading_bytes: &[u8]) -> MediaType;
  ```
  Pure function on bytes. Callers read the bytes.
- Tests: move all resolver tests, call `detect_media_type` on byte slices.
- Commit: `refactor: single magic-byte detection module`.

#### P1.4 `fs` module
- **Files:** `git mv src/io/reader.rs src/fs/reader.rs`, `git mv src/io/writer.rs src/fs/atomic_writer.rs`, new `src/fs/discovery.rs` (move `discover_files` from `batch/processor.rs` unchanged), `src/fs/mod.rs`. Delete `src/io/`.
- Commit: `refactor: move file system code to fs module`.

#### P1.5 `pdf` module
- **Files:** `git mv` each `src/compressor/*.rs` into `src/pdf/passes/` per section 3.3 (`stream.rs` → `streams.rs`, `inline.rs` → `inline_images.rs`, `image.rs` → `images.rs`, `font.rs` → `fonts.rs`). `engine.rs` → `src/pdf/engine.rs`. `pdf_utils.rs` → split: `get_primary_filter` into `src/pdf/filters.rs`, constants into `passes/images.rs`, delete `estimate_object_size` users by returning `0` estimates for now (cleanup/metadata estimates are replaced by real sizes in P6.1).
- Move `merge_page_content_streams` from `streams.rs` to `passes/merge_content.rs`.
- `CompressorEngine` keeps its current behavior for now.
- Commit: `refactor: move PDF compressor into pdf module`.

#### P1.6 `raster` module
- The module is named `raster`, not `image`: a crate-root module called `image` makes `image::...` paths ambiguous with the `image` crate.
- **Files:** `git mv src/image_compressor/engine.rs src/raster/engine.rs`, `transform.rs`, `format.rs` → `format_resolution.rs`. Delete `image_compressor/metadata.rs` (CC11) and `image_compressor/mod.rs`'s `CompressorStrategy` impl.
- Commit: `refactor: move image compressor into image module`.

#### P1.7 `app` module and removal of `strategy`
- **Files:** new `src/app/{mod,media_compressor,registry,pdf_compressor,image_compressor,output_path,compress_file,batch,info,progress}.rs`; delete `src/strategy/`, `src/batch/`, `src/utils.rs`.
- Steps:
  1. `output_path.rs` gets `default_output_path` and `default_image_output_path` from `utils.rs` with their tests.
  2. Implement the Strategy + Registry design of section 3.4 (D12): `MediaCompressor` trait, `CompressorRegistry`, `PdfCompressor`, `ImageCompressor`, `CompressionSettings`, `CompressedMedia`, `MediaDetails`, `FileReport`. In P1 the two strategies wrap today's engines (`CompressorEngine`, `ImageCompressorEngine`) with today's field assignments; the engines become pure byte-in/byte-out in P6.1/P6.2, and only the strategy bodies change then.
  3. `compress_file.rs`: `pub fn compress_file(request: &CompressRequest, registry: &CompressorRegistry) -> AppResult<FileReport>` with no `match` on `MediaType`. Until P5.4/P6.1 the strategies may still read and write through the engines' path-based APIs; the shape of `compress_file` from section 3.4 is reached in P6.1.
  4. `batch.rs` gets `process_batch(request, registry, progress)` and uses `FileReport::file_type_label` instead of matching on report types. `info.rs` gets the info logic from `main.rs` returning `InfoReport`.
  5. Add the section 3.4 tests (`registry_*`, `compress_file_uses_strategy_from_registry`, `file_report_file_type_label_for_pdf_and_image`).
  4. `progress.rs`:
     ```rust
     pub trait BatchProgress: Sync {
         fn start(&self, total_files: usize);
         fn file_finished(&self);
         fn finish(&self);
     }
     pub struct NoProgress;
     impl BatchProgress for NoProgress { ... }
     ```
     `process_batch` takes `&dyn BatchProgress`. The indicatif implementation lives in `main.rs` (`IndicatifProgress`). This removes `indicatif` from the library modules.
  6. Delete the old `CompressorStrategy` trait, both impls, and `CompressorDispatcher`.
- Commit: `refactor: separate media types with strategy and registry patterns`.

#### P1.8 `cli/mapping.rs`
- **Files:** new `src/cli/mapping.rs`.
- `impl From<CompressArgs> for CompressRequest`, `impl From<BatchArgs> for BatchRequest`. Keep today's semantics exactly (including current bugs N1–N3; they are fixed in P6.6). `main.rs` uses them.
- Tests: `compress_args_map_level_and_overrides`, `batch_args_map_continue_on_error_to_failure_policy`.
- Commit: `refactor: map CLI args to domain requests in cli::mapping`.

P1 done when: `make check` green, `rg 'strategy|image_compressor|compressor::|stats::|io::|batch::processor' src tests benches` returns nothing.

---

### P2. PDF Data Integrity: Lexer, Streams, Filters, Dedup, Inline Images

#### P2.1 Content-stream lexer (foundation for C1, C2, N13, inline images)
- **Files:** new `src/pdf/content_lexer.rs`.
- Why not lopdf: `lopdf::content::Content::decode` stops silently at the first `%` comment or unknown token and returns the operations parsed so far (no end-of-input check, lopdf 0.34 `parser.rs:304`). Using it drops content.
- API:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum TokenKind {
      Number,
      Name,
      LiteralString,
      HexString,
      ArrayStart,
      ArrayEnd,
      DictStart,
      DictEnd,
      Keyword,
      InlineImage,
  }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct Token<'a> {
      pub kind: TokenKind,
      pub raw: &'a [u8],
  }

  pub fn tokenize(data: &[u8]) -> AppResult<Vec<Token<'_>>>;
  pub fn decode_literal_string(raw: &[u8]) -> Vec<u8>;
  pub fn decode_hex_string(raw: &[u8]) -> Vec<u8>;
  ```
- Lexing rules (PDF 1.7 §7.2):
  - Whitespace bytes: `0x00 0x09 0x0A 0x0C 0x0D 0x20`. Skipped.
  - Delimiters: `( ) < > [ ] { } / %`.
  - `%` starts a comment that runs to the next `\r` or `\n`. Comments produce no token.
  - `(` starts a literal string. Track nesting depth: unescaped `(` increments, unescaped `)` decrements, `\` skips the next byte. `raw` includes the outer parentheses. Unterminated string → `AppError::InvalidContentStream`.
  - `<<` → `DictStart`, `>>` → `DictEnd`. A single `<` starts a hex string up to `>`; non-hex, non-whitespace byte inside → error. `raw` includes the angle brackets.
  - `[` `]` → array tokens. `{` `}` → treat as `Keyword` tokens (PostScript calculator braces never appear in content streams; keep them verbatim).
  - `/` starts a name: `/` plus all following regular characters (not whitespace, not delimiter).
  - Any other run of regular characters is `Number` if it matches `^[+-]?(\d+\.?\d*|\.\d+)$`, otherwise `Keyword`.
  - Keyword `BI` starts an inline image. Consume tokens until keyword `ID`, then exactly one whitespace byte, then image data up to the first occurrence of whitespace + `EI` + (whitespace or end of data). When the inline dictionary has no `/F` or `/Filter` entry, and `/W`, `/H`, `/BPC`, color components can be read, compute the expected data length `ceil(W * components * BPC / 8) * H` and take exactly that many bytes, then require whitespace + `EI`. Emit one `InlineImage` token whose `raw` spans from `B` of `BI` to `I` of `EI`. Missing `EI` → error.
  - `decode_literal_string` handles `\n \r \t \b \f \( \) \\`, octal `\ddd` (1–3 digits), backslash-newline line continuation, and drops a lone backslash before other bytes.
- Tests (all in `content_lexer.rs`):
  - `tokenize_keeps_percent_inside_literal_string` — `(50%) Tj` gives `LiteralString "(50%)"`, `Keyword "Tj"`.
  - `tokenize_skips_comment_until_end_of_line`.
  - `tokenize_handles_nested_and_escaped_parentheses` — `(a(b)c\)d) Tj`.
  - `tokenize_distinguishes_dict_start_from_hex_string`.
  - `tokenize_reads_names_numbers_and_keywords` — `/F1 12 Tf 1.5 -0.25 .5 TL`.
  - `tokenize_emits_inline_image_as_single_token_with_binary_data` — data bytes include `0x00`, `0x0A`, `E`, `I`.
  - `tokenize_uses_expected_length_for_unfiltered_inline_image` — data contains ` EI ` before its real end.
  - `tokenize_rejects_unterminated_literal_string`.
  - `tokenize_rejects_inline_image_without_end_marker`.
  - `decode_literal_string_handles_all_escape_forms`.
  - `decode_hex_string_pads_odd_length_with_zero`.
- Commit: `feat(pdf): add content stream lexer`.

#### P2.2 Content minifier (C1, C2)
- **Files:** new `src/pdf/content_minifier.rs`; `src/pdf/passes/streams.rs` (delete `remove_content_stream_comments`, `compact_whitespace`, `is_content_stream`).
- API:
  ```rust
  pub fn minify_content(data: &[u8]) -> Option<Vec<u8>>;
  ```
  - `tokenize(data)`; on error → `None`.
  - If any token is `InlineImage` → `None` (leave untouched; inline images are converted earlier in the pipeline, see D11).
  - Emit token `raw` bytes joined by a single `b' '`; omit the space around `ArrayStart`, `ArrayEnd`, `DictStart`, `DictEnd`, before `Name`, and around `LiteralString`/`HexString` boundaries (they are self-delimiting).
  - Verify: `tokenize(output)` must yield the same `(kind, raw)` sequence as the input tokens. Otherwise `None`.
  - Return `Some(output)` only if `output.len() < data.len()`.
- Tests:
  - `minify_content_preserves_spaces_inside_literal_string` — `(A   B) Tj`.
  - `minify_content_preserves_percent_inside_literal_string` — `(Discount 50%) Tj`.
  - `minify_content_removes_comments_and_collapses_whitespace`.
  - `minify_content_returns_none_for_inline_images`.
  - `minify_content_returns_none_when_not_smaller`.
  - `minify_content_output_tokenizes_to_same_tokens` (property-style loop over 10 handwritten streams).
- Commit: `fix(pdf): minify content streams with a real tokenizer`.

#### P2.3 Filters and LZW (C4, C5, N5)
- **Files:** `src/pdf/filters.rs`, `src/codec/lzw.rs`.
- `filters.rs` API:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum StreamFilter { Flate, Lzw { early_change: bool }, Dct, Other(Vec<u8>) }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct StreamEncoding { pub filters: Vec<StreamFilter>, pub has_predictor: bool }

  impl StreamEncoding {
      pub fn from_dict(dict: &Dictionary) -> Self;
      pub fn is_plain_decodable(&self) -> bool;
  }

  pub fn decode_plain_stream(stream: &Stream, max_bytes: usize) -> AppResult<Vec<u8>>;
  pub fn store_as_flate(stream: &mut Stream, raw: &[u8], deflate_level: u32) -> AppResult<()>;
  ```
  - `from_dict` reads `/Filter` (name or array) and `/DecodeParms` (dict, array of dicts, or null), resolving abbreviations `Fl`, `LZW`, `DCT`. `has_predictor` is true when any parms dict has `/Predictor` > 1. `early_change` defaults to true, false when `/EarlyChange 0`.
  - `is_plain_decodable`: filters is empty, or exactly `[Flate]`, or exactly `[Lzw]`; and `!has_predictor`.
  - `decode_plain_stream`: returns `AppError::Decompress("unsupported filter chain")` when not plain decodable. Uses `codec::zlib::inflate_bounded` / `codec::lzw::lzw_decode` with `max_bytes` (bounded versions are added in P5.1; in P2 pass `usize::MAX`).
  - `store_as_flate`: deflate, set `/Filter /FlateDecode`, set `/Length`, remove `/DecodeParms`.
- `streams.rs` re-compresses only `is_plain_decodable` streams; any other non-image stream gets `debug!(object_id = ?id, filters = ?encoding.filters, "Stream left untouched: unsupported filter chain")`. Unknown filters (JBIG2, CCITT, JPX, ASCII85, ASCIIHex, RunLength, Crypt) keep using `warn!` as CLAUDE.md requires.
- `streams.rs` minifies only streams whose IDs are content streams: page `/Contents` (single or array) plus streams with `/Subtype /Form`. Build the ID set with a helper in `src/pdf/object_graph.rs`:
  ```rust
  pub fn content_stream_ids(document: &Document) -> BTreeSet<ObjectId>;
  pub fn resolve<'a>(document: &'a Document, object: &'a Object) -> &'a Object;
  pub fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary>;
  ```
- `passes/images.rs` (N5): before decoding a Flate image, require `StreamEncoding::from_dict(&stream.dict).is_plain_decodable()`; otherwise skip the image with `debug!`. Also require the decoded length to equal `width * height * components * bpc / 8` exactly; otherwise skip.
- LZW (C5) in `codec/lzw.rs`:
  ```rust
  pub const LZW_CLEAR_CODE: u16 = 256;
  pub const LZW_END_OF_DATA_CODE: u16 = 257;
  pub const LZW_FIRST_DYNAMIC_CODE: u16 = 258;
  pub const LZW_MAX_CODE_WIDTH: u32 = 12;

  pub struct LzwSettings { pub early_change: bool, pub max_output_bytes: usize }
  pub fn lzw_decode(data: &[u8], settings: LzwSettings) -> AppResult<Vec<u8>>;
  ```
  Width increase condition after adding an entry: `next_code + u16::from(early_change) >= 1 << width` and `width < 12`. Output growing past `max_output_bytes` → `AppError::DecompressionLimitExceeded`.
- Tests:
  - `filters.rs`: `from_dict_parses_single_name`, `from_dict_parses_filter_array_and_parms_array`, `from_dict_expands_abbreviations`, `is_plain_decodable_rejects_filter_chain`, `is_plain_decodable_rejects_png_predictor`, `store_as_flate_removes_decode_parms_and_sets_length`.
  - `streams.rs`: `compress_streams_leaves_flate_ascii_hex_chain_untouched`, `compress_streams_leaves_predictor_stream_untouched`, `compress_streams_never_touches_icc_profile_stream`, `compress_streams_never_touches_sampled_function_stream`, `compress_streams_keeps_percent_inside_string_literal` (fixture page content, then decode page content and compare tokens).
  - `images.rs`: `compress_images_skips_flate_image_with_png_predictor` (stream bytes unchanged).
  - `lzw.rs`: `lzw_decode_matches_pdf_spec_example` — input `[0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01]` decodes to `[0x2D, 0x2D, 0x2D, 0x2D, 0x2D, 0x41, 0x2D, 0x2D, 0x2D, 0x42]` (PDF 1.7 §7.4.4.2). `lzw_decode_round_trips_across_all_code_widths` using a test-only encoder (`fn lzw_encode_for_test(data, early_change) -> Vec<u8>` inside `mod tests`) on 20 000 pseudo-random bytes with a small alphabet, for both `early_change` values. `lzw_decode_resets_dictionary_on_clear_code`. `lzw_decode_rejects_invalid_first_code`. `lzw_decode_stops_at_output_limit`.
- Commit: `fix(pdf): respect filter chains, predictors and LZW early change`.

#### P2.4 Dedup key (N6)
- **Files:** `src/pdf/passes/dedup.rs`.
- Key: `(dictionary without /Length, content)`. Implementation: group candidates by a `u64` hash of content (`std::hash::DefaultHasher`), then within a group compare `dict_without_length(a) == dict_without_length(b) && a.content == b.content` before treating as duplicate. Do not clone all contents up front; store `(ObjectId, u64)` and look up bytes when comparing.
- Remove duplicates by rewriting references only (`traverse_objects`), then let `prune_objects` delete them. Do not call `doc.objects.remove`.
- Tests: `dedup_merges_streams_with_identical_dict_and_content`, `dedup_keeps_streams_with_same_content_but_different_dict`, `dedup_rewrites_references_and_leaves_no_dangling_references`.
- Commit: `fix(pdf): include stream dictionary in dedup key`.

#### P2.5 Inline images rewrite (N7, N8, N9, N10, CC3)
- **Files:** `src/pdf/passes/inline_images.rs`, `src/pdf/pipeline.rs` (order), `src/pdf/object_graph.rs`.
- Rewrite rules:
  1. For each page, take its content stream IDs. Skip a stream unless `StreamEncoding::is_plain_decodable()`; decode with `decode_plain_stream`.
  2. `tokenize`; if tokenization fails, skip the stream (`debug!`).
  3. For every `InlineImage` token: parse its dictionary from the tokens between `BI` and `ID` (expand abbreviations for keys and for names, including names inside arrays, e.g. `/F [/AHx /DCT]`), build the image XObject stream, and replace the token's bytes with `/<name> Do`. Wrap the replacement in `q`/`Q` is **not** needed (inline image placement uses the current CTM exactly like `Do`).
  4. Skip inline images whose `/ColorSpace` is a name that is not a device color space (`DeviceGray`, `DeviceRGB`, `DeviceCMYK`, `Indexed` arrays are allowed) because XObjects resolve named color spaces differently.
  5. Write the new content with `store_as_flate`. If deflate fails, leave the stream unchanged (no raw fallback under a Flate filter).
  6. Register XObjects without `unsafe`:
     ```rust
     fn page_resources_for_update(document: &mut Document, page_id: ObjectId) -> AppResult<ObjectId>;
     ```
     - If the page has no own `/Resources`, copy the inherited resources dictionary (from `document.get_page_resources`) into a new indirect object, set the page `/Resources` to a reference to it, and return its ID. This keeps inherited fonts.
     - If `/Resources` is a direct dictionary on the page, move it into a new indirect object and point the page at it.
     - If `/Resources` is a reference, return that ID.
     - Then ensure that resources object has `/XObject`: a direct dictionary, or a reference to a dictionary object. Insert names into whichever holds it.
     - Two-step pattern: compute IDs with immutable borrows first, then mutate. No raw pointers.
  7. Name allocation: `ImInl{n}` with `n` starting above the highest existing `ImInl` index in that resources `/XObject` dictionary.
- Pipeline order (D11): `cleanup → inline_images → dedup → streams → images → merge_content → fonts → metadata`.
- Tests:
  - `inline_images_converts_image_and_registers_xobject` (rendered via `Do`, XObject has `/Subtype /Image`, `/Width`, `/Height`).
  - `inline_images_keeps_inherited_font_resources` (fixture with fonts on `/Pages` only; after conversion the page resources still contain `/Font /F1`).
  - `inline_images_registers_xobject_when_resources_is_indirect_with_direct_xobject_dict`.
  - `inline_images_skips_stream_with_unsupported_filter_chain` (ASCII85 + Flate content stays byte-identical).
  - `inline_images_expands_abbreviated_filter_array`.
  - `inline_images_ignores_bi_inside_literal_string` (`(BI ID EI) Tj` untouched).
  - `pipeline_compresses_images_created_from_inline_images` (engine-level: inline JPEG image ends up as a DCT XObject smaller than before).
- Commit: `fix(pdf): rewrite inline image conversion without unsafe and with correct resources`.

---

### P3. Cleanup and Metadata per Level (C6, C7, N14)

#### P3.1 Cleanup rewrite
- **Files:** `src/pdf/passes/cleanup.rs`, `src/pdf/object_graph.rs`.
- Behavior table (matches `CleanupPolicy` in section 4.3):

| Removal | Low | Medium | High | Extreme |
|---------|:---:|:------:|:----:|:-------:|
| Page `/Thumb` | ✓ | ✓ | ✓ | ✓ |
| JavaScript actions | ✓ | ✓ | ✓ | ✓ |
| `/PieceInfo` (catalog and pages) | ✓ | ✓ | ✓ | ✓ |
| Markup annotations | | | ✓ | ✓ |
| Widget annotations + `/AcroForm` | | | | ✓ |
| `/StructTreeRoot`, `/MarkInfo`, page `/StructParents` | | | ✓ | ✓ |
| `/Names /EmbeddedFiles`, `FileAttachment` annotations | | | ✓ | ✓ |
| `/OutputIntents` | | | | ✓ |
| Named destinations | never | never | never | never |

- Markup annotation subtypes (removable at High+): `Text, FreeText, Line, Square, Circle, Polygon, PolyLine, Highlight, Underline, Squiggly, StrikeOut, Stamp, Caret, Ink, Popup, FileAttachment, Sound, Redact`. Any other subtype (Link, Widget, Screen, Movie, 3D, RichMedia, PrinterMark, TrapNet, Watermark, unknown) is kept.
- Form removal at Extreme: if any field in `/AcroForm /Fields` (recursively through `/Kids`) has `/FT /Sig`, skip form removal and `warn!(reason = "signature field present", "Form removal skipped")`.
- JavaScript removal: remove `/Names /JavaScript`; remove catalog `/OpenAction` if it resolves to an action dict with `/S /JavaScript`; remove `/AA` entries (catalog, pages, annotations) and annotation `/A` entries that resolve to `/S /JavaScript`; for `/Next` chains, remove the whole action entry if any action in the chain is JavaScript.
- **Reference-only removal:** every removal edits the owning dictionary or array (removing a key or an array element). Never call `document.objects.remove`. `prune_objects` at the end of the pipeline deletes what became unreachable. `/Annots` may be a direct array or a reference to an array; handle both.
- Delete `remove_named_destinations` and its tests.
- Return value: the pass returns `0` bytes saved; real savings are reported by the engine from serialized sizes (P6.1).
- Tests (one per table cell, using `PdfFixture`), e.g.:
  - `cleanup_low_keeps_markup_annotations`, `cleanup_high_removes_markup_annotations`, `cleanup_high_keeps_link_and_widget_annotations`, `cleanup_extreme_removes_widgets_and_acroform`, `cleanup_extreme_keeps_form_with_signature_field`, `cleanup_medium_keeps_output_intents`, `cleanup_extreme_removes_output_intents`, `cleanup_never_removes_named_destinations`, `cleanup_removes_javascript_open_action_at_every_level`, `cleanup_removes_shared_thumbnail_reference_without_dangling_references`, and for every level `cleanup_at_<level>_leaves_no_dangling_references_after_prune`.
- Commit: `fix(pdf): apply cleanup per level and remove references instead of objects`.

#### P3.2 Metadata (N14)
- **Files:** `src/pdf/passes/metadata.rs`.
- Strip: remove trailer `/Info`; remove the `/Metadata` key from the catalog and from every dictionary or stream dictionary whose `/Metadata` resolves to a stream with `/Type /Metadata`. Do not delete objects; prune does it.
- Runs only when `options.metadata == MetadataPolicy::FollowLevel && preset.strip_metadata`.
- Tests: `metadata_strip_removes_info_and_catalog_metadata_reference`, `metadata_keep_policy_leaves_document_unchanged`, `metadata_strip_leaves_no_dangling_references`.
- Commit: `fix(pdf): strip metadata by removing references`.

---

### P4. Font Subsetting Rewrite (N11, N12, N13)

- **Files:** `src/pdf/passes/fonts.rs`.
- Delete `deduplicate_fonts` (the generic dedup from P2.4 already merges identical font file streams with identical dictionaries).
- Subset only fonts that meet **all** of:
  1. Font dict `/Subtype /Type0` with `/Encoding /Identity-H` or `/Identity-V`.
  2. Exactly one descendant font with `/Subtype /CIDFontType2`, `/CIDToGIDMap /Identity`, and a `/FontDescriptor /FontFile2`.
  3. `/BaseFont` is not already subset-tagged (`ABCDEF+Name`).
  4. The font is not referenced from `/AcroForm /DR`.
  5. The font program decodes with `decode_plain_stream` (bounded by `limits.max_stream_bytes`).
  Every other font is skipped with `debug!(font = %name, reason = "...", "Font subsetting skipped")`.
- Character collection (N13) uses `content_lexer::tokenize`, never lopdf's parser:
  - Walk page content streams, Form XObjects reached through `Do` (recursively, with a `HashSet<ObjectId>` visited guard), and annotation appearance streams (`/AP` `/N`, `/R`, `/D`, including state sub-dictionaries).
  - Track the current font through `Tf` operands resolved against the resources dictionary in scope (page resources, or the Form XObject's own `/Resources`, falling back to the parent's).
  - Collect 2-byte CIDs from string operands of `Tj`, `'`, `"`, and string elements of `TJ` arrays (`decode_literal_string` / `decode_hex_string`, then big-endian pairs).
  - If any content stream in scope fails to tokenize, skip subsetting for the whole document (`warn!`), because the used-glyph set would be incomplete.
- Subsetting steps per font:
  1. `glyphs = sorted(used CIDs ∪ {0})`. With Identity mapping, CID = old glyph ID.
  2. `let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);` and `subsetter::subset(&font_program, 0, &remapper)`.
  3. Build a new `/CIDToGIDMap` stream: `2 * (max_cid + 1)` bytes, entry `cid` = big-endian `remapper.get(cid).unwrap_or(0)`. Store with `store_as_flate`. Set the descendant font's `/CIDToGIDMap` to a reference to it.
  4. Store the subset program with `store_as_flate`, and set `/Length1` to the uncompressed subset length (required for `FontFile2`).
  5. Prefix a subset tag: six uppercase letters derived deterministically from a hash of `glyphs` (`fn subset_tag(glyphs: &[u16]) -> String`), applied to the Type0 `/BaseFont`, the descendant `/BaseFont`, and the descriptor `/FontName`.
  6. Replace only if the stored program plus the new map is smaller than the original stored program; otherwise leave the font untouched.
- Tests (pure helpers need no font file):
  - `collect_used_cids_reads_tj_and_tj_array_strings`, `collect_used_cids_follows_form_xobjects_once`, `collect_used_cids_includes_annotation_appearance_streams`, `collect_used_cids_is_not_truncated_by_comments`, `build_cid_to_gid_map_maps_old_glyphs_to_new_glyphs`, `subset_tag_is_six_uppercase_letters_and_deterministic`, `font_eligibility_rejects_simple_truetype`, `font_eligibility_rejects_non_identity_encoding`, `font_eligibility_rejects_font_in_acroform_default_resources`.
  - With D9 font file: `subset_fonts_shrinks_font_and_keeps_used_glyph_outlines` (re-parse subset with `subsetter` is not possible; instead assert: subset program smaller, `/Length1` equals inflated length, CIDToGIDMap entry for each used CID is non-zero and distinct, `/BaseFont` has tag). Mark `#[ignore = "requires tests/fixtures/fonts/test-font.ttf"]` if the file is missing.
- Commit: `fix(pdf): subset only identity CID fonts and update CIDToGIDMap`.

---

### P5. Security Hardening (S1–S6)

#### P5.1 Bounded decompression (S1)
- **Files:** `src/codec/zlib.rs`, `src/codec/lzw.rs`, `src/pdf/filters.rs`, `src/pdf/passes/images.rs`.
- `codec/zlib.rs`:
  ```rust
  pub fn inflate_bounded(data: &[u8], max_output_bytes: usize) -> AppResult<Vec<u8>> {
      let mut output = Vec::new();
      let limit = (max_output_bytes as u64).saturating_add(1);
      ZlibDecoder::new(data).take(limit).read_to_end(&mut output)
          .map_err(|error| AppError::Decompress(error.to_string()))?;
      if output.len() > max_output_bytes {
          return Err(AppError::DecompressionLimitExceeded { limit: max_output_bytes });
      }
      Ok(output)
  }
  ```
  Delete the unbounded `inflate`.
- Every decode passes `options.limits.max_stream_bytes`.
- Raw image decoding: before allocating, check `u64::from(width) * u64::from(height) <= limits.max_image_pixels`, else `AppError::ImageTooLarge`. Use checked multiplication for byte sizes.
- Tests: `inflate_bounded_rejects_flate_bomb` (deflate 64 MiB of zeros, limit 1 MiB → `DecompressionLimitExceeded`), `inflate_bounded_accepts_data_at_limit`, `lzw_decode_stops_at_output_limit`, `compress_streams_leaves_bomb_stream_untouched`, `compress_images_rejects_declared_dimensions_above_pixel_limit`.
- Commit: `fix(security): bound decompression and image allocation`.

#### P5.2 mozjpeg panics (S2)
- Already wrapped in P1.2. Add tests: `encode_jpeg_returns_error_for_zero_sized_image`.
- Add integration test `batch_continues_after_corrupt_image_with_continue_on_error` (a `.jpg` file with valid SOI then garbage, plus a valid PDF; assert one `error`, one `ok`). Run it also in release once manually: `cargo test --release --test cli_batch`.
- Commit: `test(security): cover encoder failures in batch`.

#### P5.3 Discovery and output paths (S3)
- **Files:** `src/fs/discovery.rs`, `src/app/output_path.rs`.
- `discovery.rs`:
  ```rust
  pub struct DiscoveryRequest<'a> {
      pub root: &'a Path,
      pub pattern: &'a str,
      pub recursive: bool,
      pub excluded_dir: Option<&'a Path>,
  }
  pub fn discover_files(request: &DiscoveryRequest<'_>) -> AppResult<Vec<PathBuf>>;
  ```
  - `GlobBuilder::new(pattern).case_insensitive(true).build()?.compile_matcher()`.
  - `WalkDir::new(root).follow_links(false).max_depth(if recursive { usize::MAX } else { 1 })`.
  - `filter_entry` skips `excluded_dir` (compare canonicalized paths when the excluded dir exists).
  - Unreadable entries: `warn!(path = ..., error = %e, "Skipping unreadable directory entry")`, not silently dropped.
  - Sorted output.
- `output_path.rs`:
  ```rust
  pub fn batch_output_path(file: &Path, input_dir: &Path, output_dir: &Path, extension: Option<&str>) -> AppResult<PathBuf>;
  ```
  - `strip_prefix` failure → `AppError::PathOutsideInput`. No fallback to `file`.
  - If the result equals `file` → `AppError::OutputEqualsInput`.
- Tests: `discover_files_does_not_follow_symlinks` (`#[cfg(unix)]`, symlink to a file outside root), `discover_files_skips_excluded_output_dir`, `discover_files_matches_pattern_case_insensitively`, `batch_output_path_rejects_file_outside_input_dir`, `batch_output_path_rejects_output_equal_to_input`, `batch_output_path_replaces_extension_when_given`.
- Commit: `fix(security): never follow symlinks or fall back to source paths in batch`.

#### P5.4 Bounded reads and image limits (S4, S6)
- **Files:** `src/fs/reader.rs`, `src/raster/engine.rs`, `src/cli/args.rs`, `src/cli/mapping.rs`.
- `reader.rs`:
  ```rust
  pub fn read_bounded(path: &Path, max_bytes: u64) -> AppResult<Vec<u8>>;
  pub fn parse_pdf(bytes: &[u8]) -> AppResult<lopdf::Document>;
  ```
  - `File::open` directly; `ErrorKind::NotFound` → `AppError::InputNotFound(path)` (S6, no `exists()` pre-check).
  - Compare `metadata.len()` with `max_bytes` before reading → `InputTooLarge`. Read with `take(max_bytes + 1)` and re-check.
  - `parse_pdf` checks `%PDF-` magic, then `Document::load_mem`, mapping errors as today (`EncryptedPdf`, `InvalidPdf`).
- Image decoding:
  ```rust
  let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
  let mut limits = image::Limits::default();
  limits.max_image_width = Some(MAX_IMAGE_SIDE_PIXELS);
  limits.max_image_height = Some(MAX_IMAGE_SIDE_PIXELS);
  limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC_BYTES);
  reader.limits(limits);
  ```
  `image::Limits` is `#[non_exhaustive]`: build it from `default()` and assign fields, never with a struct literal. Constants: `MAX_IMAGE_SIDE_PIXELS = 65_535`, `MAX_IMAGE_DECODE_ALLOC_BYTES = 512 * 1024 * 1024`. Also enforce `limits.max_image_pixels` after reading dimensions.
- CLI: `--max-input-size <BYTES>` on `compress` and `batch`, default `DEFAULT_MAX_INPUT_BYTES`, mapped into `Limits.max_input_bytes`.
- Tests: `read_bounded_returns_input_not_found`, `read_bounded_rejects_file_above_limit`, `read_bounded_reads_file_at_limit`, `parse_pdf_rejects_missing_magic`, `compress_image_rejects_dimensions_above_limit` (PNG header declaring 70 000 × 70 000), CLI `compress_rejects_input_larger_than_max_input_size`.
- Commit: `fix(security): bound input size and image decode limits`.

#### P5.5 Atomic writes (S5)
- **Files:** `src/fs/atomic_writer.rs`.
  ```rust
  pub fn write_atomically(destination: &Path, bytes: &[u8]) -> AppResult<()> {
      let directory = parent_directory(destination);
      std::fs::create_dir_all(directory)?;
      let mut temp_file = tempfile::NamedTempFile::new_in(directory)?;
      temp_file.write_all(bytes)?;
      temp_file.as_file().sync_all()?;
      temp_file.persist(destination).map_err(|error| AppError::Io(error.error))?;
      Ok(())
  }
  ```
  `parent_directory` returns `Path::new(".")` for a bare file name.
- Tests: `write_atomically_creates_file_with_exact_bytes`, `write_atomically_replaces_existing_file`, `write_atomically_creates_parent_directories`, `write_atomically_leaves_no_temp_file_on_success`, `write_atomically_leaves_no_temp_file_when_destination_is_a_directory` (persist fails), `concurrent_write_atomically_to_same_destination_yields_one_complete_file` (two threads, each writes 1 MiB of a distinct byte; result is all-one-byte).
- Commit: `fix(security): atomic writes through unique temp files`.

---

### P6. App Layer, Presentation, Entry Point, Correctness

#### P6.1 Pure PDF engine and never-bigger guard (A5, CC12, C8, C14)
- **Files:** `src/pdf/engine.rs`, `src/pdf/pipeline.rs`, `src/pdf/passes/mod.rs`, `src/app/compress_file.rs`.
- `pipeline.rs`:
  ```rust
  pub trait PdfPass {
      fn name(&self) -> &'static str;
      fn is_enabled(&self, options: &PdfOptions) -> bool;
      fn run(&self, document: &mut Document, options: &PdfOptions) -> AppResult<i64>;
  }

  pub fn standard_pipeline() -> Vec<Box<dyn PdfPass>>;
  ```
  Order: `CleanupPass, InlineImagesPass, DedupPass, StreamsPass, ImagesPass, MergeContentPass, FontsPass, MetadataPass`. `FontsPass::is_enabled` = `options.fonts == FontOptimization::Subset`. `MetadataPass::is_enabled` = `options.metadata == FollowLevel && options.level.preset().strip_metadata`.
- `engine.rs`:
  ```rust
  pub struct CompressedPdf { pub bytes: Vec<u8>, pub passes: Vec<PassStat>, pub kept_original: bool }

  pub fn compress_pdf_bytes(input: &[u8], options: &PdfOptions) -> AppResult<CompressedPdf>;
  ```
  Steps: `parse_pdf(input)` (moved call to `fs::reader::parse_pdf` is an import from `fs`; to respect the dependency rule, move `parse_pdf` into `pdf/engine.rs` itself and keep only `read_bounded` in `fs/reader.rs`), run enabled passes collecting `PassStat`, `prune_objects`, `renumber_objects`, `save_to(&mut Vec)`. If `output.len() >= input.len()`, return the input bytes with `kept_original: true`.
- `app/pdf_compressor.rs` `PdfCompressor::compress` now calls `compress_pdf_bytes`. `app/compress_file.rs` reaches its final media-agnostic shape from section 3.4: `read_bounded` → `detect_media_type` → `registry.find` → `compressor.compress` → unless `WriteMode::DryRun`, `write_atomically`. Report sizes come from actual byte lengths, so dry-run equals the real run (C14).
- Tests: `compress_pdf_bytes_never_returns_larger_output` (already-minimal PDF → `kept_original` and bytes identical), `compress_pdf_bytes_runs_passes_in_standard_order` (names collected from `passes`), `dry_run_reports_same_size_as_real_run` (app level, tempdir), `dry_run_writes_no_file`.
- Commit: `refactor(pdf): pure in-memory engine with pass pipeline and never-bigger guard`.

#### P6.2 Pure image engine (A5, C9, C12, C13)
- **Files:** `src/raster/engine.rs`, `src/raster/transform.rs`, `src/raster/format_resolution.rs`, `src/codec/png.rs`.
- API:
  ```rust
  pub struct EncodedImage {
      pub bytes: Vec<u8>,
      pub output_format: ImageFormat,
      pub original_dimensions: Dimensions,
      pub output_dimensions: Dimensions,
      pub kept_original: bool,
  }

  pub fn compress_image_bytes(input: &[u8], input_format: ImageFormat, options: &ImageOptions) -> AppResult<EncodedImage>;
  ```
- Rules:
  - `input_format.is_decodable()` false → `AppError::UnsupportedFormat(format!("{} input is not supported; supported image formats: JPEG, PNG", name))` (D8).
  - PNG→PNG with no resize and `ColorMode::Preserve`: run `oxipng::optimize_from_memory(input, &Options::from_preset(effort))` on the original bytes (keeps alpha, 16-bit, palette).
  - Otherwise decode, transform, encode. PNG encoding keeps the color type of the transformed `DynamicImage` (`image.write_to(&mut Cursor, image::ImageFormat::Png)`, then oxipng). Grayscale with alpha → `to_luma_alpha8`.
  - JPEG output from an image with alpha: `warn!(reason = "JPEG has no alpha channel", "Flattening transparency onto white")` and composite onto white (`fn flatten_onto_white(image: &DynamicImage) -> RgbImage`).
  - `MetadataPolicy::Keep` → `warn!("Image metadata cannot be preserved; re-encoding strips EXIF and XMP")` (C13).
  - Same format and output not smaller → return input bytes, `kept_original: true`.
  - Lossless + JPEG input + `OutputFormat::Original` → PNG (today's behavior).
- `app/image_compressor.rs` `ImageCompressor::compress` now calls `compress_image_bytes`. Image output is written by the shared `compress_file` flow through `write_atomically` (C12); no image-specific write code remains.
- Tests: `png_to_png_keeps_alpha_channel`, `png_to_png_keeps_16_bit_depth`, `resize_keeps_alpha_for_png_output`, `jpeg_output_flattens_alpha_onto_white` (top-left transparent pixel becomes `[255,255,255]`), `same_format_not_smaller_keeps_original_bytes`, `gif_input_is_rejected_as_unsupported`, `keep_metadata_logs_warning` (use a `tracing` test subscriber capturing to a `Vec<u8>`), `lossless_jpeg_becomes_png`.
- Commit: `fix(image): preserve alpha and bit depth, reject undecodable formats, write atomically`.

#### P6.3 Output paths (C10)
- **Files:** `src/app/output_path.rs`, `src/raster/format_resolution.rs`.
- `fn resolved_output_format(input: ImageFormat, options: &ImageOptions) -> ImageFormat` (single place; Lossless rule included).
- `fn output_extension(input_path: &Path, input_format: ImageFormat, output_format: ImageFormat) -> String`: same format → keep the input's own extension (`.jpeg` stays `.jpeg`); different → `output_format.canonical_extension()`.
- Single file default path: `<stem>_compressed.<extension>`. Batch: `batch_output_path(file, input_dir, output_dir, Some(extension))`.
- Tests: `default_output_keeps_jpeg_extension_spelling`, `format_conversion_changes_extension_in_single_mode`, `format_conversion_changes_extension_in_batch_mode`, `lossless_jpeg_output_uses_png_extension`.
- Commit: `fix(app): derive output extension from the resolved output format`.

#### P6.4 Batch (A4, C11, CC9, CC10)
- **Files:** `src/app/batch.rs`.
- Signature: `pub fn process_batch(request: &BatchRequest, progress: &dyn BatchProgress) -> AppResult<BatchReport>`.
- Steps:
  1. `request.input_dir.is_dir()` else `AppError::InvalidInputDir`.
  2. Resolve output dir (default `<input_dir>_compressed` sibling, current behavior).
  3. Discover with `excluded_dir = Some(output_dir)`.
  4. Build a local pool: `rayon::ThreadPoolBuilder::new().num_threads(request.workers).build()` (0 = rayon default). Map build errors to `AppError::Io(std::io::Error::other(e))`. No `build_global`.
  5. `pool.install(|| files.par_iter().map(|file| process_one(file, ...)).collect::<Vec<FileResult>>())`. No `Mutex`; `collect` preserves input order.
  6. Fail-fast: an `AtomicBool stop_requested`. Before processing a file, if set → `Skipped { reason: "aborted after an earlier failure" }`. After an error with `FailurePolicy::StopOnFirstError` → set it.
  7. `process_one` builds a `CompressRequest` from `request.settings`, `request.write_mode` and the output path, and calls the same `compress_file(&request, registry)` used by single mode. The output path comes from `batch_output_path(file, input_dir, output_dir, Some(&compressor.output_extension(...)))`, where `compressor` is `registry.find(&media_type)` for that file, so batch never branches on media type. Existing output with `WriteMode::Write` → `Skipped { reason: "output already exists" }`.
  8. `BatchReport::from_results`.
- CLI (A4): `batch` gains `--quality`, `--format`, `--max-dimension`, `--grayscale`, `--lossless`, `--max-input-size`, with the same help text as `compress`.
- Exit code: `main` returns 1 when `report.failed > 0`.
- Tests: `process_batch_compresses_all_valid_files`, `process_batch_skips_existing_outputs_without_overwrite`, `process_batch_stop_on_first_error_skips_remaining_files` (workers = 1 for determinism), `process_batch_continue_on_error_processes_every_file`, `process_batch_preserves_directory_structure_recursively`, `process_batch_does_not_reprocess_output_dir_inside_input_dir`, `process_batch_reports_progress_for_every_file` (counting `BatchProgress` impl), `process_batch_rejects_non_directory_input`.
- Commit: `fix(batch): honor continue-on-error, scoped thread pool, shared single-file path`.

#### P6.5 Error model and entry point (CC4, CC5)
- **Files:** `src/error.rs`, `src/main.rs`, all modules still returning `Result<_, String>` or `AppError::Custom`.
- Replace every `Result<_, String>` with `AppResult<_>` and the matching typed variant from section 4.1. Delete `AppError::Custom` and `AppError::WalkDir`.
- `main.rs` final shape (comment-free, ~80 lines):
  ```rust
  fn main() -> ExitCode {
      let cli = Cli::parse();
      init_logging(&cli);
      match run(cli) {
          Ok(code) => code,
          Err(error) => {
              eprintln!("{} {error}", "Error:".red().bold());
              ExitCode::from(error.exit_code())
          }
      }
  }

  fn run(cli: Cli) -> AppResult<ExitCode> {
      match cli.command {
          Commands::Compress(args) => run_compress(args),
          Commands::Batch(args) => run_batch(args, cli.quiet),
          Commands::Info(args) => run_info(args),
      }
  }
  ```
  - No `std::process::exit`.
  - `run_compress`: `compress_file(&args.into(), &CompressorRegistry::standard())`, print JSON or `render_file_report`. `run_batch` passes the same `CompressorRegistry::standard()`. The overwrite guard lives in `compress_file` (`WriteMode::Write` + existing output → `AppError::OutputExists`, exit 2).
  - `run_info`: `info::inspect(&path, limits)` → `render_info_report`. The duplicated error mapping is deleted.
  - `IndicatifProgress` implements `BatchProgress`; use `NoProgress` when `--json` or `--quiet`. `ProgressStyle::with_template(...)` error → fall back to `ProgressStyle::default_bar()` (no unwrap).
- Remove `anyhow` from `Cargo.toml` if nothing uses it anymore (removing a dependency is allowed).
- Tests (integration, `tests/cli_usage.rs`): `compress_existing_output_without_overwrite_exits_with_code_2`, `batch_on_file_instead_of_directory_exits_with_code_2`, `invalid_glob_exits_with_code_2`, `compress_missing_input_exits_with_code_1_and_names_the_file`.
- Commit: `refactor: typed errors and thin main with exit codes`.

#### P6.6 CLI wiring fixes (N1, N2, N3, D4, D10, N4)
- **Files:** `src/cli/args.rs`, `src/cli/mapping.rs`.
- `--quality` sets both `image.quality` and `pdf.jpeg_quality`; `--image-quality` (PDF-specific) wins over `--quality` for PDFs. `--lossless` → `ImageEncodingMode::Lossless` for both (PDF image pass skips lossy JPEG re-encoding of Flate images and DCT re-encoding entirely when Lossless). `--grayscale` → `ColorMode::Grayscale` for both.
- Remove `--strip-metadata` (D4) and `webp` from `FormatArg` (D10).
- Batch default pattern: `*.{pdf,jpg,jpeg,png}` (case-insensitive matching from P5.3) (D8).
- Tests (`cli/mapping.rs`): `quality_applies_to_pdf_images`, `image_quality_overrides_quality_for_pdf`, `lossless_applies_to_pdf`, `grayscale_applies_to_pdf`, `extreme_level_implies_grayscale_without_flag`. Integration: `strip_metadata_flag_is_rejected`, `webp_format_is_rejected`.
- `pdf/passes/images.rs` tests: `lossless_mode_skips_dct_reencoding`, `lossless_mode_uses_only_flate_for_raw_images`.
- Commit: `fix(cli): wire quality, lossless and grayscale to PDF; drop no-op flags`.

#### P6.7 PNG predictor for PDF Flate images (CC6)
- **Files:** `src/codec/png.rs`, `src/pdf/passes/images.rs`.
- `codec/png.rs`:
  ```rust
  pub struct PngIdat { pub zlib_data: Vec<u8>, pub width: u32, pub height: u32, pub bit_depth: u8, pub color_type: u8 }
  pub fn extract_idat(png_bytes: &[u8]) -> AppResult<PngIdat>;
  pub fn pdf_safe_oxipng_options(effort: u8) -> oxipng::Options;
  ```
  - `extract_idat` parses the 8-byte signature, then chunks (`length u32 BE`, `type [u8;4]`, data, CRC). Reads IHDR (width, height, bit depth, color type, interlace). Rejects interlaced images and color types other than 0 (gray) and 2 (RGB). Concatenates all IDAT data.
  - `pdf_safe_oxipng_options` = `Options::from_preset(effort)` with `bit_depth_reduction = false`, `color_type_reduction = false`, `palette_reduction = false`, `grayscale_reduction = false`, `interlace = Some(Interlacing::None)`.
- In the PDF image pass, the lossless candidate is: encode RGB8/L8 PNG, optimize with `pdf_safe_oxipng_options`, `extract_idat`, store as `/Filter /FlateDecode` with `/DecodeParms << /Predictor 15 /Colors <1|3> /BitsPerComponent 8 /Columns <width> >>`. Second lossless candidate: plain deflate of raw pixels. Pick the smallest of (JPEG unless Lossless, PNG-predictor, plain deflate).
- Tests: `extract_idat_reads_dimensions_and_concatenates_chunks`, `extract_idat_rejects_interlaced_png`, `png_predictor_stream_decodes_back_to_original_pixels` (decode with lopdf's `Stream::decompressed_content`, compare to original raw pixels), `images_pass_prefers_smallest_candidate`.
- Commit: `perf(pdf): keep PNG filtering gains with predictor 15 for Flate images`.

---

### P7. No Comments, English Only, Readability

#### P7.1 CLI help without doc comments (N15)
- **Files:** `src/cli/args.rs`.
- Replace every `///` on clap items with attributes:
  ```rust
  #[derive(Debug, Parser)]
  #[command(name = "media-compressor", version, author, about = "Fast media compressor for PDF and image files", propagate_version = true)]
  pub struct Cli { ... }

  #[derive(Debug, Subcommand)]
  pub enum Commands {
      #[command(about = "Compress a PDF or image file (type detected from magic bytes)")]
      Compress(CompressArgs),
      ...
  }

  #[arg(long = "keep-metadata", help = "Preserve document metadata regardless of level")]
  pub keep_metadata: bool,
  ```
  Use `long_help = "..."` where today's doc comment has a second paragraph.
- Test: snapshot that `media-compressor compress --help` output contains every flag with its help line (`tests/cli_usage.rs::compress_help_lists_every_flag_with_description`), same for `batch` and `info`. Write this test **before** removing the doc comments, run it green, then convert.
- Commit: `refactor(cli): move help text from doc comments to clap attributes`.

#### P7.2 Remove all comments
- Run `./scripts/check-no-comments.sh` to list offenders. Remove every comment in `src/`, `tests/`, `benches/`.
- For each removed comment that carried meaning, apply one of:
  - Magic number → named `const` (`PDF_POINTS_PER_INCH`, `ORPHAN_IMAGE_MIN_SIDE_PIXELS`, `MAX_JPEG_QUALITY`, `FLAT_IMAGE_STDDEV_THRESHOLD`, …).
  - Section banner (`// ── 1. Read input ──`) → one function per step.
  - "Why" explanations → a function or variable whose name states the reason (`skip_if_already_subset_tagged`, `ensure_page_owns_resources`).
  - Test explanation → test name.
- Delete `#[allow(unused_variables)]`, `#[allow(dead_code)]` and the code they hide.
- Commit (may be several, one per module group): `refactor: remove comments in favor of self-describing code`.

#### P7.3 English only
- Translate `summary.md` to English (keep structure and IDs).
- Check README, Makefile echo/help text, CLI help, log and error messages, test names. Makefile `#` comments are allowed (the no-comment rule covers `.rs` only) but must be English.
- Commit: `docs: translate review summary to English`.

#### P7.4 Readability sweep and gates
- For every function over ~40 lines or with more than 4 parameters, split or introduce a parameter struct.
- Replace remaining `bool` parameters that select behavior with the enums from section 4.4.
- Wire gates: in `Makefile`, `check: test lint fmt-check no-comments`.
- Commit: `refactor: readability sweep and enforce no-comments in make check`.

---

### P8. Test Completion, Coverage, Documentation

#### P8.1 Unit test completion (T3, T6, T7)
- Run `make coverage`. For each module under 90% lines, add tests until the crate passes. Minimum per module:

| Module | Required tests |
|--------|----------------|
| `detection` | each format, short input (0–3 bytes), unknown input |
| `codec::zlib` | round trip, limit exceeded, corrupt input → `Decompress` |
| `codec::lzw` | spec vector, width boundaries for both early-change modes, CLEAR, invalid code, limit |
| `codec::jpeg` | RGB/gray, progressive/baseline, zero-size error |
| `codec::png` | RGB/gray/RGBA/16-bit encode, IDAT extraction, interlaced rejection |
| `pdf::content_lexer` | section P2.1 list |
| `pdf::content_minifier` | section P2.2 list |
| `pdf::filters` | section P2.3 list |
| `pdf::object_graph` | `content_stream_ids` (single, array, forms), `resolve` through reference chains |
| `pdf::passes::*` | sections P2–P4 lists, plus `merge_content_preserves_operation_order`, `merge_content_skips_page_with_undecodable_stream`, `images_downsample_by_page_dpi`, `images_downsample_orphan_by_pixel_budget`, `images_quality_adjusts_for_flat_and_complex_images`, `images_skip_unsupported_color_space`, `images_convert_cmyk_to_rgb` |
| `pdf::engine` / `pipeline` | order, enabled flags, never-bigger, stats |
| `raster::*` | P6.2 list, resize bounds and aspect ratio, grayscale, format resolution matrix (every `OutputFormat` × decodable input × encoding mode) |
| `fs::*` | P5.3–P5.5 lists |
| `app::*` | P6.1–P6.5 lists, `info_reports_page_count_and_version` |
| `presentation` | per renderer: with `colored::control::set_override(false)`, assert exact lines (e.g. contains `"Reduction:"` and `"50.0%"`), dry-run variant, negative savings variant; JSON: exact values |
| `cli::mapping` | P6.6 list, every flag of every subcommand |

- Replace every remaining `*_does_not_panic` test and bare `is_ok()` assertion with value assertions (T6).
- Image tests use gradients, RGBA with transparency and 16-bit images from `image_fixture.rs`, never `new_rgb8` black images (T7).
- Commit: `test: complete unit coverage per module`.

#### P8.2 Integration tests (T1, T4, T8)
- Split `tests/integration_test.rs` into the files of section 3.2 and add:
  - `cli_compress_pdf.rs`: happy path at each level (output loads, same page count, size ≤ input), `--dry-run` writes nothing, `--overwrite` behavior, `--json` exact keys.
  - `cli_compress_image.rs`: existing image tests (moved), `--format png` on JPEG gives `.png`, transparent PNG keeps alpha.
  - `cli_batch.rs`: mixed valid/invalid with and without `--continue-on-error` (exit code 1 either way, but skipped vs processed counts differ), `--recursive` keeps structure, JSON schema, symlink not followed (`#[cfg(unix)]`).
  - `cli_info.rs`: text and `--json` happy path.
  - `cli_usage.rs`: help/version/usage errors (moved) plus P6.5 and P7.1 tests.
- Commit: `test: end-to-end CLI coverage for compress, batch and info`.

#### P8.3 Coverage gate
- `Makefile`: `check: test lint fmt-check no-comments coverage`.
- Commit: `build: enforce 90% line coverage in make check`.

#### P8.4 Documentation (A6)
- `CLAUDE.md`:
  - Replace "Architecture" pipeline list with the new pass order (D11) and module paths.
  - Replace "Module Layout" with section 3.2 tree.
  - Replace "Compression Levels" table with section 4.3 preset table plus the P3.1 cleanup table.
  - Update rules that mention old paths (`pdf_utils.rs` → `codec::zlib` and `pdf::filters`; `stream.rs`/`image.rs`/`inline.rs` → new paths).
  - Add: "Never use `lopdf::content::Content::decode`; use `pdf::content_lexer`."
  - Add: "Media types are handled by `MediaCompressor` strategies registered in `CompressorRegistry::standard()`. To support a new media type, add a detection rule in `detection`, a strategy in `app/`, and register it. Never add `match MediaType` branches in `compress_file` or `batch`."
- `README.md`: flags (new `--max-input-size`, batch image flags; removed `--strip-metadata`, `--format webp`), supported formats (JPEG, PNG, PDF), level table, exit codes (0 ok, 1 runtime error or failed batch files, 2 usage error), and a "Behavior changes" section listing D3, D4, D8, D10, D11, C10, C11.
- `Makefile`: remove `STRIP_META`, `FORMAT=webp`, update batch default pattern text, help text.
- `summary.md`: append a "Resolution" column or section mapping every finding to its commit.
- Commit: `docs: update CLAUDE.md, README and Makefile for the new architecture`.

---

## 6. Findings Traceability

| Finding | Task | | Finding | Task | | Finding | Task |
|---------|------|-|---------|------|-|---------|------|
| S1 | P5.1 | | C11 | P6.4 | | A1 | P1.7 (D12, section 3.4) |
| S2 | P0.2, P1.2, P5.2 | | C12 | P6.2 | | A2 | P1.1 |
| S3 | P5.3 | | C13 | P6.2 | | A3 | P1.1 |
| S4 | P5.4 | | C14 | P6.1 | | A4 | P6.4 |
| S5 | P0.2, P5.5 | | CC1 | P1.2 | | A5 | P6.1, P6.2 |
| S6 | P5.4 | | CC2 | P1.3 | | A6 | P8.4 |
| C1 | P2.1, P2.2 | | CC3 | P1.2, P2.5 | | T1 | P0.4, P8.2 |
| C2 | P2.1, P2.2 | | CC4 | P6.5 | | T2 | P2.2, P2.3 |
| C3 | P2.3 | | CC5 | P6.5 | | T3 | P8.1 |
| C4 | P2.3 | | CC6 | P6.7 | | T4 | P6.4, P8.2 |
| C5 | P2.3 | | CC7 | P1.1 (options), P6.1, P6.2 | | T5 | P2.3, P5.1 |
| C6 | P3.1 | | CC8 | P1.1 (`LevelPreset`) | | T6 | P8.1 |
| C7 | P3.1 | | CC9 | P6.4 | | T7 | P8.1 |
| C8 | P6.1 | | CC10 | P6.4, P6.5 | | T8 | P8.2 |
| C9 | P6.2 | | CC11 | P1.6 | | fmt | P0.1 |
| C10 | P6.3 | | CC12 | P6.1 | | N1–N3 | P6.6 |
| N4 | P6.2, P6.6 | | N5 | P2.3 | | N6 | P2.4 |
| N7–N10 | P2.5 | | N11–N13 | P4 | | N14 | P3.2 |
| N15 | P7.1 | | N16 | P0.5 | | | |

---

## 7. Risks and Stop Conditions

Stop and ask the user when:
- A test written from this plan cannot pass without changing behavior that is not listed in the plan.
- A lopdf, image, oxipng, subsetter or tempfile API differs from the signatures used here (versions: lopdf 0.34.0, image 0.25.10, oxipng 9.1.5, subsetter 0.2.3, tempfile 3.27.0, flate2 1.1.9).
- Coverage cannot reach 90% without testing `main.rs` (main is excluded from `--lib` coverage on purpose; do not lower the threshold yourself).
- The D9 font file is missing and the user has not answered.

Known risks:
- The content lexer must never be more permissive than the PDF spec. On any doubt it returns an error and the stream stays untouched.
- `git mv` in P1 must happen in commits separate from content edits, so `git log --follow` keeps history.
- Behavior changes visible to users: D3, D4, D8, D10, D11, C10 (extensions), C11 (fail-fast batch). All are listed in the README in P8.4.
