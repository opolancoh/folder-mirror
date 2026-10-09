//! Use cases: chain the features into the operations a user interface offers.
//! The CLI uses these today; a TUI or GUI can call the same functions.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::features::compare::{self, CompareOptions};
use crate::features::plan::{self, SyncPlan};
use crate::features::report;
use crate::features::scan::{self, Excludes};
use crate::features::sync;
use crate::shared::config::Settings;
use crate::shared::error::{MirrorError, Result};
use crate::shared::paths::{self, display_path};
use crate::shared::progress::ProgressReporter;

/// Sets the number of worker threads (once per process).
pub fn configure_threads(settings: &Settings) -> Result<()> {
    if let Some(threads) = settings.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| MirrorError::Config(e.to_string()))?;
    }
    Ok(())
}

/// Scans both folders, compares them and returns the plan. Changes nothing.
pub fn analyze(settings: &Settings, progress: &dyn ProgressReporter) -> Result<SyncPlan> {
    let source_root = paths::resolve_root(&settings.source)?;
    let destination_root = paths::resolve_root(&settings.destination)?;
    if paths::overlaps(&source_root, &destination_root) {
        return Err(MirrorError::Overlap(display_path(&source_root), display_path(&destination_root)));
    }

    let excludes = Excludes::new(&settings.excludes, settings.default_excludes)?;
    let (source, destination) = scan::scan_both(&source_root, &destination_root, &excludes, progress)?;

    let case_insensitive = paths::detect_case_insensitive(&destination_root);
    let options = CompareOptions {
        mode: settings.mode,
        mtime_tolerance: settings.mtime_tolerance,
        case_insensitive,
    };
    let diff = compare::compare(&source, &destination, &options, progress);

    let mut plan = plan::build_plan(diff, source, destination, settings, case_insensitive);
    plan::check_safety(&mut plan, settings);
    Ok(plan)
}

/// Loads a plan written by `analyze`, ready to be executed.
pub fn load_plan(path: &Path) -> Result<SyncPlan> {
    let mut plan = report::read_plan(path)?;
    plan.source_root = paths::resolve_root(Path::new(&plan.source))?;
    if !plan.source_root.is_dir() {
        return Err(MirrorError::SourceNotFound(plan.source.clone()));
    }
    plan.destination_root = paths::resolve_root(Path::new(&plan.destination))?;
    plan.prepare_for_execution();
    Ok(plan)
}

/// Executes the plan. Refuses to run when a safety check failed.
pub fn sync(
    plan: &mut SyncPlan,
    settings: &Settings,
    progress: &dyn ProgressReporter,
    cancel: &AtomicBool,
) -> Result<()> {
    if !plan.blockers.is_empty() {
        return Err(MirrorError::Blocked(plan.blockers.clone()));
    }
    let execution = sync::execute(plan, settings, progress, cancel);
    plan.execution = Some(execution);
    let code = plan.exit_code();
    if let Some(execution) = plan.execution.as_mut() {
        execution.exit_code = code;
    }
    Ok(())
}
