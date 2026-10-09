use super::{ChangeReason, CompareContext, CompareError, Verdict};
use crate::features::compare::Comparator;
use crate::features::scan::FileEntry;

/// Size, then modification time. Never reads file content.
pub struct MetadataComparator;

impl Comparator for MetadataComparator {
    fn compare(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        ctx: &CompareContext,
    ) -> Result<Verdict, CompareError> {
        if source.size != destination.size {
            Ok(Verdict::Modified(ChangeReason::SizeChanged))
        } else if !ctx.same_time(source, destination) {
            Ok(Verdict::Modified(ChangeReason::TimeChanged))
        } else {
            Ok(Verdict::Unchanged)
        }
    }
}
