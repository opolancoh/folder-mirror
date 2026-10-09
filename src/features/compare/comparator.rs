//! The comparison strategy interface (Strategy pattern) and helpers shared by the strategies.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::strategies::{
    FullHashComparator, MetadataComparator, NameComparator, QuickHashComparator, SmartComparator,
};
use crate::features::scan::{FileEntry, Side};
use crate::shared::config::CompareMode;
use crate::shared::error::Problem;
use crate::shared::files;
use crate::shared::hashing;
use crate::shared::paths::display_path;

/// Result of comparing a file that exists on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Unchanged,
    Modified(ChangeReason),
    /// Same content, different modification time: only the time needs updating.
    TimeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeReason {
    SizeChanged,
    TimeChanged,
    ContentChanged,
}

/// A file that could not be read while comparing.
#[derive(Debug)]
pub struct CompareError {
    pub side: Side,
    pub path: PathBuf,
    pub error: io::Error,
}

impl CompareError {
    pub fn into_problem(self) -> Problem {
        let problem = Problem::from_io(&self.error);
        match self.side {
            Side::Source => problem.with_source(display_path(&self.path)),
            Side::Destination => problem.with_destination(display_path(&self.path)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashKind {
    Sampled,
    Full,
}

pub struct CompareContext<'a> {
    pub source_root: &'a Path,
    pub destination_root: &'a Path,
    pub mtime_tolerance: Duration,
}

impl CompareContext<'_> {
    pub fn same_time(&self, source: &FileEntry, destination: &FileEntry) -> bool {
        files::times_match(source.modified, destination.modified, self.mtime_tolerance)
    }

    /// Hashes both files in parallel and tells whether their content is equal.
    pub fn same_content(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        kind: HashKind,
    ) -> Result<bool, CompareError> {
        let source_path = source.rel.to_native(self.source_root);
        let destination_path = destination.rel.to_native(self.destination_root);
        let hash = |path: &Path, size: u64| match kind {
            HashKind::Sampled => hashing::sampled_hash(path, size),
            HashKind::Full => hashing::full_hash(path),
        };
        let (a, b) = rayon::join(
            || hash(&source_path, source.size),
            || hash(&destination_path, destination.size),
        );
        let a = a.map_err(|error| CompareError { side: Side::Source, path: source_path.clone(), error })?;
        let b = b.map_err(|error| CompareError { side: Side::Destination, path: destination_path.clone(), error })?;
        Ok(a == b)
    }
}

/// A comparison strategy. Add a new mode by implementing this trait and registering it in
/// [`comparator_for`].
pub trait Comparator: Send + Sync {
    fn compare(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        ctx: &CompareContext,
    ) -> Result<Verdict, CompareError>;
}

pub fn comparator_for(mode: CompareMode) -> Box<dyn Comparator> {
    match mode {
        CompareMode::Name => Box::new(NameComparator),
        CompareMode::Metadata => Box::new(MetadataComparator),
        CompareMode::Smart => Box::new(SmartComparator),
        CompareMode::QuickHash => Box::new(QuickHashComparator),
        CompareMode::FullHash => Box::new(FullHashComparator),
    }
}
