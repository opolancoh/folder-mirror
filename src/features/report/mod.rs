//! Report feature: writes the result file (JSON or CSV) and reads JSON plans back.

mod csv;
mod html;
mod json;

pub use html::write_html;
pub use json::read_plan;

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::Local;

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

/// `mirror-result-2026-10-08_143000.json` in the current folder.
pub fn default_report_path(format: ReportFormat) -> PathBuf {
    PathBuf::from(format!(
        "mirror-result-{}.{}",
        Local::now().format("%Y-%m-%d_%H%M%S"),
        format.extension()
    ))
}

pub fn write_report(plan: &SyncPlan, path: &Path, format: ReportFormat) -> Result<()> {
    match format {
        ReportFormat::Json => json::write_plan(plan, path),
        ReportFormat::Csv => csv::write_csv(plan, path),
    }
}
