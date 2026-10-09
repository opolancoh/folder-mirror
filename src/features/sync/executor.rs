//! Runs every action of a plan, records the outcome of each one, and never stops on a failed
//! file (unless `stop_on_error` is set or the user cancels).

use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use chrono::{Local, SecondsFormat};
use rayon::prelude::*;

use super::copier::{self, Failure, OpError, Retry};
use super::deleter::{prune_trash, Deleter};
use crate::features::plan::{ActionKind, ActionStatus, Execution, SyncAction, SyncPlan, EXIT_OK};
use crate::shared::config::Settings;
use crate::shared::error::{classify_io_error, ProblemReason, Stage};
use crate::shared::files;
use crate::shared::paths::display_path;
use crate::shared::progress::{ProgressReporter, Step};

struct Context<'a> {
    source_root: &'a Path,
    destination_root: &'a Path,
    settings: &'a Settings,
    deleter: &'a Deleter,
    retry: Retry,
    cancel: &'a AtomicBool,
    progress: &'a dyn ProgressReporter,
}

/// Executes the plan in place: each action gets its status, and failures are added to
/// `plan.problems`. Set `cancel` to stop after the files in progress.
pub fn execute(
    plan: &mut SyncPlan,
    settings: &Settings,
    progress: &dyn ProgressReporter,
    cancel: &AtomicBool,
) -> Execution {
    let started = Local::now();
    let clock = Instant::now();
    let run_id = started.format("%Y-%m-%d_%H%M%S").to_string();
    let source_root = plan.source_root.clone();
    let destination_root = plan.destination_root.clone();
    let deleter = Deleter::new(settings.delete_mode, &destination_root, &run_id);
    let ctx = Context {
        source_root: &source_root,
        destination_root: &destination_root,
        settings,
        deleter: &deleter,
        retry: Retry { attempts: settings.retries, delay: settings.retry_delay },
        cancel,
        progress,
    };

    progress.step_started(Step::Sync, Some(plan.actions.len() as u64));
    match fs::create_dir_all(&destination_root) {
        Ok(()) => {
            // Structural changes run in order; file transfers run in parallel.
            for kind in [ActionKind::RenameCase, ActionKind::Delete, ActionKind::RemoveDir, ActionKind::CreateDir] {
                for action in plan.actions.iter_mut().filter(|a| a.action == kind) {
                    run(action, &ctx);
                }
            }
            plan.actions
                .par_iter_mut()
                .filter(|a| matches!(a.action, ActionKind::Copy | ActionKind::Overwrite | ActionKind::UpdateTime))
                .for_each(|action| run(action, &ctx));
        }
        Err(err) => {
            let message = format!("cannot create the destination folder: {err}");
            for action in plan.actions.iter_mut().filter(|a| a.status == ActionStatus::Pending) {
                record_failure(action, OpError::new(classify_io_error(&err), message.clone(), 1), &ctx);
            }
        }
    }
    progress.step_finished(Step::Sync);

    let mut warnings = Vec::new();
    if let Some(note) = deleter.fallback_note() {
        warnings.push(note);
    }
    if let Err(err) = prune_trash(&destination_root, settings.trash_keep) {
        warnings.push(format!("could not clean up old trash folders: {err}"));
    }

    plan.problems.retain(|p| p.stage == Stage::Analyze);
    let failures: Vec<_> = plan
        .actions
        .iter()
        .filter(|a| matches!(a.status, ActionStatus::Failed | ActionStatus::Skipped))
        .filter(|a| a.problem != Some(ProblemReason::Cancelled))
        .map(SyncAction::to_problem)
        .collect();
    plan.problems.extend(failures);
    plan.refresh_summary();

    Execution {
        started_at: started.to_rfc3339_opts(SecondsFormat::Secs, false),
        finished_at: Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
        duration_secs: clock.elapsed().as_secs_f64(),
        delete_mode_used: deleter.effective_mode(),
        delete_mode_fallback: deleter.fallback_note(),
        trash_folder: deleter.trash_dir_used().map(display_path),
        cancelled: plan.summary.cancelled > 0,
        exit_code: EXIT_OK,
        warnings,
    }
}

fn run(action: &mut SyncAction, ctx: &Context) {
    if action.status == ActionStatus::Pending {
        if ctx.cancel.load(Ordering::Relaxed) {
            action.status = ActionStatus::Skipped;
            action.problem = Some(ProblemReason::Cancelled);
            action.error = Some("the sync was cancelled".into());
        } else {
            match perform(action, ctx) {
                Ok(attempts) => {
                    action.status = ActionStatus::Done;
                    action.attempts = (attempts > 1).then_some(attempts);
                }
                Err(err) => record_failure(action, err, ctx),
            }
        }
    }
    ctx.progress.advance(Step::Sync, 1);
}

fn record_failure(action: &mut SyncAction, err: OpError, ctx: &Context) {
    // A source that changed since the analysis is skipped, not failed: running again fixes it.
    action.status = if err.reason == ProblemReason::SourceChangedSincePlan {
        ActionStatus::Skipped
    } else {
        ActionStatus::Failed
    };
    action.problem = Some(err.reason);
    action.error = Some(err.error);
    action.attempts = (err.attempts > 0).then_some(err.attempts);
    if action.status == ActionStatus::Failed && ctx.settings.stop_on_error {
        ctx.cancel.store(true, Ordering::Relaxed);
    }
    ctx.progress.problem(Step::Sync);
}

fn perform(action: &SyncAction, ctx: &Context) -> Result<u32, OpError> {
    let source = action.path.to_native(ctx.source_root);
    let destination = action.path.to_native(ctx.destination_root);
    let cancel = ctx.cancel;

    match action.action {
        ActionKind::RenameCase => {
            let from = action
                .from_path
                .as_ref()
                .map(|p| p.to_native(ctx.destination_root))
                .ok_or_else(|| OpError::new(ProblemReason::IoError, "rename without an original name", 0))?;
            ctx.retry.run(cancel, || copier::rename_case(&from, &destination))
        }
        ActionKind::Delete => {
            ensure_absent_in_source(&source)?;
            ctx.retry.run(cancel, || match fs::symlink_metadata(&destination) {
                Ok(_) => ctx.deleter.dispose(&destination, &action.path).map_err(Failure::from),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(err) => Err(err.into()),
            })
        }
        ActionKind::RemoveDir => {
            ensure_absent_in_source(&source)?;
            ctx.retry.run(cancel, || remove_dir(&destination, action, ctx.deleter))
        }
        ActionKind::CreateDir => ctx.retry.run(cancel, || fs::create_dir_all(&destination).map_err(Failure::from)),
        ActionKind::Copy | ActionKind::Overwrite => {
            ensure_source_unchanged(&source, action)?;
            let backup = ctx.settings.backup_overwritten.then_some(ctx.deleter);
            ctx.retry.run(cancel, || {
                copier::copy_file(&source, &destination, &action.path, backup, ctx.settings.verify)
            })
        }
        ActionKind::UpdateTime => {
            let modified = ensure_source_unchanged(&source, action)?;
            ctx.retry.run(cancel, || files::set_modified(&destination, modified).map_err(Failure::from))
        }
    }
}

/// Removes an empty folder. If something is still inside (excluded files such as `Thumbs.db`),
/// the whole folder goes through the deleter.
fn remove_dir(path: &Path, action: &SyncAction, deleter: &Deleter) -> Result<(), Failure> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            let has_entries = fs::read_dir(path).map(|mut d| d.next().is_some()).unwrap_or(false);
            if has_entries {
                deleter.dispose(path, &action.path).map_err(Failure::from)
            } else {
                Err(err.into())
            }
        }
    }
}

fn skipped(message: &str) -> OpError {
    OpError::new(ProblemReason::SourceChangedSincePlan, message, 0)
}

/// The source must still match what the analysis saw; returns its modification time.
fn ensure_source_unchanged(source: &Path, action: &SyncAction) -> Result<SystemTime, OpError> {
    let meta = fs::metadata(source).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            skipped("the file no longer exists in the source; run the analysis again")
        } else {
            OpError::new(classify_io_error(&err), err.to_string(), 1)
        }
    })?;
    let modified = meta
        .modified()
        .map_err(|err| OpError::new(classify_io_error(&err), err.to_string(), 1))?;
    let same_size = action.size.map_or(true, |size| size == meta.len());
    let same_time = action
        .source_modified
        .as_deref()
        .and_then(files::time_from_string)
        .map_or(true, |time| time == modified);
    if same_size && same_time {
        Ok(modified)
    } else {
        Err(skipped("the source file changed after the analysis; run the analysis again"))
    }
}

/// Before deleting from the destination, make sure the entry did not reappear in the source.
/// Compares the exact name, so a case-insensitive source does not match a different-case name.
fn ensure_absent_in_source(source: &Path) -> Result<(), OpError> {
    if fs::symlink_metadata(source).is_err() {
        return Ok(());
    }
    let exists = match (source.parent(), source.file_name()) {
        (Some(parent), Some(name)) => fs::read_dir(parent)
            .map(|entries| entries.filter_map(Result::ok).any(|e| e.file_name() == name))
            .unwrap_or(false),
        _ => false,
    };
    if exists {
        Err(skipped("the entry exists again in the source; run the analysis again"))
    } else {
        Ok(())
    }
}
