# CLAUDE.md — media-compressor

## Project Overview

A Rust CLI tool for compressing PDF and image files. Supports single-file and batch compression with four compression levels (Low/Medium/High/Extreme). Uses `lopdf` for PDF manipulation, `mozjpeg`/`oxipng` for image re-encoding, and `flate2` for stream compression. Only JPEG and PNG images are decodable; other image formats are rejected.

## Build & Run

```bash
make build            # Debug build
make build-release    # Optimized release build (LTO, strip)
make run FILE=input.pdf LEVEL=medium
make batch DIR=./pdfs LEVEL=high
make dry-run FILE=input.pdf
```

## Test & Lint

```bash
make test             # cargo test --all-features
make lint             # clippy -D warnings
make fmt-check        # rustfmt check
make check            # test + lint + fmt-check + no-comments (run before committing)
make coverage         # cargo llvm-cov, fails under 90% line coverage
make bench            # criterion benchmarks
```

## Architecture

Single-file flow (`src/app/compress_file.rs`): bounded read → `detect_media_type` → `CompressorRegistry::find` → `MediaCompressor::compress` (bytes in, bytes out) → atomic write unless dry run. `app::compress_file` and `app::batch` never branch on media type.

The PDF pipeline is a list of `PdfPass` values in `src/pdf/pipeline.rs`, run in this order:

1. **Cleanup** (`pdf/passes/cleanup.rs`) — per level, removes references only
2. **Inline images** (`inline_images.rs`) — BI/ID/EI to XObjects, so later passes compress them
3. **Stream dedup** (`dedup.rs`) — key is stream dictionary plus bytes
4. **Stream compression** (`streams.rs`) — tokenizer-based content minification, re-deflate plain Flate/LZW streams
5. **Image compression** (`images.rs`) — DPI downsampling, mozjpeg / PNG-predictor / plain Flate, smallest wins
6. **Content stream merging** (`merge_content.rs`)
7. **Font subsetting** (`fonts.rs`, opt-in) — identity CID TrueType fonts only, usage from `pdf/font_usage.rs`
8. **Metadata stripping** (`metadata.rs`)

Then prune, renumber, serialize. If the output is not smaller than the input, the original bytes are returned.

Adding a media type: implement `MediaCompressor` in `src/app/` and register it in `CompressorRegistry::standard()`. Do not touch `compress_file` or `batch`.

## Module Layout

```
src/
  main.rs              # Entry point, exit codes
  lib.rs               # Module declarations
  error.rs             # AppError (thiserror), AppResult, exit codes
  domain/              # media types, levels, options, reports (no I/O, no clap)
  detection/           # magic-byte detection, one table
  codec/               # zlib (bounded inflate), lzw, jpeg (mozjpeg), png (oxipng)
  pdf/                 # engine, pipeline, content_lexer, content_minifier, filters,
                       # font_usage, object_graph, passes/*
  raster/              # image engine, transforms, format resolution
  fs/                  # read_bounded, atomic_writer, discovery
  app/                 # strategy trait, registry, per-media compressors,
                       # compress_file, batch, info, output_path, progress
  presentation/        # pretty and JSON rendering
  cli/                 # clap args, request mapping, indicatif progress
  test_support/        # test-only fixture builders (PdfFixture, image fixtures)
tests/                 # assert_cmd integration tests
benches/               # criterion, builds its own sample PDF
scripts/               # check-no-comments.sh
```

## Code Conventions

- **Error handling**: All functions return `AppResult<T>`. Use `?` for propagation. Use `thiserror` for enum variants; `main.rs` prints the error and exits with `AppError::exit_code()`.
- **Logging**: Use `tracing` macros (`info!`, `debug!`, `warn!`). Always use structured fields: `warn!(object_id = ?id, error = %e, "message")`.
- **Formatting**: `rustfmt.toml` — edition 2021, max_width 100. Run `make fmt` before committing.
- **Linting**: `clippy.toml` — MSRV 1.75.0. Zero warnings policy (`-D warnings`).
- **Borrow pattern**: When iterating `doc.objects` mutably, collect IDs first into `Vec<ObjectId>` to avoid simultaneous borrow conflicts.
- **Filter handling**: Image XObjects (Subtype=Image) are processed only by `passes/images.rs`; `passes/streams.rs` skips them. Only `StreamEncoding::is_plain_decodable` streams (none, one Flate, one LZW, no predictor) are rewritten. Other filters (JBIG2Decode, CCITTFaxDecode, JPXDecode, ASCII85, chains) are left untouched with a `warn!`.
- **Content streams**: never use `lopdf::content::Content::decode` (it silently stops at `%` comments). Use `pdf::content_lexer::tokenize`.
- **Atomic writes**: Always write to temp file first, then rename. Clean up temp on failure.
- **Tests**: Unit tests in `#[cfg(test)] mod tests` within each module. Integration tests in `tests/`. Use `assert_cmd` + `predicates` for CLI tests.

## Compression Levels

| Level    | JPEG Quality | DPI Target | Strip Metadata | Grayscale | Markup Annots / Struct Tree / Embedded Files | Forms / Output Intents |
|----------|:---:|:---:|:---:|:---:|:---:|:---:|
| Low      | 90  | None | No  | No  | Keep   | Keep   |
| Medium   | 75  | 200  | No  | No  | Keep   | Keep   |
| High     | 60  | 150  | Yes | No  | Remove | Keep   |
| Extreme  | 40  | 96   | Yes | Yes | Remove | Remove |

Thumbnails, JavaScript and piece info are removed at every level. Named destinations are never removed. Forms are kept when a signature field exists.

## Rules

- Do NOT add new dependencies without explicit approval. The dependency set is intentionally minimal.
- Do NOT modify the pass order in `pdf/pipeline.rs` without understanding cascading effects (cleanup first reduces object count, inline images before dedup and image compression).
- Cleanup removes references only and never calls `doc.objects.remove`; `prune_objects` deletes what becomes unreachable.
- Always run `make check` before considering work complete.
- When adding new PDF filter support, update `pdf/filters.rs` (`StreamEncoding`, `decode_plain_stream`) and the callers in `passes/images.rs` / `passes/inline_images.rs` as appropriate.
- Never silently discard PDF data — unsupported filters must log a warning and leave the stream untouched.
- Batch mode must be resilient: individual file failures should not abort the batch (controlled by `--continue-on-error`).
- Image compression must only replace if the result is smaller — never make a PDF bigger.
- Keep `codec/` as the single source of truth for zlib, LZW, JPEG and PNG encoding. Do not duplicate compression logic across modules.

### Language

- Always use English: source code identifiers, log and error messages, CLI help text, tests, documentation (README, markdown files), commit messages, PR descriptions, and responses.

### Clean Architecture

- Dependencies point inward: `main` → `cli` → `app` → `pdf`/`raster`/`fs`/`detection` → `codec` → `domain`. Inner modules must never import outer ones. Only `cli` imports `clap`; `pdf`, `raster` and `domain` never use `colored`, `indicatif`, or `println!`.
- Keep pure transformation logic separate from I/O. Engines operate on in-memory data (`lopdf::Document`, `DynamicImage`, byte buffers); file reading and writing stay in `fs/` and `app/`.
- Presentation (pretty printing, JSON formatting) belongs in `presentation/`, not in engines, `app/`, or `main.rs`.
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
