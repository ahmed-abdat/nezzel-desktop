//! Pure extraction decisions, kept separate from engine I/O for deterministic fallback.
// U1 intentionally defines the contract before U2 wires it into the fetch coordinator.
// TODO(U2): remove once the chain wires this module
#![allow(dead_code)]

pub mod chain;
pub mod outcome;
pub mod policy;
pub mod precedence;
pub(crate) mod probe;
pub(crate) mod registry;
