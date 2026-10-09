//! Scan feature: walks a folder tree into a [`Snapshot`].

mod excludes;
mod scanner;

pub use excludes::{Excludes, DEFAULT_EXCLUDES};
pub use scanner::{scan, FileEntry, Side, SkippedEntry, Snapshot};

use std::path::Path;

use crate::shared::error::Result;
use crate::shared::progress::ProgressReporter;

/// Scans source and destination at the same time.
pub fn scan_both(
    source_root: &Path,
    destination_root: &Path,
    excludes: &Excludes,
    progress: &dyn ProgressReporter,
) -> Result<(Snapshot, Snapshot)> {
    let (source, destination) = rayon::join(
        || scan(source_root, Side::Source, excludes, progress),
        || scan(destination_root, Side::Destination, excludes, progress),
    );
    Ok((source?, destination?))
}
