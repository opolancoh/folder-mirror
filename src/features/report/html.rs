//! Self-contained HTML report (no external files). The plan is embedded as JSON and rendered in
//! the browser, which keeps this side simple and allows filtering and searching.

use std::fs;
use std::path::Path;

use crate::features::plan::SyncPlan;
use crate::shared::error::{MirrorError, Result};

const TEMPLATE: &str = include_str!("report.html");

pub fn write_html(plan: &SyncPlan, path: &Path, exit_code: i32) -> Result<()> {
    let data = serde_json::json!({ "exit_code": exit_code, "plan": plan });
    let json = serde_json::to_string(&data).map_err(|e| MirrorError::Report(e.to_string()))?;
    // Keep the JSON from closing the <script> element early.
    let json = json.replace("</", "<\\/");
    fs::write(path, TEMPLATE.replace("__REPORT_DATA__", &json))?;
    Ok(())
}
