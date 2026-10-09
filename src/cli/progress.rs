//! Console progress: one line, updated in place and cleared when each step ends
//! (hidden automatically when the output is not a terminal).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

use folder_mirror::shared::format::format_count;
use folder_mirror::shared::progress::{ProgressReporter, Step};

pub struct ConsoleProgress {
    bar: Mutex<Option<ProgressBar>>,
    scanned_source: AtomicU64,
    scanned_destination: AtomicU64,
    /// Source and destination are scanned at the same time and share one line.
    scans_running: AtomicU64,
    problems: AtomicU64,
}

impl ConsoleProgress {
    pub fn new() -> Self {
        Self {
            bar: Mutex::new(None),
            scanned_source: AtomicU64::new(0),
            scanned_destination: AtomicU64::new(0),
            scans_running: AtomicU64::new(0),
            problems: AtomicU64::new(0),
        }
    }

    fn replace(&self, new: Option<ProgressBar>) {
        let mut bar = self.bar.lock().unwrap();
        if let Some(old) = bar.take() {
            old.finish_and_clear();
        }
        *bar = new;
    }

    fn with_bar(&self, f: impl FnOnce(&ProgressBar)) {
        if let Some(bar) = self.bar.lock().unwrap().as_ref() {
            f(bar);
        }
    }

    fn scan_message(&self) -> String {
        format!(
            "{} source / {} destination files",
            format_count(self.scanned_source.load(Ordering::Relaxed)),
            format_count(self.scanned_destination.load(Ordering::Relaxed))
        )
    }
}

fn is_scan(step: Step) -> bool {
    matches!(step, Step::ScanSource | Step::ScanDestination)
}

impl ProgressReporter for ConsoleProgress {
    fn step_started(&self, step: Step, total: Option<u64>) {
        if is_scan(step) {
            if self.scans_running.fetch_add(1, Ordering::Relaxed) == 0 {
                let bar = ProgressBar::new_spinner()
                    .with_style(ProgressStyle::with_template("{spinner:.cyan} Scanning {msg}").unwrap());
                bar.set_message(self.scan_message());
                bar.enable_steady_tick(Duration::from_millis(120));
                self.replace(Some(bar));
            }
            return;
        }
        let bar = ProgressBar::new(total.unwrap_or(0)).with_style(
            ProgressStyle::with_template("{prefix:<10} [{bar:30.cyan/blue}] {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_prefix(step.label());
        bar.enable_steady_tick(Duration::from_millis(120));
        self.replace(Some(bar));
    }

    fn advance(&self, step: Step, amount: u64) {
        match step {
            Step::ScanSource => {
                self.scanned_source.fetch_add(amount, Ordering::Relaxed);
                self.with_bar(|bar| bar.set_message(self.scan_message()));
            }
            Step::ScanDestination => {
                self.scanned_destination.fetch_add(amount, Ordering::Relaxed);
                self.with_bar(|bar| bar.set_message(self.scan_message()));
            }
            _ => self.with_bar(|bar| bar.inc(amount)),
        }
    }

    fn problem(&self, _step: Step) {
        let count = self.problems.fetch_add(1, Ordering::Relaxed) + 1;
        self.with_bar(|bar| bar.set_message(format!("⚠ {count} error{}", if count == 1 { "" } else { "s" })));
    }

    fn step_finished(&self, step: Step) {
        if is_scan(step) && self.scans_running.fetch_sub(1, Ordering::Relaxed) > 1 {
            return; // the other side is still scanning
        }
        self.replace(None);
    }
}

impl Drop for ConsoleProgress {
    fn drop(&mut self) {
        self.replace(None);
    }
}
