use std::sync::Mutex;

use indicatif::{ProgressBar, ProgressStyle};

use crate::app::progress::BatchProgress;

const PROGRESS_TEMPLATE: &str = "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}";
const PROGRESS_CHARS: &str = "=> ";

#[derive(Default)]
pub struct IndicatifProgress {
    bar: Mutex<Option<ProgressBar>>,
}

impl IndicatifProgress {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_bar(&self, action: impl FnOnce(&ProgressBar)) {
        if let Ok(guard) = self.bar.lock() {
            if let Some(bar) = guard.as_ref() {
                action(bar);
            }
        }
    }
}

impl BatchProgress for IndicatifProgress {
    fn start(&self, total_files: usize) {
        let bar = ProgressBar::new(total_files as u64);
        if let Ok(style) = ProgressStyle::with_template(PROGRESS_TEMPLATE) {
            bar.set_style(style.progress_chars(PROGRESS_CHARS));
        }
        bar.set_message("Compressing...");
        if let Ok(mut guard) = self.bar.lock() {
            *guard = Some(bar);
        }
    }

    fn file_finished(&self) {
        self.with_bar(|bar| bar.inc(1));
    }

    fn finish(&self) {
        self.with_bar(|bar| bar.finish_with_message("Done"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_before_start_are_ignored() {
        let progress = IndicatifProgress::new();
        progress.file_finished();
        progress.finish();
    }

    #[test]
    fn progress_bar_tracks_finished_files() {
        let progress = IndicatifProgress::new();
        progress.start(3);
        progress.file_finished();
        progress.file_finished();
        let position = progress
            .bar
            .lock()
            .unwrap()
            .as_ref()
            .map(|bar| bar.position());
        assert_eq!(position, Some(2));
        progress.finish();
    }
}
