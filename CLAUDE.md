# CLAUDE.md — media-compressor

## Project Overview

A Rust CLI tool for compressing PDF and image files. Supports single-file and batch compression with four compression levels (Low/Medium/High/Extreme). Uses `lopdf` for PDF manipulation, `mozjpeg`/`oxipng` for image re-encoding, and `flate2` for stream compression.

## Build & Run

```bash
make build            # Debug build
make build-release    # Optimized release build (LTO, strip, panic=abort)
make run FILE=input.pdf LEVEL=medium
make batch DIR=./pdfs LEVEL=high
make dry-run FILE=input.pdf
```

## Test & Lint

```bash
make test             # cargo test --all-features
make lint             # clippy -D warnings
make fmt-check        # rustfmt check
make check            # test + lint + fmt-check (run before committing)
make bench            # criterion benchmarks
```

## Architecture

The compression pipeline runs in this order (defined in `src/compressor/engine.rs`):

1. **Load PDF** (`src/io/reader.rs`) — validate magic bytes, parse with lopdf
2. **Cleanup** (`src/compressor/cleanup.rs`) — remove thumbnails, JS, annotations, etc.
3. **Stream dedup** (`src/compressor/dedup.rs`) — find byte-identical streams, rewrite refs
4. **Stream compression** (`src/compressor/stream.rs`) — decompress + re-compress with zlib
5. **Image compression** (`src/compressor/image.rs`) — DPI downsampling, quality adjustment, mozjpeg/oxipng re-encoding
6. **Content stream merging** (`src/compressor/stream.rs`) — merge multi-stream pages
7. **Font dedup & subsetting** (`src/compressor/font.rs`) — deduplicate fonts, strip unused glyphs via `subsetter`
8. **Inline image conversion** (`src/compressor/inline.rs`) — convert BI/ID/EI to XObjects
9. **Metadata stripping** (`src/compressor/metadata.rs`) — remove /Info and XMP
10. **Prune & renumber** (lopdf) — remove unreferenced objects
11. **Save** (`src/io/writer.rs`) — atomic write via temp file + rename

Each step is isolated in its own module. The engine orchestrates.

## Module Layout

```
src/
  main.rs              # Entry point, command dispatch
  lib.rs               # Module declarations
  error.rs             # AppError enum (thiserror), AppResult<T> alias
  utils.rs             # Shared helpers (default_output_path)
  cli/args.rs          # Clap derive structs
  compressor/
    engine.rs          # CompressorEngine — orchestrates pipeline
    level.rs           # CompressionLevel enum with preset methods
    stream.rs          # FlateDecode/LZW decompression, zlib re-compression
    image.rs           # Image XObject compression (mozjpeg, oxipng, DPI)
    cleanup.rs         # Remove unnecessary PDF objects
    dedup.rs           # Stream deduplication
    font.rs            # Font dedup + TrueType/OpenType subsetting
    inline.rs          # Inline image → XObject conversion
    metadata.rs        # Metadata stripping
    pdf_utils.rs       # Shared: zlib helpers, constants, filter extraction
  io/
    reader.rs          # PDF loading + validation
    writer.rs          # Atomic PDF saving
  batch/processor.rs   # Parallel batch processing (rayon)
  stats/report.rs      # CompressionReport, BatchReport, JSON output
```

## Code Conventions

- **Error handling**: All functions return `AppResult<T>`. Use `?` for propagation. Use `thiserror` for enum variants, `anyhow::Context` only in `main.rs`.
- **Logging**: Use `tracing` macros (`info!`, `debug!`, `warn!`). Always use structured fields: `warn!(object_id = ?id, error = %e, "message")`.
- **Formatting**: `rustfmt.toml` — edition 2021, max_width 100. Run `make fmt` before committing.
- **Linting**: `clippy.toml` — MSRV 1.75.0. Zero warnings policy (`-D warnings`).
- **Borrow pattern**: When iterating `doc.objects` mutably, collect IDs first into `Vec<ObjectId>` to avoid simultaneous borrow conflicts.
- **Filter handling**: Image XObjects (Subtype=Image) are processed only by `image.rs`. `stream.rs` must skip them. Other unsupported filters (JBIG2Decode, CCITTFaxDecode, JPXDecode) are left untouched with a `warn!`.
- **Atomic writes**: Always write to temp file first, then rename. Clean up temp on failure.
- **Tests**: Unit tests in `#[cfg(test)] mod tests` within each module. Integration tests in `tests/`. Use `assert_cmd` + `predicates` for CLI tests.

## Compression Levels

| Level    | JPEG Quality | DPI Target | Strip Metadata | Grayscale | Remove Struct Tree | Remove AcroForm |
|----------|:---:|:---:|:---:|:---:|:---:|:---:|
| Low      | 90  | None | No  | No  | No  | No  |
| Medium   | 75  | 200  | No  | No  | No  | No  |
| High     | 60  | 150  | Yes | No  | Yes | No  |
| Extreme  | 40  | 96   | Yes | Yes | Yes | Yes |

## Rules

- Do NOT add new dependencies without explicit approval. The dependency set is intentionally minimal.
- Do NOT modify the compression pipeline order without understanding cascading effects — the order is optimized (cleanup first reduces object count for later passes).
- Always run `make check` before considering work complete.
- When adding new PDF filter support, update both `decompress_stream()` in `stream.rs` AND the filter handling in `image.rs` / `inline.rs` as appropriate.
- Never silently discard PDF data — unsupported filters must log a warning and leave the stream untouched.
- Batch mode must be resilient: individual file failures should not abort the batch (controlled by `--continue-on-error`).
- Image compression must only replace if the result is smaller — never make a PDF bigger.
- Keep `pdf_utils.rs` as the single source of truth for shared constants and zlib helpers. Do not duplicate compression logic across modules.

### Language

- Always use English: source code identifiers, log and error messages, CLI help text, tests, documentation (README, markdown files), commit messages, PR descriptions, and responses.

### Clean Architecture

- Dependencies point inward: `cli`/`main.rs` → `strategy`/`batch` → engines (`compressor`, `image_compressor`) → domain types. Inner modules must never import outer ones (no `clap`, `colored`, or `println!` in engines or domain types).
- Keep pure transformation logic separate from I/O. Engines operate on in-memory data (`lopdf::Document`, `DynamicImage`, byte buffers); file reading and writing stay in `io/` or thin wrappers.
- Presentation (pretty printing, JSON formatting) belongs in `stats/`, not in engines, `batch/`, or `main.rs`.
- One responsibility per module and per function. Split functions that grow beyond a single clear purpose.
- No duplicated logic: shared encoders, detectors, and helpers live in exactly one module and are reused.
- Do not add abstractions (traits, generics) that nothing uses. Every trait must have a real caller.

### Unit Tests

- Every new or changed function with logic must ship with unit tests in the same change, in `#[cfg(test)] mod tests` of its module.
- Every bug fix must include a regression test that fails without the fix.
- Cover the happy path, edge cases, and error paths. Assert concrete outputs; `is_ok()` or "does not panic" alone is not enough.
- New CLI behavior requires an integration test in `tests/` using `assert_cmd` + `predicates`.
- Work is not complete until `make check` passes.

### No Comments in Source Code

- Do not write comments in source code (`//`, `/* */`, `///`, `//!`). Make the code explain itself instead.
- Use descriptive names for functions, variables, types, and constants instead of explaining them in a comment.
- Extract complex expressions or steps into well-named functions or variables rather than annotating them.
- Replace magic numbers with named constants.
- When touching existing code, remove comments in the edited area and refactor for readability where the comment was needed.
