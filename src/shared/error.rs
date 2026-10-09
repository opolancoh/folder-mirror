//! Error types: fatal errors that stop the program, and per-file problems that are recorded and reported.

use std::fmt;
use std::io;

use serde::{Deserialize, Serialize};

/// Errors that stop the whole operation.
#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    #[error("source folder not found: {0}")]
    SourceNotFound(String),

    #[error("not a folder: {0}")]
    NotADirectory(String),

    #[error("source and destination overlap (one is inside the other): {0} / {1}")]
    Overlap(String, String),

    #[error("cannot read folder {path}: {source}")]
    Unreadable {
        path: String,
        #[source]
        source: io::Error,
    },

    #[error("configuration error: {0}")]
    Config(String),

    #[error("plan file error: {0}")]
    PlanFile(String),

    #[error("result file error: {0}")]
    Report(String),

    #[error("sync blocked by safety checks:\n  - {}", .0.join("\n  - "))]
    Blocked(Vec<String>),

    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, MirrorError>;

/// Why a file could not be processed (or was intentionally skipped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemReason {
    FileLocked,
    PermissionDenied,
    NotFound,
    PathTooLong,
    DiskFull,
    InvalidName,
    CaseConflict,
    ChangedDuringCopy,
    SourceChangedSincePlan,
    SourceUnreadable,
    DestinationUnreadable,
    VerifyFailed,
    Symlink,
    SpecialFile,
    CloudPlaceholder,
    Cancelled,
    IoError,
}

impl ProblemReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileLocked => "file_locked",
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::PathTooLong => "path_too_long",
            Self::DiskFull => "disk_full",
            Self::InvalidName => "invalid_name",
            Self::CaseConflict => "case_conflict",
            Self::ChangedDuringCopy => "changed_during_copy",
            Self::SourceChangedSincePlan => "source_changed_since_plan",
            Self::SourceUnreadable => "source_unreadable",
            Self::DestinationUnreadable => "destination_unreadable",
            Self::VerifyFailed => "verify_failed",
            Self::Symlink => "symlink",
            Self::SpecialFile => "special_file",
            Self::CloudPlaceholder => "cloud_placeholder",
            Self::Cancelled => "cancelled",
            Self::IoError => "io_error",
        }
    }

    /// Temporary conditions worth retrying after a short wait.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::FileLocked | Self::ChangedDuringCopy)
    }
}

impl fmt::Display for ProblemReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When a problem happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    #[default]
    Analyze,
    Sync,
}

/// A problem with one file or folder, with full paths for the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub reason: ProblemReason,
    #[serde(default)]
    pub stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts: Option<u32>,
    pub error: String,
}

impl Problem {
    pub fn new(reason: ProblemReason, error: impl Into<String>) -> Self {
        Self {
            reason,
            stage: Stage::Analyze,
            action: None,
            source: None,
            destination: None,
            attempts: None,
            error: error.into(),
        }
    }

    pub fn from_io(err: &io::Error) -> Self {
        Self::new(classify_io_error(err), err.to_string())
    }

    pub fn with_source(mut self, path: impl Into<String>) -> Self {
        self.source = Some(path.into());
        self
    }

    pub fn with_destination(mut self, path: impl Into<String>) -> Self {
        self.destination = Some(path.into());
        self
    }

    /// The most relevant full path to show to the user.
    pub fn path(&self) -> &str {
        self.source
            .as_deref()
            .or(self.destination.as_deref())
            .unwrap_or("")
    }
}

/// Maps an OS error to a problem reason the user can act on.
pub fn classify_io_error(err: &io::Error) -> ProblemReason {
    if let Some(reason) = err.raw_os_error().and_then(classify_os_code) {
        return reason;
    }
    match err.kind() {
        io::ErrorKind::PermissionDenied => ProblemReason::PermissionDenied,
        io::ErrorKind::NotFound => ProblemReason::NotFound,
        _ => ProblemReason::IoError,
    }
}

#[cfg(windows)]
fn classify_os_code(code: i32) -> Option<ProblemReason> {
    // https://learn.microsoft.com/windows/win32/debug/system-error-codes
    match code {
        32 | 33 => Some(ProblemReason::FileLocked), // sharing / lock violation
        5 => Some(ProblemReason::PermissionDenied),  // access denied
        39 | 112 => Some(ProblemReason::DiskFull),   // handle disk full / disk full
        111 | 206 => Some(ProblemReason::PathTooLong), // buffer overflow / filename exceeds range
        123 => Some(ProblemReason::InvalidName),     // invalid name
        _ => None,
    }
}

#[cfg(unix)]
fn classify_os_code(code: i32) -> Option<ProblemReason> {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    const ENAMETOOLONG: i32 = 63;
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
    const ENAMETOOLONG: i32 = 36;
    const ETXTBSY: i32 = 26;
    const ENOSPC: i32 = 28;

    match code {
        ETXTBSY => Some(ProblemReason::FileLocked),
        ENOSPC => Some(ProblemReason::DiskFull),
        ENAMETOOLONG => Some(ProblemReason::PathTooLong),
        _ => None,
    }
}

#[cfg(not(any(windows, unix)))]
fn classify_os_code(_code: i32) -> Option<ProblemReason> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_io_errors() {
        let not_found = io::Error::from(io::ErrorKind::NotFound);
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(classify_io_error(&not_found), ProblemReason::NotFound);
        assert_eq!(classify_io_error(&denied), ProblemReason::PermissionDenied);
        assert_eq!(classify_io_error(&io::Error::other("x")), ProblemReason::IoError);
    }

    #[cfg(windows)]
    #[test]
    fn classifies_windows_lock_and_disk_errors() {
        assert_eq!(classify_io_error(&io::Error::from_raw_os_error(32)), ProblemReason::FileLocked);
        assert_eq!(classify_io_error(&io::Error::from_raw_os_error(112)), ProblemReason::DiskFull);
    }

    #[test]
    fn only_temporary_problems_are_retried() {
        assert!(ProblemReason::FileLocked.is_retryable());
        assert!(ProblemReason::ChangedDuringCopy.is_retryable());
        assert!(!ProblemReason::PermissionDenied.is_retryable());
        assert!(!ProblemReason::NotFound.is_retryable());
    }

    #[test]
    fn problem_path_prefers_the_source() {
        let problem = Problem::new(ProblemReason::IoError, "x").with_destination("dst").with_source("src");
        assert_eq!(problem.path(), "src");
        assert_eq!(Problem::new(ProblemReason::IoError, "x").with_destination("dst").path(), "dst");
    }

    #[test]
    fn reasons_serialize_in_snake_case() {
        assert_eq!(serde_json::to_string(&ProblemReason::FileLocked).unwrap(), "\"file_locked\"");
        assert_eq!(ProblemReason::SourceChangedSincePlan.as_str(), "source_changed_since_plan");
    }
}
