use std::path::Path;

use crate::features::plan::SyncPlan;
use crate::shared::error::{MirrorError, Problem, Result, Stage};

const HEADER: [&str; 9] = [
    "action", "reason", "status", "source", "destination", "size", "problem", "attempts", "error",
];

/// One row per action, then analysis problems and skipped entries.
pub fn write_csv(plan: &SyncPlan, path: &Path) -> Result<()> {
    let mut writer = csv::Writer::from_path(path).map_err(report_error)?;
    writer.write_record(HEADER).map_err(report_error)?;

    for action in &plan.actions {
        let size = action.size.map(|s| s.to_string()).unwrap_or_default();
        let attempts = action.attempts.map(|a| a.to_string()).unwrap_or_default();
        writer
            .write_record([
                action.action.as_str(),
                action.reason.as_str(),
                action.status.as_str(),
                action.source.as_deref().unwrap_or(""),
                &action.destination,
                &size,
                action.problem.map(|p| p.as_str()).unwrap_or(""),
                &attempts,
                action.error.as_deref().unwrap_or(""),
            ])
            .map_err(report_error)?;
    }
    for problem in plan.problems.iter().filter(|p| p.stage == Stage::Analyze) {
        write_problem(&mut writer, "problem", problem)?;
    }
    for entry in &plan.skipped {
        write_problem(&mut writer, "skipped", entry)?;
    }

    writer.flush()?;
    Ok(())
}

fn write_problem(writer: &mut csv::Writer<std::fs::File>, kind: &str, problem: &Problem) -> Result<()> {
    writer
        .write_record([
            kind,
            problem.reason.as_str(),
            kind,
            problem.source.as_deref().unwrap_or(""),
            problem.destination.as_deref().unwrap_or(""),
            "",
            problem.reason.as_str(),
            "",
            &problem.error,
        ])
        .map_err(report_error)
}

fn report_error(err: csv::Error) -> MirrorError {
    MirrorError::Report(err.to_string())
}
