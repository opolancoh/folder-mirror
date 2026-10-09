use super::{compare_by_hash, CompareContext, CompareError, HashKind, Verdict};
use crate::features::compare::Comparator;
use crate::features::scan::FileEntry;

/// Size, then a hash of the whole file on both sides. Exact, but reads everything.
pub struct FullHashComparator;

impl Comparator for FullHashComparator {
    fn compare(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        ctx: &CompareContext,
    ) -> Result<Verdict, CompareError> {
        compare_by_hash(source, destination, ctx, HashKind::Full)
    }
}
