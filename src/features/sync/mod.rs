//! Sync feature: executes a plan on the destination. Never stops on a failed file unless asked to.

mod copier;
mod deleter;
mod executor;

pub use deleter::{prune_trash, Deleter};
pub use executor::execute;
