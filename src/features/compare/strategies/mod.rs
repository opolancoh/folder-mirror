//! One file per comparison mode.

mod full_hash;
mod metadata;
mod name;
mod quick_hash;
mod smart;

pub use full_hash::FullHashComparator;
pub use metadata::MetadataComparator;
pub use name::NameComparator;
pub use quick_hash::QuickHashComparator;
pub use smart::SmartComparator;

use super::comparator::{ChangeReason, CompareContext, CompareError, HashKind, Verdict};
use crate::features::scan::FileEntry;

/// Shared by the hash modes: size first, then content, then time.
fn compare_by_hash(
    source: &FileEntry,
    destination: &FileEntry,
    ctx: &CompareContext,
    kind: HashKind,
) -> Result<Verdict, CompareError> {
    if source.size != destination.size {
        return Ok(Verdict::Modified(ChangeReason::SizeChanged));
    }
    if !ctx.same_content(source, destination, kind)? {
        return Ok(Verdict::Modified(ChangeReason::ContentChanged));
    }
    if ctx.same_time(source, destination) {
        Ok(Verdict::Unchanged)
    } else {
        Ok(Verdict::TimeOnly)
    }
}
