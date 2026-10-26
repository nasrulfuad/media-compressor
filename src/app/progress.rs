pub trait BatchProgress: Sync {
    fn start(&self, total_files: usize);
    fn file_finished(&self);
    fn finish(&self);
}

pub struct NoProgress;

impl BatchProgress for NoProgress {
    fn start(&self, _total_files: usize) {}

    fn file_finished(&self) {}

    fn finish(&self) {}
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    pub struct CountingProgress {
        pub total: AtomicUsize,
        pub finished_files: AtomicUsize,
        pub finish_calls: AtomicUsize,
    }

    impl BatchProgress for CountingProgress {
        fn start(&self, total_files: usize) {
            self.total.store(total_files, Ordering::SeqCst);
        }

        fn file_finished(&self) {
            self.finished_files.fetch_add(1, Ordering::SeqCst);
        }

        fn finish(&self) {
            self.finish_calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn no_progress_accepts_every_event() {
        let progress = NoProgress;
        progress.start(3);
        progress.file_finished();
        progress.finish();
    }

    #[test]
    fn counting_progress_records_events() {
        let progress = CountingProgress::default();
        progress.start(2);
        progress.file_finished();
        progress.file_finished();
        progress.finish();
        assert_eq!(progress.total.load(Ordering::SeqCst), 2);
        assert_eq!(progress.finished_files.load(Ordering::SeqCst), 2);
        assert_eq!(progress.finish_calls.load(Ordering::SeqCst), 1);
    }
}
