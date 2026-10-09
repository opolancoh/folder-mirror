use super::{CompareContext, CompareError, Verdict};
use crate::features::compare::Comparator;
use crate::features::scan::FileEntry;

/// Only presence matters: a file on both sides is always unchanged.
pub struct NameComparator;

impl Comparator for NameComparator {
    fn compare(&self, _: &FileEntry, _: &FileEntry, _: &CompareContext) -> Result<Verdict, CompareError> {
        Ok(Verdict::Unchanged)
    }
}
