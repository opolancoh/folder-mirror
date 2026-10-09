//! Business logic, one module per feature. Data flows in one direction:
//! `scan` → `compare` → `plan` → `sync` → `report`.

pub mod compare;
pub mod plan;
pub mod report;
pub mod scan;
pub mod sync;
