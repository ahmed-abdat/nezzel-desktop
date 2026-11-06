//! Pure extraction decisions, kept separate from engine I/O for deterministic fallback.

pub mod chain;
pub(crate) mod deadline;
pub mod outcome;
mod page_scan;
pub mod policy;
pub mod precedence;
pub(crate) mod probe;
pub(crate) mod registry;
pub(crate) mod terminal;
