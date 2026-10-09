//! Safety checks that block a sync before anything is changed.

use super::model::SyncPlan;
use crate::shared::config::Settings;
use crate::shared::files;
use crate::shared::format::format_bytes;

/// The deletion limit only applies from this many deletions, so small folders are not blocked
/// by a single removed file.
pub const MIN_DELETIONS_CHECKED: u64 = 10;

pub fn check_safety(plan: &mut SyncPlan, settings: &Settings) {
    let removed = plan.summary.removed.planned;
    let total = plan.summary.destination_files;
    if removed >= MIN_DELETIONS_CHECKED && total > 0 {
        let percent = removed as f64 * 100.0 / total as f64;
        if percent > settings.max_delete_percent {
            plan.blockers.push(format!(
                "{removed} of {total} destination files ({percent:.1}%) would be deleted, above the --max-delete limit of {}%. \
                 Check that the source path is correct, or raise --max-delete if this is intended",
                settings.max_delete_percent
            ));
        }
    }

    let needed = plan.summary.bytes_added + plan.summary.bytes_modified;
    if needed > 0 {
        match files::available_space(&plan.destination_root) {
            Ok(free) if needed > free => plan.blockers.push(format!(
                "not enough free space on the destination: {} needed, {} available",
                format_bytes(needed),
                format_bytes(free)
            )),
            Ok(_) => {}
            Err(err) => plan.warnings.push(format!("could not check free space on the destination: {err}")),
        }
    }
}
