//! Report feature: writes the result file (JSON or CSV) and reads JSON plans back.

mod csv;
mod html;
mod json;

pub use html::write_html;
pub use json::read_plan;

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Local};

use crate::features::plan::SyncPlan;
use crate::shared::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportFormat {
    /// Complete plan; can be executed later with `sync --plan`.
    #[default]
    Json,
    /// One row per action/problem, for spreadsheets.
    Csv,
}

impl ReportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
        }
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()?.to_str()?.parse().ok()
    }
}

impl FromStr for ReportFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            _ => Err(format!("unknown format '{value}' (expected: json, csv)")),
        }
    }
}

impl fmt::Display for ReportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.extension())
    }
}

/// Where a run's reports go: the result file and the HTML report share one name.
#[derive(Debug, Clone)]
pub struct ReportPaths {
    pub result: PathBuf,
    pub html: PathBuf,
}

/// Picks unique report names in `dir` (created if missing), based on `time` with its time zone:
/// `folder-mirror-2026-10-09T143000-0500.json` + `.html`. If that name is taken, `-2`, `-3`...
/// is added. The result file is created empty right away so no other run can take the name.
pub fn report_paths(dir: &Path, format: ReportFormat, time: DateTime<Local>) -> Result<ReportPaths> {
    fs::create_dir_all(dir)?;
    let base = format!("folder-mirror-{}", time.format("%Y-%m-%dT%H%M%S%z"));
    for attempt in 1u32.. {
        let stem = if attempt == 1 { base.clone() } else { format!("{base}-{attempt}") };
        let result = dir.join(format!("{stem}.{}", format.extension()));
        let html = dir.join(format!("{stem}.html"));
        if html.exists() {
            continue;
        }
        match OpenOptions::new().write(true).create_new(true).open(&result) {
            Ok(_) => return Ok(ReportPaths { result, html }),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    unreachable!("an unused report name always exists")
}

pub fn write_report(plan: &SyncPlan, path: &Path, format: ReportFormat) -> Result<()> {
    match format {
        ReportFormat::Json => json::write_plan(plan, path),
        ReportFormat::Csv => csv::write_csv(plan, path),
    }
}
