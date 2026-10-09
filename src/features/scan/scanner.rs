//! Walks one folder tree and records files, folders, skipped entries and read errors.

use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use walkdir::{DirEntry, WalkDir};

use super::excludes::Excludes;
use crate::shared::error::{MirrorError, Problem, ProblemReason, Result};
use crate::shared::files::TRASH_DIR_NAME;
use crate::shared::paths::{display_path, RelPath};
use crate::shared::progress::{ProgressReporter, Step};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Source,
    Destination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub rel: RelPath,
    pub size: u64,
    pub modified: SystemTime,
}

/// An entry that is intentionally not mirrored (symlink, special or online-only file).
#[derive(Debug, Clone)]
pub struct SkippedEntry {
    pub rel: RelPath,
    pub problem: Problem,
}

/// Everything found under one root.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub root: PathBuf,
    /// False when the destination folder does not exist yet.
    pub exists: bool,
    pub files: Vec<FileEntry>,
    pub dirs: Vec<RelPath>,
    pub skipped: Vec<SkippedEntry>,
    pub problems: Vec<Problem>,
    /// Entries that could not be read. Whatever is below them on the other side is protected.
    pub unreadable: Vec<RelPath>,
}

pub fn scan(
    root: &Path,
    side: Side,
    excludes: &Excludes,
    progress: &dyn ProgressReporter,
) -> Result<Snapshot> {
    let mut snapshot = Snapshot {
        root: root.to_path_buf(),
        ..Default::default()
    };
    match fs::metadata(root) {
        Ok(meta) if meta.is_dir() => snapshot.exists = true,
        Ok(_) => return Err(MirrorError::NotADirectory(display_path(root))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return match side {
                Side::Source => Err(MirrorError::SourceNotFound(display_path(root))),
                Side::Destination => Ok(snapshot),
            };
        }
        Err(source) => {
            return Err(MirrorError::Unreadable {
                path: display_path(root),
                source,
            })
        }
    }

    let step = match side {
        Side::Source => Step::ScanSource,
        Side::Destination => Step::ScanDestination,
    };
    progress.step_started(step, None);

    let walker = WalkDir::new(root)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| !is_excluded(root, entry, excludes));

    for item in walker {
        match item {
            Ok(entry) => {
                if record_entry(&mut snapshot, side, &entry) {
                    progress.advance(step, 1);
                }
            }
            Err(err) => {
                let path = err.path().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
                let rel = RelPath::from_root(root, &path);
                if rel.as_ref().map_or(true, RelPath::is_root) {
                    progress.step_finished(step);
                    return Err(MirrorError::Unreadable {
                        path: display_path(root),
                        source: err.into_io_error().unwrap_or_else(|| io::Error::other("walk error")),
                    });
                }
                let reason = match side {
                    Side::Source => ProblemReason::SourceUnreadable,
                    Side::Destination => ProblemReason::DestinationUnreadable,
                };
                snapshot.problems.push(at(side, Problem::new(reason, err.to_string()), &path));
                snapshot.unreadable.extend(rel);
            }
        }
    }

    progress.step_finished(step);
    Ok(snapshot)
}

fn is_excluded(root: &Path, entry: &DirEntry, excludes: &Excludes) -> bool {
    match RelPath::from_root(root, entry.path()) {
        Some(rel) => {
            (entry.depth() == 1 && rel.as_str() == TRASH_DIR_NAME) || excludes.is_excluded(&rel)
        }
        None => false,
    }
}

/// Records one entry. Returns true when it was a file.
fn record_entry(snapshot: &mut Snapshot, side: Side, entry: &DirEntry) -> bool {
    let path = entry.path();
    let Some(rel) = RelPath::from_root(&snapshot.root, path) else {
        snapshot.problems.push(at(
            side,
            Problem::new(ProblemReason::InvalidName, "the name is not valid Unicode"),
            path,
        ));
        return false;
    };

    let file_type = entry.file_type();
    if file_type.is_symlink() {
        skip(snapshot, side, rel, path, ProblemReason::Symlink, "symbolic links and junctions are not followed");
        return false;
    }
    if file_type.is_dir() {
        snapshot.dirs.push(rel);
        return false;
    }
    if !file_type.is_file() {
        skip(snapshot, side, rel, path, ProblemReason::SpecialFile, "sockets, pipes and device files are not copied");
        return false;
    }

    let meta = match entry.metadata() {
        Ok(meta) => meta,
        Err(err) => {
            let io_err = err.into_io_error().unwrap_or_else(|| io::Error::other("metadata error"));
            snapshot.problems.push(at(side, Problem::from_io(&io_err), path));
            snapshot.unreadable.push(rel);
            return false;
        }
    };
    if is_cloud_placeholder(&meta) {
        skip(snapshot, side, rel, path, ProblemReason::CloudPlaceholder, "online-only file; reading it would download it");
        return false;
    }

    snapshot.files.push(FileEntry {
        rel,
        size: meta.len(),
        modified: meta.modified().unwrap_or(UNIX_EPOCH),
    });
    true
}

fn skip(snapshot: &mut Snapshot, side: Side, rel: RelPath, path: &Path, reason: ProblemReason, message: &str) {
    snapshot.skipped.push(SkippedEntry {
        rel,
        problem: at(side, Problem::new(reason, message), path),
    });
}

fn at(side: Side, problem: Problem, path: &Path) -> Problem {
    match side {
        Side::Source => problem.with_source(display_path(path)),
        Side::Destination => problem.with_destination(display_path(path)),
    }
}

#[cfg(windows)]
fn is_cloud_placeholder(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
    meta.file_attributes() & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn is_cloud_placeholder(_meta: &Metadata) -> bool {
    false
}
