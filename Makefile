.PHONY: build build-release run batch dry-run info test test-verbose bench lint fmt-check fmt check clean install help

# Default target
all: build

# ─── Build ────────────────────────────────────────────────────────────────────

## Build development binary
build:
	cargo build

## Build optimized release binary (LTO, strip, panic=abort)
build-release:
	cargo build --release

# ─── Compress Single File ─────────────────────────────────────────────────────
#
# Supports PDF and images (JPEG, PNG, GIF, TIFF, BMP, WebP).
# File type auto-detected from magic bytes.
#
# Variables:
#   FILE       (required) Input file path
#   OUTPUT     Output file path (default: <input>_compressed.<ext>)
#   LEVEL      Compression level: low|medium|high|extreme (default: medium)
#   QUALITY    Encoding quality 1-100 (overrides level preset)
#   DPI        Target DPI for image downsampling
#   FORMAT     Output format for images: original|jpeg|png|webp
#   MAXDIM     Max dimension in pixels (aspect ratio preserved)
#   GRAYSCALE  Set to 1 for grayscale conversion
#   LOSSLESS   Set to 1 for lossless mode (JPEG input → PNG)
#   FONTS      Set to 1 to enable font dedup + subsetting (PDF only)
#   STRIP_META Set to 1 to force metadata stripping
#   KEEP_META  Set to 1 to preserve metadata regardless of level
#   OVERWRITE  Set to 1 to overwrite existing output
#   DRYRUN     Set to 1 for dry-run mode
#   JSON       Set to 1 for JSON output

run:
	@if [ -z "$(FILE)" ]; then \
		echo "Usage: make run FILE=<input> [LEVEL=medium] [QUALITY=75] [DPI=150]"; \
		echo "       [FORMAT=jpeg|png|webp] [MAXDIM=1920] [GRAYSCALE=1]"; \
		echo "       [LOSSLESS=1] [FONTS=1] [STRIP_META=1] [KEEP_META=1]"; \
		echo "       [OUTPUT=out.pdf] [OVERWRITE=1] [DRYRUN=1] [JSON=1]"; \
		exit 1; \
	fi
	cargo run --release -- compress "$(FILE)" \
		--level $(or $(LEVEL),medium) \
		$(if $(OUTPUT),-o "$(OUTPUT)") \
		$(if $(QUALITY),--quality $(QUALITY)) \
		$(if $(DPI),--dpi $(DPI)) \
		$(if $(FORMAT),-f $(FORMAT)) \
		$(if $(MAXDIM),--max-dimension $(MAXDIM)) \
		$(if $(filter 1,$(GRAYSCALE)),--grayscale) \
		$(if $(filter 1,$(LOSSLESS)),--lossless) \
		$(if $(filter 1,$(FONTS)),--compress-fonts) \
		$(if $(filter 1,$(STRIP_META)),--strip-metadata) \
		$(if $(filter 1,$(KEEP_META)),--keep-metadata) \
		$(if $(filter 1,$(OVERWRITE)),--overwrite) \
		$(if $(filter 1,$(DRYRUN)),--dry-run) \
		$(if $(filter 1,$(JSON)),--json)

# ─── Batch Compress Directory ─────────────────────────────────────────────────
#
# Compress all supported files (PDF + images) in a directory.
# Default pattern: *.{pdf,jpg,jpeg,png,webp,tif,tiff,bmp,gif}
#
# Variables:
#   DIR        (required) Input directory path
#   OUTPUT     Output directory (default: <dir>_compressed)
#   LEVEL      Compression level: low|medium|high|extreme (default: medium)
#   PATTERN    Glob pattern (default: all supported formats)
#   WORKERS    Parallel workers (0 = auto-detect CPU count)
#   RECURSIVE  Set to -r for recursive traversal
#   QUALITY    Encoding quality 1-100 (overrides level preset)
#   DPI        Target DPI for image downsampling
#   FONTS      Set to 1 to enable font dedup + subsetting (PDF only)
#   KEEP_META  Set to 1 to preserve metadata
#   OVERWRITE  Set to 1 to overwrite existing outputs (default: on)
#   DRYRUN     Set to 1 for dry-run mode
#   JSON       Set to 1 for JSON output

batch:
	@if [ -z "$(DIR)" ]; then \
		echo "Usage: make batch DIR=<folder> [OUTPUT=<dir>] [LEVEL=medium]"; \
		echo "       [PATTERN=*.pdf] [WORKERS=4] [RECURSIVE=-r]"; \
		echo "       [QUALITY=75] [DPI=150] [FONTS=1] [KEEP_META=1]"; \
		echo "       [OVERWRITE=1] [DRYRUN=1] [JSON=1]"; \
		exit 1; \
	fi
	cargo run --release -- batch "$(DIR)" \
		--level $(or $(LEVEL),medium) \
		$(if $(OUTPUT),-o "$(OUTPUT)") \
		$(if $(PATTERN),-p "$(PATTERN)") \
		$(if $(WORKERS),-w $(WORKERS)) \
		$(RECURSIVE) \
		$(if $(QUALITY),--quality $(QUALITY)) \
		$(if $(DPI),--dpi $(DPI)) \
		$(if $(filter 1,$(FONTS)),--compress-fonts) \
		$(if $(filter 1,$(KEEP_META)),--keep-metadata) \
		$(if $(filter 1,$(DRYRUN)),--dry-run) \
		$(if $(filter 1,$(JSON)),--json) \
		--continue-on-error \
		$(if $(filter 1,$(OVERWRITE)),--overwrite,--overwrite)

# ─── Dry Run ─────────────────────────────────────────────────────────────────

## Estimate compression savings without writing: make dry-run FILE=input.pdf
dry-run:
	@if [ -z "$(FILE)" ]; then echo "Usage: make dry-run FILE=<input> [LEVEL=medium]"; exit 1; fi
	cargo run --release -- compress "$(FILE)" --level $(or $(LEVEL),medium) --dry-run

# ─── Info ─────────────────────────────────────────────────────────────────────

## Show PDF metadata and size info: make info FILE=input.pdf [JSON=1]
info:
	@if [ -z "$(FILE)" ]; then echo "Usage: make info FILE=<input.pdf> [JSON=1]"; exit 1; fi
	cargo run --release -- info "$(FILE)" \
		$(if $(filter 1,$(JSON)),--json)

# ─── Test & Lint ──────────────────────────────────────────────────────────────

## Run all tests
test:
	cargo test --all-features

## Run tests with output
test-verbose:
	cargo test --all-features -- --nocapture

## Run benchmarks (criterion)
bench:
	cargo bench

## Run clippy linter (zero warnings policy)
lint:
	cargo clippy --all-targets --all-features -- -D warnings

## Check formatting
fmt-check:
	cargo fmt --all -- --check

## Apply formatting
fmt:
	cargo fmt --all

## Run all checks (test + lint + fmt) — run before committing
check: test lint fmt-check

# ─── Utility ──────────────────────────────────────────────────────────────────

## Remove build artifacts
clean:
	cargo clean

## Install binary globally
install:
	cargo install --path .

# ─── Help ─────────────────────────────────────────────────────────────────────

help:
	@echo "media-compressor — Makefile Commands"
	@echo ""
	@echo "Build:"
	@echo "  make build                Build dev binary"
	@echo "  make build-release        Build release binary (LTO, strip)"
	@echo "  make install              Install binary globally"
	@echo ""
	@echo "Compress:"
	@echo "  make run FILE=x.pdf       Compress single file (PDF or image)"
	@echo "    LEVEL=medium              Compression level (low|medium|high|extreme)"
	@echo "    QUALITY=75                Encoding quality 1-100"
	@echo "    DPI=150                   Target DPI for downsampling"
	@echo "    FORMAT=jpeg               Output format (jpeg|png|webp)"
	@echo "    MAXDIM=1920               Max dimension in pixels"
	@echo "    GRAYSCALE=1               Convert to grayscale"
	@echo "    LOSSLESS=1                Lossless mode"
	@echo "    FONTS=1                   Enable font dedup + subsetting (PDF)"
	@echo "    STRIP_META=1              Force metadata stripping"
	@echo "    KEEP_META=1               Preserve metadata"
	@echo "    OUTPUT=out.pdf            Custom output path"
	@echo "    OVERWRITE=1               Overwrite existing output"
	@echo "    DRYRUN=1                  Estimate without writing"
	@echo "    JSON=1                    JSON output"
	@echo ""
	@echo "Batch:"
	@echo "  make batch DIR=./docs     Batch compress directory"
	@echo "    LEVEL=medium              Compression level"
	@echo "    PATTERN=*.pdf             Glob pattern filter"
	@echo "    WORKERS=4                 Parallel workers (0=auto)"
	@echo "    RECURSIVE=-r              Recurse subdirectories"
	@echo "    QUALITY=75                Encoding quality 1-100"
	@echo "    DPI=150                   Target DPI"
	@echo "    FONTS=1                   Enable font compression (PDF)"
	@echo "    KEEP_META=1               Preserve metadata"
	@echo "    OUTPUT=./out              Output directory"
	@echo "    OVERWRITE=1               Overwrite existing outputs"
	@echo "    DRYRUN=1                  Estimate without writing"
	@echo "    JSON=1                    JSON output"
	@echo ""
	@echo "Info:"
	@echo "  make dry-run FILE=x.pdf   Estimate compression savings"
	@echo "  make info FILE=x.pdf      Show PDF metadata (JSON=1 for JSON)"
	@echo ""
	@echo "Dev:"
	@echo "  make test                 Run all tests"
	@echo "  make test-verbose         Run tests with stdout"
	@echo "  make bench                Run criterion benchmarks"
	@echo "  make lint                 Run clippy (zero warnings)"
	@echo "  make fmt                  Format code"
	@echo "  make fmt-check            Check formatting"
	@echo "  make check                test + lint + fmt-check"
	@echo "  make clean                Remove build artifacts"
