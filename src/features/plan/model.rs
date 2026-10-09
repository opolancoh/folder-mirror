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
    #[serde(default)]
    pub bytes_removed: u64,
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
    /// How the sync was started: "confirmed by the user" or "automatic (--confirm false)".
    #[serde(default)]
    pub confirmation: String,
    /// Warnings raised while syncing (the analysis ones stay in `SyncPlan::warnings`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// When the analysis ran.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalysisInfo {
    pub started_at: String,
    pub finished_at: String,
    pub duration_secs: f64,
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
    #[serde(default)]
    pub analysis: AnalysisInfo,
    pub summary: Summary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<Execution>,
    /// Why the changes were not applied (dry run, declined, blocked...). `None` with no
    /// `execution` means the run has not finished yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_executed: Option<String>,
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
        summary.bytes_removed = 0;
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
                ActionKind::Delete => summary.bytes_removed += action.size.unwrap_or(0),
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
        self.not_executed = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(actions: Vec<SyncAction>) -> SyncPlan {
        let mut plan = SyncPlan {
            tool_version: "test".into(),
            created_at: String::new(),
            source: "s".into(),
            destination: "d".into(),
            mode: CompareMode::Smart,
            delete_mode: DeleteMode::TrashFolder,
            case_insensitive: false,
            analysis: AnalysisInfo::default(),
            summary: Summary::default(),
            execution: None,
            not_executed: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            problems: Vec::new(),
            skipped: Vec::new(),
            actions,
            source_root: PathBuf::new(),
            destination_root: PathBuf::new(),
        };
        plan.refresh_summary();
        plan
    }

    fn copy(size: u64, status: ActionStatus) -> SyncAction {
        let mut action = SyncAction::new(ActionKind::Copy, ActionReason::Added, RelPath::new("a"), "d/a".into());
        action.size = Some(size);
        action.status = status;
        action
    }

    fn execution() -> Execution {
        Execution {
            started_at: String::new(),
            finished_at: String::new(),
            duration_secs: 0.0,
            delete_mode_used: DeleteMode::TrashFolder,
            delete_mode_fallback: None,
            trash_folder: None,
            cancelled: false,
            exit_code: 0,
            confirmation: String::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn summary_counts_actions_by_kind_and_status() {
        let plan = plan(vec![copy(10, ActionStatus::Done), copy(5, ActionStatus::Failed), copy(1, ActionStatus::Pending)]);
        let added = &plan.summary.added;
        assert_eq!((added.planned, added.done, added.failed), (3, 1, 1));
        assert_eq!(plan.summary.bytes_added, 16);
    }

    #[test]
    fn exit_codes_follow_the_documented_meaning() {
        assert_eq!(plan(vec![]).exit_code(), EXIT_OK);
        assert_eq!(plan(vec![copy(1, ActionStatus::Pending)]).exit_code(), EXIT_DIFFERENCES);

        let mut blocked = plan(vec![copy(1, ActionStatus::Pending)]);
        blocked.blockers.push("too many deletions".into());
        assert_eq!(blocked.exit_code(), EXIT_ABORTED);

        let mut with_problem = plan(vec![]);
        with_problem.problems.push(Problem::new(ProblemReason::FileLocked, "x"));
        with_problem.refresh_summary();
        assert_eq!(with_problem.exit_code(), EXIT_PROBLEMS);

        let mut done = plan(vec![copy(1, ActionStatus::Done)]);
        done.execution = Some(execution());
        assert_eq!(done.exit_code(), EXIT_OK);
        done.execution.as_mut().unwrap().cancelled = true;
        assert_eq!(done.exit_code(), EXIT_PROBLEMS);
    }

    #[test]
    fn prepare_for_execution_keeps_done_actions_and_resets_the_rest() {
        let mut failed = copy(1, ActionStatus::Failed);
        failed.error = Some("locked".into());
        let mut plan = plan(vec![copy(1, ActionStatus::Done), failed]);
        plan.execution = Some(execution());
        plan.not_executed = Some("declined".into());
        let mut sync_problem = Problem::new(ProblemReason::FileLocked, "x");
        sync_problem.stage = Stage::Sync;
        plan.problems.push(sync_problem);

        plan.prepare_for_execution();

        assert_eq!(plan.actions[0].status, ActionStatus::Done);
        assert_eq!(plan.actions[1].status, ActionStatus::Pending);
        assert!(plan.actions[1].error.is_none());
        assert!(plan.problems.is_empty() && plan.execution.is_none() && plan.not_executed.is_none());
    }
}
