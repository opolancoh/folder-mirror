//! Compare feature: matches two snapshots and classifies every entry into a [`Diff`].

mod comparator;
mod strategies;

pub use comparator::{
    comparator_for, ChangeReason, CompareContext, CompareError, Comparator, HashKind, Verdict,
};

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use rayon::prelude::*;

use crate::features::scan::{FileEntry, Snapshot};
use crate::shared::config::CompareMode;
use crate::shared::error::{Problem, ProblemReason};
use crate::shared::paths::{ancestor_keys, display_path, windows_name_issue, RelPath};
use crate::shared::progress::{ProgressReporter, Step};

pub struct CompareOptions {
    pub mode: CompareMode,
    pub mtime_tolerance: Duration,
    /// Whether the destination treats `A.txt` and `a.txt` as the same file.
    pub case_insensitive: bool,
}

#[derive(Debug, Clone)]
pub struct MatchedPair {
    pub source: FileEntry,
    pub destination: FileEntry,
}

/// An entry whose name differs only by letter case between both sides.
#[derive(Debug, Clone)]
pub struct CaseRename {
    pub from: RelPath,
    pub to: RelPath,
}

#[derive(Debug, Default)]
pub struct Diff {
    pub added: Vec<FileEntry>,
    pub modified: Vec<(MatchedPair, ChangeReason)>,
    pub time_only: Vec<MatchedPair>,
    pub removed: Vec<FileEntry>,
    pub case_renames: Vec<CaseRename>,
    pub dirs_added: Vec<RelPath>,
    pub dirs_removed: Vec<RelPath>,
    pub unchanged: u64,
    /// Destination entries kept because their source counterpart could not be read or was skipped.
    pub protected: u64,
    pub problems: Vec<Problem>,
}

pub fn compare(
    source: &Snapshot,
    destination: &Snapshot,
    options: &CompareOptions,
    progress: &dyn ProgressReporter,
) -> Diff {
    let ci = options.case_insensitive;
    let mut diff = Diff::default();

    // Keys whose destination entries (and everything below them) must not be deleted.
    let mut protected: HashSet<String> = source.unreadable.iter().map(|r| r.key(ci)).collect();
    protected.extend(source.skipped.iter().map(|s| s.rel.key(ci)));

    let mut source_files: HashMap<String, &FileEntry> = HashMap::with_capacity(source.files.len());
    for file in &source.files {
        let key = file.rel.key(ci);
        let source_path = || display_path(&file.rel.to_native(&source.root));
        if let Some(issue) = destination_name_issue(&file.rel) {
            diff.problems.push(
                Problem::new(ProblemReason::InvalidName, format!("the name {issue}")).with_source(source_path()),
            );
            protected.insert(key);
            continue;
        }
        if let Some(existing) = source_files.get(&key) {
            diff.problems.push(
                Problem::new(
                    ProblemReason::CaseConflict,
                    format!("differs only by letter case from '{}', which the destination cannot store separately", existing.rel),
                )
                .with_source(source_path()),
            );
            continue;
        }
        source_files.insert(key, file);
    }

    let source_dirs: HashMap<String, &RelPath> = source.dirs.iter().map(|d| (d.key(ci), d)).collect();
    let destination_files: HashMap<String, &FileEntry> =
        destination.files.iter().map(|f| (f.rel.key(ci), f)).collect();
    let destination_dirs: HashMap<String, &RelPath> =
        destination.dirs.iter().map(|d| (d.key(ci), d)).collect();
    let is_protected =
        |key: &str| protected.contains(key) || ancestor_keys(key).any(|a| protected.contains(a));

    // Files: added, matched pairs and case-only renames.
    let mut pairs: Vec<(&FileEntry, &FileEntry)> = Vec::new();
    for (key, src) in &source_files {
        match destination_files.get(key) {
            Some(dst) => {
                if src.rel.name() != dst.rel.name() {
                    diff.case_renames.push(CaseRename { from: dst.rel.clone(), to: src.rel.clone() });
                }
                pairs.push((src, dst));
            }
            None => diff.added.push((*src).clone()),
        }
    }
    for (key, dst) in &destination_files {
        if source_files.contains_key(key) {
            continue;
        }
        if is_protected(key) {
            diff.protected += 1;
        } else {
            diff.removed.push((*dst).clone());
        }
    }

    // Folders.
    for (key, src) in &source_dirs {
        match destination_dirs.get(key) {
            Some(dst) if src.name() != dst.name() => {
                diff.case_renames.push(CaseRename { from: (*dst).clone(), to: (*src).clone() });
            }
            Some(_) => {}
            None => diff.dirs_added.push((*src).clone()),
        }
    }
    for (key, dst) in &destination_dirs {
        if !source_dirs.contains_key(key) && !is_protected(key) {
            diff.dirs_removed.push((*dst).clone());
        }
    }

    // Content comparison of matched files, in parallel.
    let comparator = comparator_for(options.mode);
    let ctx = CompareContext {
        source_root: &source.root,
        destination_root: &destination.root,
        mtime_tolerance: options.mtime_tolerance,
    };
    progress.step_started(Step::Compare, Some(pairs.len() as u64));
    let verdicts: Vec<Result<Verdict, CompareError>> = pairs
        .par_iter()
        .map(|(src, dst)| {
            let verdict = comparator.compare(src, dst, &ctx);
            progress.advance(Step::Compare, 1);
            verdict
        })
        .collect();
    progress.step_finished(Step::Compare);

    for ((src, dst), verdict) in pairs.into_iter().zip(verdicts) {
        let pair = || MatchedPair { source: src.clone(), destination: dst.clone() };
        match verdict {
            Ok(Verdict::Unchanged) => diff.unchanged += 1,
            Ok(Verdict::TimeOnly) => diff.time_only.push(pair()),
            Ok(Verdict::Modified(reason)) => diff.modified.push((pair(), reason)),
            // Could not read one side: leave the destination untouched and report it.
            Err(err) => diff.problems.push(err.into_problem()),
        }
    }

    diff
}

/// A name the destination cannot store. Only checked on Windows, the strictest platform.
fn destination_name_issue(rel: &RelPath) -> Option<&'static str> {
    if cfg!(windows) {
        rel.as_str().split('/').find_map(windows_name_issue)
    } else {
        None
    }
}
