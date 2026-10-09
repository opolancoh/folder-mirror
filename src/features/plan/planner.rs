//! Turns a [`Diff`] into an ordered [`SyncPlan`].

use chrono::{Local, SecondsFormat};

use super::model::{ActionKind, ActionReason, Summary, SyncAction, SyncPlan};
use crate::features::compare::{ChangeReason, Diff};
use crate::features::scan::{FileEntry, Snapshot};
use crate::shared::config::{DeleteMode, Settings};
use crate::shared::files::{self, TRASH_DIR_NAME};
use crate::shared::paths::{display_path, RelPath};

/// Builds the plan. Actions are ordered so they can run safely top to bottom:
/// case renames, file deletions (deepest first), folder removals (deepest first),
/// folder creations (shallowest first), then copies, overwrites and time updates.
pub fn build_plan(
    diff: Diff,
    source: Snapshot,
    destination: Snapshot,
    settings: &Settings,
    case_insensitive: bool,
) -> SyncPlan {
    let source_root = source.root.clone();
    let destination_root = destination.root.clone();
    let destination_path = |rel: &RelPath| display_path(&rel.to_native(&destination_root));
    let with_source = |mut action: SyncAction, file: &FileEntry| {
        action.source = Some(display_path(&file.rel.to_native(&source_root)));
        action.size = Some(file.size);
        action.source_modified = Some(files::time_to_string(file.modified));
        action
    };

    let mut actions = Vec::new();

    let mut renames = diff.case_renames;
    renames.sort_by(|a, b| (a.to.depth(), &a.to).cmp(&(b.to.depth(), &b.to)));
    for rename in renames {
        let mut action = SyncAction::new(
            ActionKind::RenameCase,
            ActionReason::CaseChanged,
            rename.to.clone(),
            destination_path(&rename.to),
        );
        action.from_path = Some(rename.from);
        actions.push(action);
    }

    let mut removed = diff.removed;
    removed.sort_by(|a, b| b.rel.depth().cmp(&a.rel.depth()).then_with(|| a.rel.cmp(&b.rel)));
    for file in removed {
        let mut action = SyncAction::new(ActionKind::Delete, ActionReason::Removed, file.rel.clone(), destination_path(&file.rel));
        action.size = Some(file.size);
        actions.push(action);
    }

    let mut dirs_removed = diff.dirs_removed;
    dirs_removed.sort_by(|a, b| b.depth().cmp(&a.depth()).then_with(|| a.cmp(b)));
    for dir in dirs_removed {
        let path = destination_path(&dir);
        actions.push(SyncAction::new(ActionKind::RemoveDir, ActionReason::Removed, dir, path));
    }

    let mut dirs_added = diff.dirs_added;
    dirs_added.sort_by(|a, b| a.depth().cmp(&b.depth()).then_with(|| a.cmp(b)));
    for dir in dirs_added {
        let path = destination_path(&dir);
        actions.push(SyncAction::new(ActionKind::CreateDir, ActionReason::Added, dir, path));
    }

    let mut added = diff.added;
    added.sort_by(|a, b| a.rel.cmp(&b.rel));
    for file in &added {
        let action = SyncAction::new(ActionKind::Copy, ActionReason::Added, file.rel.clone(), destination_path(&file.rel));
        actions.push(with_source(action, file));
    }

    let mut modified = diff.modified;
    modified.sort_by(|a, b| a.0.source.rel.cmp(&b.0.source.rel));
    for (pair, reason) in &modified {
        let reason = match reason {
            ChangeReason::SizeChanged => ActionReason::SizeChanged,
            ChangeReason::TimeChanged => ActionReason::TimeChanged,
            ChangeReason::ContentChanged => ActionReason::ContentChanged,
        };
        let rel = &pair.source.rel;
        let action = SyncAction::new(ActionKind::Overwrite, reason, rel.clone(), destination_path(rel));
        actions.push(with_source(action, &pair.source));
    }

    let mut time_only = diff.time_only;
    time_only.sort_by(|a, b| a.source.rel.cmp(&b.source.rel));
    for pair in &time_only {
        let rel = &pair.source.rel;
        let action = SyncAction::new(ActionKind::UpdateTime, ActionReason::TimeOnly, rel.clone(), destination_path(rel));
        actions.push(with_source(action, &pair.source));
    }

    let mut warnings = Vec::new();
    if !destination.exists {
        warnings.push("the destination folder does not exist yet; it will be created".to_string());
    }
    if diff.protected > 0 {
        warnings.push(format!(
            "{} destination file(s) were left untouched because the matching source entry could not be read or was skipped",
            diff.protected
        ));
    }
    let disposes_files = actions.iter().any(|a| {
        a.action == ActionKind::Delete || (a.action == ActionKind::Overwrite && settings.backup_overwritten)
    });
    if settings.delete_mode == DeleteMode::RecycleBin && disposes_files {
        if let Some(why) = files::recycle_bin_unavailable_reason(&destination_root) {
            warnings.push(format!(
                "Recycle Bin not available for {} ({why}); removed files will be moved to {}",
                display_path(&destination_root),
                display_path(&destination_root.join(TRASH_DIR_NAME)),
            ));
        }
    }

    let mut problems = source.problems;
    problems.extend(destination.problems);
    problems.extend(diff.problems);
    let skipped = source
        .skipped
        .into_iter()
        .chain(destination.skipped)
        .map(|entry| entry.problem)
        .collect();

    let mut plan = SyncPlan {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: Local::now().to_rfc3339_opts(SecondsFormat::Secs, false),
        source: display_path(&source_root),
        destination: display_path(&destination_root),
        mode: settings.mode,
        delete_mode: settings.delete_mode,
        case_insensitive,
        summary: Summary {
            source_files: source.files.len() as u64,
            destination_files: destination.files.len() as u64,
            unchanged: diff.unchanged,
            protected: diff.protected,
            ..Summary::default()
        },
        execution: None,
        blockers: Vec::new(),
        warnings,
        problems,
        skipped,
        actions,
        source_root,
        destination_root,
    };
    plan.refresh_summary();
    plan
}
