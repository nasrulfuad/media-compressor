use std::process::ExitCode;

use clap::Parser;
use colored::Colorize;

use media_compressor::app::batch::process_batch;
use media_compressor::app::compress_file::compress_file;
use media_compressor::app::info::read_pdf_info;
use media_compressor::app::progress::{BatchProgress, NoProgress};
use media_compressor::app::registry::CompressorRegistry;
use media_compressor::app::{BatchRequest, CompressRequest};
use media_compressor::cli::args::{BatchArgs, Cli, Commands, CompressArgs, InfoArgs};
use media_compressor::cli::progress::IndicatifProgress;
use media_compressor::domain::options::DEFAULT_MAX_INPUT_BYTES;
use media_compressor::error::{AppResult, EXIT_CODE_RUNTIME_ERROR};
use media_compressor::presentation::json::to_pretty_json;
use media_compressor::presentation::pretty::{
    render_batch_report, render_file_report, render_info_report,
};

fn init_logging(verbose: bool, quiet: bool) {
    let default_filter = if quiet {
        "error"
    } else if verbose {
        "debug"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

fn run_compress(args: CompressArgs) -> AppResult<ExitCode> {
    let json = args.json;
    let request = CompressRequest::from(args);
    let report = compress_file(&request, &CompressorRegistry::standard())?;
    if json {
        println!("{}", to_pretty_json(&report)?);
    } else {
        print!("{}", render_file_report(&report));
    }
    Ok(ExitCode::SUCCESS)
}

fn batch_progress(silent: bool) -> Box<dyn BatchProgress> {
    if silent {
        Box::new(NoProgress)
    } else {
        Box::new(IndicatifProgress::new())
    }
}

fn run_batch(args: BatchArgs, quiet: bool) -> AppResult<ExitCode> {
    let json = args.json;
    let progress = batch_progress(json || quiet);
    let request = BatchRequest::from(args);
    let report = process_batch(&request, &CompressorRegistry::standard(), progress.as_ref())?;
    if json {
        println!("{}", to_pretty_json(&report)?);
    } else {
        print!("{}", render_batch_report(&report));
    }
    if report.failed > 0 {
        return Ok(ExitCode::from(EXIT_CODE_RUNTIME_ERROR));
    }
    Ok(ExitCode::SUCCESS)
}

fn run_info(args: InfoArgs) -> AppResult<ExitCode> {
    let report = read_pdf_info(&args.input, DEFAULT_MAX_INPUT_BYTES)?;
    if args.json {
        println!("{}", to_pretty_json(&report)?);
    } else {
        print!("{}", render_info_report(&report));
    }
    Ok(ExitCode::SUCCESS)
}

fn run(cli: Cli) -> AppResult<ExitCode> {
    match cli.command {
        Commands::Compress(args) => run_compress(args),
        Commands::Batch(args) => run_batch(args, cli.quiet),
        Commands::Info(args) => run_info(args),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);
    match run(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {}", "Error:".red().bold(), error);
            ExitCode::from(error.exit_code())
        }
    }
}
