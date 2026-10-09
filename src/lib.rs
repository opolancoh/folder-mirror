//! folder-mirror: keep a destination folder identical to a source folder.
//!
//! Layout:
//! - [`features`]: business logic, one module per feature (scan → compare → plan → sync → report).
//! - [`shared`]: reusable building blocks with no sync decisions (paths, errors, hashing, config...).
//! - [`app`]: use cases that chain the features; the CLI (and a future TUI) only call these.

pub mod app;
pub mod features;
pub mod shared;
