//! `soro-fuzz-strategies`: reusable `Arbitrary` value types for common
//! Soroban argument shapes. Compose these into your contract's `Command`
//! enum fields instead of hand-rolling generation for the same handful of
//! shapes every contributor needs (bounded amounts, address references,
//! bounded maps, ledger-time advances).
//!
//! This crate has no knowledge of any specific contract.
//!
//! contributors: add a strategy for a new type here as its own module (see
//! `amounts.rs` for the bias-toward-edge-cases pattern most numeric
//! strategies should follow), then re-export it below. One strategy per
//! issue — see CONTRIBUTING.md.

pub mod address_ref;
pub mod amounts;
pub mod bytes32;
pub mod map;
pub mod time;

pub use address_ref::AddressIndex;
pub use amounts::{BoundedI128, NonNegativeI128};
pub use bytes32::Bytes32;
pub use map::BoundedEntries;
pub use time::TimeAdvance;
