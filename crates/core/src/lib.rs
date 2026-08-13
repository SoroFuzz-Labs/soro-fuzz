//! `soro-fuzz-core`: the contract-agnostic fuzzing/property-testing engine.
//!
//! This crate has no knowledge of any specific contract (no token
//! semantics, no escrow semantics). It defines the extension-point traits
//! contributors implement per contract ([`ContractAdapter`], [`Command`],
//! [`ReferenceModel`], [`Invariant`]) and the [`Harness`] run loop that ties
//! them together: generate a command sequence with an arbitrary auth
//! subset, execute each step against the contract in a real `soroban-sdk`
//! host `Env`, update a shadow model, and check invariants after every
//! step.
//!
//! See the workspace README for the full execution-model diagram and a
//! walkthrough of wiring up a new contract.

pub mod adapter;
pub mod auth;
pub mod command;
pub mod harness;
pub mod invariant;
pub mod model;

pub use adapter::ContractAdapter;
pub use auth::{AddressPool, AuthSelection};
pub use command::{Command, ExecContext, Outcome, Run, Step};
pub use harness::Harness;
pub use invariant::{Invariant, InvariantCtx, Violation};
pub use model::ReferenceModel;
