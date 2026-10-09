use super::{compare_by_hash, CompareContext, CompareError, HashKind, Verdict};
use crate::features::compare::Comparator;
use crate::features::scan::FileEntry;

/// Size, then a hash of three sampled blocks (small files are hashed fully).
pub struct QuickHashComparator;

impl Comparator for QuickHashComparator {
    fn compare(
        &self,
        source: &FileEntry,
        destination: &FileEntry,
        ctx: &CompareContext,
    ) -> Result<Verdict, CompareError> {
        compare_by_hash(source, destination, ctx, HashKind::Sampled)
    }
}
