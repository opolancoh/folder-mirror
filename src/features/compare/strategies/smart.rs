use super::{ChangeReason, CompareContext, CompareError, HashKind, Verdict};
use crate::features::compare::Comparator;
use crate::features::scan::FileEntry;

/// Size first; equal size and time means unchanged. Only when the size matches but the time
/// differs is the content read, using a full hash: a sampled hash could miss an edit in the
/// middle of a file whose timestamp did change.
pub struct SmartComparator;

impl Comparator for SmartComparator {
    fn compare(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        ctx: &CompareContext,
    ) -> Result<Verdict, CompareError> {
        if source.size != destination.size {
            return Ok(Verdict::Modified(ChangeReason::SizeChanged));
        }
        if ctx.same_time(source, destination) {
            return Ok(Verdict::Unchanged);
        }
        if ctx.same_content(source, destination, HashKind::Full)? {
            Ok(Verdict::TimeOnly)
        } else {
            Ok(Verdict::Modified(ChangeReason::ContentChanged))
        }
    }
}
