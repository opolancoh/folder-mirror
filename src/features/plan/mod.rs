//! Plan feature: the list of actions to make the destination identical, and safety checks.

mod model;
mod planner;
mod safety;

pub use model::*;
pub use planner::build_plan;
pub use safety::{check_safety, MIN_DELETIONS_CHECKED};
