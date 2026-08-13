//! The shadow ("reference") state trait.

/// Marker trait for a contract's shadow state.
///
/// contributors: define a plain struct capturing whatever the invariants you
/// care about need to know (balances, an admin address, a counter value,
/// ...) and implement this marker on it. The actual mutation logic lives in
/// each [`Command::apply_to_model`](crate::command::Command::apply_to_model)
/// impl, one command variant at a time, rather than in one big match here —
/// that way adding a command doesn't require touching a central function.
///
/// No `Default` bound: the model's *initial* state comes from
/// [`ContractAdapter::setup`](crate::adapter::ContractAdapter::setup)
/// constructing it directly (e.g. from the address pool), not from
/// `Default::default()` — which matters because most non-trivial models
/// need to store at least one `soroban_sdk::Address`, and `Address` has no
/// meaningful default without a live `Env`.
pub trait ReferenceModel: Clone + core::fmt::Debug {}
