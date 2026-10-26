# media-compressor

Fast media compressor written in Rust. Compresses PDF and image files by recompressing images (mozjpeg + oxipng), optimizing streams, deduplicating objects, subsetting fonts, and stripping unnecessary metadata. Auto-detects file type from magic bytes — no extension guessing.

## Features

### PDF Compression
- **4 compression levels** — low, medium, high, extreme with tunable presets
- **Image re-encoding** — mozjpeg (progressive JPEG) and oxipng for embedded images
- **DPI downsampling** — page-aware DPI calculation with configurable targets
- **Complexity-aware quality** — adjusts JPEG quality based on image content (flat vs photographic)
- **Stream optimization** — zlib recompression, LZW-to-Flate conversion, content stream merging, whitespace compaction
- **Object cleanup** — remove thumbnails, JavaScript, non-link annotations, output intents, piece info, named destinations
- **Stream deduplication** — find byte-identical streams and rewrite references
- **Font optimization** — deduplicate identical font file streams, TrueType/OpenType glyph subsetting via content stream analysis (opt-in)
- **Inline image conversion** — convert BI/ID/EI inline images to XObject references
- **Metadata stripping** — remove /Info dictionary and XMP metadata streams
- **Structure tree removal** — strip /StructTreeRoot, /MarkInfo at high/extreme levels
- **AcroForm removal** — strip interactive form fields at extreme level
- **Embedded file removal** — strip attached files at high/extreme levels
- **Atomic writes** — write to temp file, then rename; clean up on failure

### Image Compression
- **Format detection** — JPEG, PNG, GIF, TIFF, BMP, WebP (via magic bytes)
- **Format conversion** — convert between JPEG, PNG (WebP output planned)
- **Quality control** — configurable encoding quality (1-100)
- **Lossless mode** — PNG/WebP lossless encode; JPEG input auto-converted to PNG
- **Resize** — max-dimension constraint with aspect ratio preserved (Lanczos3)
- **Grayscale conversion** — single-channel luminance output
- **Progressive JPEG** — enabled by default via mozjpeg
- **PNG optimization** — oxipng with effort levels 2-6 based on compression level
- **EXIF/XMP stripping** — metadata automatically removed during re-encoding

### Batch Processing
- **Parallel compression** — rayon-based with configurable worker count
- **Recursive traversal** — walkdir with glob pattern filtering
- **Mixed media** — PDF and images in the same batch, auto-dispatched
- **Resilient** — continue-on-error mode for large batches
- **Progress indicator** — indicatif progress bars
- **Default pattern** — `*.{pdf,jpg,jpeg,png,webp,tif,tiff,bmp,gif}`

### General
- **Auto-detection** — file type identified from magic bytes, not extension
- **Dry-run** — estimate compression savings without writing output
- **JSON output** — structured reports for scripting and automation
- **Cross-platform** — Linux, macOS, Windows

## Installation

Requires Rust 1.75+ and a C compiler (for mozjpeg).

```bash
# From source
cargo install --path .

# Or build manually
cargo build --release
# Binary at: target/release/media-compressor
```

## Usage

### Compress a single file

```bash
# Auto-detect file type, default (medium) level
media-compressor compress input.pdf
media-compressor compress photo.jpg

# Custom output, level, and quality
media-compressor compress input.pdf -o output.pdf -l high
media-compressor compress input.pdf --image-quality 50 --dpi 120

# Image with format conversion and resize
media-compressor compress photo.png -f jpeg --quality 80 --max-dimension 1920

# Lossless mode (skip lossy re-encoding)
media-compressor compress input.pdf --lossless
media-compressor compress photo.jpg --lossless  # converts JPEG → PNG

# Grayscale conversion
media-compressor compress photo.jpg --grayscale

# Force metadata stripping
media-compressor compress input.pdf --strip-metadata

# Enable font optimization (PDF only)
media-compressor compress input.pdf --compress-fonts

# Dry run — estimate savings without writing
media-compressor compress input.pdf --dry-run

# JSON output for scripting
media-compressor compress input.pdf --json
```

### Batch compress a directory

```bash
# Compress all supported files in a folder
media-compressor batch ./documents -l medium

# Recursive with custom output directory
media-compressor batch ./documents -r -o ./compressed -l high

# Custom glob pattern and worker count
media-compressor batch ./docs -p "report_*.pdf" -w 4 --continue-on-error

# Batch with font optimization for PDFs
media-compressor batch ./docs -l high --compress-fonts --overwrite

# Batch dry run with JSON output
media-compressor batch ./documents --dry-run --json
```

### Show PDF info

```bash
media-compressor info document.pdf
media-compressor info document.pdf --json
```

## Compression Levels

| Level | Image Quality | DPI Target | Strip Metadata | Grayscale | Remove Struct Tree | Remove AcroForm | Remove Embedded Files |
|-------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `low` | 90% | none | no | no | no | no | no |
| `medium` (default) | 75% | 200 | no | no | no | no | no |
| `high` | 60% | 150 | yes | no | yes | no | yes |
| `extreme` | 40% | 96 | yes | yes | yes | yes | yes |

All levels perform: stream recompression, object deduplication, thumbnail/JS/annotation removal, inline image conversion, content stream optimization.

## CLI Reference

```
media-compressor [OPTIONS] <COMMAND>

Commands:
  compress    Compress a single file (auto-detects PDF or image)
  batch       Compress all supported files in a directory
  info        Display metadata and size information about a PDF file

Global options:
  -v, --verbose    Enable verbose logging
  -q, --quiet      Suppress all output except errors
```

### `compress` options

| Flag | Description |
|---|---|
| `INPUT` | Input file path (positional, required) |
| `-o, --output <PATH>` | Output file path (default: `<input>_compressed.<ext>`) |
| `-l, --level <LEVEL>` | Compression level: `low`, `medium`, `high`, `extreme` |
| `--keep-metadata` | Preserve document metadata regardless of level |
| `--strip-metadata` | Force metadata stripping (conflicts with `--keep-metadata`) |
| `--image-quality <1-100>` | Override JPEG quality for PDF embedded images |
| `--quality <1-100>` | Override encoding quality (images and PDF; takes precedence over `--image-quality`) |
| `--dpi <NUM>` | Override target DPI for image downsampling |
| `-f, --format <FMT>` | Output format for images: `original`, `jpeg`, `png`, `webp` |
| `--max-dimension <PX>` | Resize so no dimension exceeds PX (aspect ratio preserved) |
| `--grayscale` | Convert to grayscale |
| `--lossless` | Lossless mode (skip lossy recompression; JPEG → PNG) |
| `--compress-fonts` | Enable font dedup + subsetting (PDF only, opt-in) |
| `--dry-run` | Estimate savings without writing output |
| `--json` | Output report as JSON |
| `--overwrite` | Overwrite existing output file |

### `batch` options

| Flag | Description |
|---|---|
| `INPUT_DIR` | Input directory path (positional, required) |
| `-o, --output-dir <PATH>` | Output directory (default: `<input>_compressed`) |
| `-l, --level <LEVEL>` | Compression level: `low`, `medium`, `high`, `extreme` |
| `-p, --pattern <GLOB>` | Glob pattern (default: `*.{pdf,jpg,jpeg,png,webp,tif,tiff,bmp,gif}`) |
| `-r, --recursive` | Recurse into subdirectories |
| `-w, --workers <NUM>` | Parallel workers (0 = auto-detect CPU count) |
| `--continue-on-error` | Skip failed files, continue batch |
| `--keep-metadata` | Preserve document metadata |
| `--image-quality <1-100>` | Override JPEG quality |
| `--dpi <NUM>` | Override target DPI |
| `--compress-fonts` | Enable font dedup + subsetting (PDF only) |
| `--dry-run` | Estimate savings without writing |
| `--json` | Output report as JSON |
| `--overwrite` | Overwrite existing outputs |

### `info` options

| Flag | Description |
|---|---|
| `INPUT` | PDF file path (positional, required) |
| `--json` | Output as JSON |

## Supported File Types

| Format | Detected By | Compress | Batch | Info |
|--------|-------------|:---:|:---:|:---:|
| PDF | `%PDF` header | yes | yes | yes |
| JPEG | `FF D8 FF` magic | yes | yes | - |
| PNG | `89 50 4E 47` magic | yes | yes | - |
| GIF | `GIF87a`/`GIF89a` magic | yes | yes | - |
| TIFF | `49 49 2A 00` / `4D 4D 00 2A` magic | yes | yes | - |
| BMP | `42 4D` magic | yes | yes | - |
| WebP | `RIFF`+`WEBP` magic | yes | yes | - |

## Architecture

### PDF Compression Pipeline

The compression pipeline runs in strict order (each step is isolated in its own module):

1. **Load PDF** — validate magic bytes, parse with lopdf
2. **Cleanup** — remove thumbnails, JavaScript, annotations, output intents, piece info, named destinations, struct tree, AcroForm, embedded files (level-dependent)
3. **Stream dedup** — find byte-identical streams, rewrite references
4. **Stream compression** — decompress + re-compress with zlib (LZW → Flate conversion)
5. **Image compression** — DPI downsampling, complexity-aware quality, mozjpeg/oxipng re-encoding
6. **Content stream merging** — merge multi-stream pages into single stream
7. **Font dedup** (opt-in) — deduplicate identical font file streams
8. **Font subsetting** (opt-in) — parse content streams for used glyphs, strip unused via `subsetter`
9. **Inline image conversion** — convert BI/ID/EI to XObject references
10. **Metadata stripping** — remove /Info and XMP (level-dependent)
11. **Prune & renumber** — remove unreferenced objects, compact numbering
12. **Save** — atomic write via temp file + rename

### Image Compression Pipeline

1. **Detect format** — magic byte identification
2. **Decode** — load to DynamicImage
3. **Transform** — resize (Lanczos3), grayscale conversion
4. **Encode** — mozjpeg (JPEG) or oxipng (PNG) with quality/effort settings
5. **Smart replace** — only write if result is smaller (same-format mode)

### Module Layout

```
src/
  main.rs                    # Entry point, command dispatch
  lib.rs                     # Module declarations
  error.rs                   # AppError enum (thiserror)
  utils.rs                   # Shared helpers
  cli/args.rs                # Clap derive structs
  strategy/
    mod.rs                   # CompressorStrategy trait, MediaType, ImageFormat enums
    resolver.rs              # FileTypeResolver (magic bytes detection)
    dispatcher.rs            # CompressorDispatcher (routes to correct engine)
  compressor/
    engine.rs                # PDF CompressorEngine — orchestrates pipeline
    level.rs                 # CompressionLevel enum with preset methods
    stream.rs                # FlateDecode/LZW decompression, zlib re-compression
    image.rs                 # Embedded image compression (mozjpeg, oxipng, DPI)
    cleanup.rs               # Remove unnecessary PDF objects
    dedup.rs                 # Stream deduplication
    font.rs                  # Font dedup + TrueType/OpenType subsetting
    inline.rs                # Inline image → XObject conversion
    metadata.rs              # Metadata stripping
    pdf_utils.rs             # Shared: zlib helpers, constants, filter extraction
  image_compressor/
    engine.rs                # ImageCompressorEngine — orchestrates image pipeline
    format.rs                # Format detection and resolution
    jpeg.rs                  # JPEG encoding via mozjpeg
    png.rs                   # PNG encoding via oxipng
    transform.rs             # Resize, grayscale transforms
    metadata.rs              # EXIF/XMP metadata handling
  io/
    reader.rs                # PDF loading + validation
    writer.rs                # Atomic PDF saving
  batch/processor.rs         # Parallel batch processing (rayon)
  stats/report.rs            # CompressionReport, BatchReport, JSON output
```

## Development

```bash
cargo build              # Build dev binary
cargo test               # Run all tests
cargo clippy -- -D warnings  # Lint (zero warnings policy)
cargo fmt                # Format code
make check               # All of the above
```

### Benchmarks

Uses [criterion](https://docs.rs/criterion) for statistical benchmarking.

```bash
cargo bench                    # Run all benchmarks
# HTML report: target/criterion/report/index.html
```

Benchmarks cover:
- **Compression levels** — low, medium, high, extreme compared side-by-side
- **Dry-run overhead** — processing cost without file I/O
- **Stream-only** — low level (minimal image work) to isolate stream compression cost

## License

MIT
