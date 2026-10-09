//! Console progress: one line, updated in place and cleared when each step ends
//! (hidden automatically when the output is not a terminal).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressState, ProgressStyle};

use folder_mirror::shared::format::{format_bytes, format_count};
use folder_mirror::shared::progress::{ProgressReporter, Step};

pub struct ConsoleProgress {
    bar: Mutex<Option<ProgressBar>>,
    scanned_source: AtomicU64,
    scanned_destination: AtomicU64,
    /// Source and destination are scanned at the same time and share one line.
    scans_running: AtomicU64,
    problems: AtomicU64,
    /// Sync shown by size (bytes) instead of by number of actions.
    sync_by_size: AtomicBool,
    sync_done: AtomicU64,
    sync_total: AtomicU64,
}

impl ConsoleProgress {
    pub fn new() -> Self {
        Self {
            bar: Mutex::new(None),
            scanned_source: AtomicU64::new(0),
            scanned_destination: AtomicU64::new(0),
            scans_running: AtomicU64::new(0),
            problems: AtomicU64::new(0),
            sync_by_size: AtomicBool::new(false),
            sync_done: AtomicU64::new(0),
            sync_total: AtomicU64::new(0),
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

    fn errors_message(&self) -> String {
        match self.problems.load(Ordering::Relaxed) {
            0 => String::new(),
            1 => "⚠ 1 error".into(),
            n => format!("⚠ {n} errors"),
        }
    }

    /// `534/1,188 actions · ⚠ 2 errors` for the size-based sync line.
    fn sync_message(&self) -> String {
        let mut message = format!(
            "{}/{} actions",
            format_count(self.sync_done.load(Ordering::Relaxed)),
            format_count(self.sync_total.load(Ordering::Relaxed))
        );
        let errors = self.errors_message();
        if !errors.is_empty() {
            message.push_str(" · ");
            message.push_str(&errors);
        }
        message
    }
}

fn is_scan(step: Step) -> bool {
    matches!(step, Step::ScanSource | Step::ScanDestination)
}

/// `1m 23s` style durations for the ETA.
fn short_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60),
    }
}

fn size_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{prefix:<10} [{bar:30.cyan/blue}] {copied} / {size} ({percent}%) · {speed} · ETA {left} · {msg}",
    )
    .unwrap()
    .with_key("copied", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = w.write_str(&format_bytes(state.pos()));
    })
    .with_key("size", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = w.write_str(&format_bytes(state.len().unwrap_or(0)));
    })
    .with_key("speed", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = write!(w, "{}/s", format_bytes(state.per_sec() as u64));
    })
    .with_key("left", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
        let _ = w.write_str(&short_duration(state.eta()));
    })
    .progress_chars("=> ")
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
            ProgressStyle::with_template("{prefix:<10} [{bar:30.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        bar.set_prefix(step.label());
        bar.enable_steady_tick(Duration::from_millis(120));
        self.replace(Some(bar));
    }

    fn sync_started(&self, actions: u64, bytes: u64) {
        self.sync_done.store(0, Ordering::Relaxed);
        self.sync_total.store(actions, Ordering::Relaxed);
        if bytes == 0 {
            // Nothing to copy (only deletions, folders...): count actions instead.
            self.sync_by_size.store(false, Ordering::Relaxed);
            self.step_started(Step::Sync, Some(actions));
            return;
        }
        self.sync_by_size.store(true, Ordering::Relaxed);
        let bar = ProgressBar::new(bytes).with_style(size_style());
        bar.set_prefix(Step::Sync.label());
        bar.set_message(self.sync_message());
        bar.enable_steady_tick(Duration::from_millis(200));
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
            Step::Sync if self.sync_by_size.load(Ordering::Relaxed) => {
                self.sync_done.fetch_add(amount, Ordering::Relaxed);
                self.with_bar(|bar| bar.set_message(self.sync_message()));
            }
            _ => self.with_bar(|bar| bar.inc(amount)),
        }
    }

    fn bytes_copied(&self, amount: u64) {
        if self.sync_by_size.load(Ordering::Relaxed) {
            self.with_bar(|bar| bar.inc(amount));
        }
    }

    fn problem(&self, step: Step) {
        self.problems.fetch_add(1, Ordering::Relaxed);
        let message = if step == Step::Sync && self.sync_by_size.load(Ordering::Relaxed) {
            self.sync_message()
        } else {
            self.errors_message()
        };
        self.with_bar(|bar| bar.set_message(message));
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
