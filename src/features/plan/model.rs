//! The sync plan: an ordered list of actions plus its summary. It is what `analyze` writes to the
//! result file and what `sync` executes, so the dry run and the real run always match.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::shared::config::{CompareMode, DeleteMode};
use crate::shared::error::{Problem, ProblemReason, Stage};
use crate::shared::paths::RelPath;

/// Exit codes shared by every interface.
pub const EXIT_OK: i32 = 0;
pub const EXIT_DIFFERENCES: i32 = 1;
pub const EXIT_PROBLEMS: i32 = 2;
pub const EXIT_ABORTED: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Fix the letter case of an existing destination entry.
    RenameCase,
    Delete,
    RemoveDir,
    CreateDir,
    Copy,
    Overwrite,
    /// Content is identical; only the modification time is updated.
    UpdateTime,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RenameCase => "rename_case",
            Self::Delete => "delete",
            Self::RemoveDir => "remove_dir",
            Self::CreateDir => "create_dir",
            Self::Copy => "copy",
            Self::Overwrite => "overwrite",
            Self::UpdateTime => "update_time",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionReason {
    Added,
    SizeChanged,
    TimeChanged,
    ContentChanged,
    TimeOnly,
    Removed,
    CaseChanged,
}

impl ActionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::SizeChanged => "size_changed",
            Self::TimeChanged => "time_changed",
            Self::ContentChanged => "content_changed",
            Self::TimeOnly => "time_only",
            Self::Removed => "removed",
            Self::CaseChanged => "case_changed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    #[default]
    Pending,
    Done,
    Failed,
    Skipped,
}

impl ActionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// One operation on the destination, with full paths for the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAction {
    pub action: ActionKind,
    pub reason: ActionReason,
    /// Relative path (with `/`), used to rebuild native paths when a plan file is executed.
    pub path: RelPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_path: Option<RelPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub destination: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Source modification time at analysis, to detect files that changed before the sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_modified: Option<String>,
    #[serde(default)]
    pub status: ActionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ProblemReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SyncAction {
    pub fn new(action: ActionKind, reason: ActionReason, path: RelPath, destination: String) -> Self {
        Self {
            action,
            reason,
            path,
            from_path: None,
            source: None,
            destination,
            size: None,
            source_modified: None,
            status: ActionStatus::Pending,
            problem: None,
            attempts: None,
            error: None,
        }
    }

    pub fn to_problem(&self) -> Problem {
        Problem {
            reason: self.problem.unwrap_or(ProblemReason::IoError),
            stage: Stage::Sync,
            action: Some(self.action.as_str().to_string()),
            source: self.source.clone(),
            destination: Some(self.destination.clone()),
            attempts: self.attempts,
            error: self.error.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Counter {
    pub planned: u64,
    pub done: u64,
    pub failed: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProblemSummary {
    pub total: u64,
    pub by_reason: BTreeMap<ProblemReason, u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub source_files: u64,
    pub destination_files: u64,
    pub added: Counter,
    pub modified: Counter,
    pub removed: Counter,
    pub time_updated: Counter,
    pub renamed: Counter,
    pub dirs_created: Counter,
    pub dirs_removed: Counter,
    pub unchanged: u64,
    pub protected: u64,
    pub bytes_added: u64,
    pub bytes_modified: u64,
    pub skipped_entries: u64,
    pub cancelled: u64,
    pub problems: ProblemSummary,
}

/// What happened when the plan was executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub started_at: String,
    pub finished_at: String,
    pub duration_secs: f64,
    pub delete_mode_used: DeleteMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_mode_fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trash_folder: Option<String>,
    pub cancelled: bool,
    pub exit_code: i32,
    /// Warnings raised while syncing (the analysis ones stay in `SyncPlan::warnings`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPlan {
    pub tool_version: String,
    pub created_at: String,
    pub source: String,
    pub destination: String,
    pub mode: CompareMode,
    pub delete_mode: DeleteMode,
    #[serde(default)]
    pub case_insensitive: bool,
    pub summary: Summary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Execution>,
    /// Safety checks that failed; the sync refuses to run while this is not empty.
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub problems: Vec<Problem>,
    /// Entries intentionally not mirrored (symlinks, special and online-only files).
    #[serde(default)]
    pub skipped: Vec<Problem>,
    pub actions: Vec<SyncAction>,
    /// Native roots (`\\?\` form on Windows). Not stored in the file.
    #[serde(skip)]
    pub source_root: PathBuf,
    #[serde(skip)]
    pub destination_root: PathBuf,
}

impl SyncPlan {
    pub fn has_pending_actions(&self) -> bool {
        self.actions.iter().any(|a| a.status == ActionStatus::Pending)
    }

    /// Recomputes the counters from the actions and problems.
    pub fn refresh_summary(&mut self) {
        let summary = &mut self.summary;
        for counter in [
            &mut summary.added,
            &mut summary.modified,
            &mut summary.removed,
            &mut summary.time_updated,
            &mut summary.renamed,
            &mut summary.dirs_created,
            &mut summary.dirs_removed,
        ] {
            *counter = Counter::default();
        }
        summary.bytes_added = 0;
        summary.bytes_modified = 0;
        summary.cancelled = 0;

        for action in &self.actions {
            let counter = match action.action {
                ActionKind::Copy => &mut summary.added,
                ActionKind::Overwrite => &mut summary.modified,
                ActionKind::Delete => &mut summary.removed,
                ActionKind::UpdateTime => &mut summary.time_updated,
                ActionKind::RenameCase => &mut summary.renamed,
                ActionKind::CreateDir => &mut summary.dirs_created,
                ActionKind::RemoveDir => &mut summary.dirs_removed,
            };
            counter.planned += 1;
            match action.status {
                ActionStatus::Pending => {}
                ActionStatus::Done => counter.done += 1,
                ActionStatus::Failed => counter.failed += 1,
                ActionStatus::Skipped => counter.skipped += 1,
            }
            match action.action {
                ActionKind::Copy => summary.bytes_added += action.size.unwrap_or(0),
                ActionKind::Overwrite => summary.bytes_modified += action.size.unwrap_or(0),
                _ => {}
            }
            if action.problem == Some(ProblemReason::Cancelled) {
                summary.cancelled += 1;
            }
        }

        summary.skipped_entries = self.skipped.len() as u64;
        summary.problems = ProblemSummary::default();
        for problem in &self.problems {
            summary.problems.total += 1;
            *summary.problems.by_reason.entry(problem.reason).or_default() += 1;
        }
    }

    /// Resets everything not done yet, so a plan file (even a partially executed one) can run again.
    pub fn prepare_for_execution(&mut self) {
        for action in &mut self.actions {
            if action.status != ActionStatus::Done {
                action.status = ActionStatus::Pending;
                action.problem = None;
                action.attempts = None;
                action.error = None;
            }
        }
        self.problems.retain(|p| p.stage == Stage::Analyze);
        self.execution = None;
        self.refresh_summary();
    }

    /// 0 = in sync / success, 1 = differences found (not synced), 2 = problems, 3 = blocked.
    pub fn exit_code(&self) -> i32 {
        if !self.blockers.is_empty() {
            return EXIT_ABORTED;
        }
        if self.summary.problems.total > 0 {
            return EXIT_PROBLEMS;
        }
        match &self.execution {
            Some(execution) if execution.cancelled => EXIT_PROBLEMS,
            Some(_) => EXIT_OK,
            None if self.has_pending_actions() => EXIT_DIFFERENCES,
            None => EXIT_OK,
        }
    }
}

pub fn exit_code_meaning(code: i32) -> &'static str {
    match code {
        EXIT_OK => "success, folders are in sync",
        EXIT_DIFFERENCES => "differences found, nothing changed",
        EXIT_PROBLEMS => "completed with problems",
        _ => "aborted",
    }
}
