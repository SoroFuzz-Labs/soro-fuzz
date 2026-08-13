//! A generatable reference to one of the harness's pool addresses.
//!
//! `soroban_sdk::Address` can't implement `Arbitrary` directly — building
//! one requires a live `Env`, which `arbitrary()` doesn't have access to.
//! `AddressIndex` sidesteps that: it's just a `u8`, resolved against the
//! run's `AddressPool` (modulo pool length) inside `Command::execute`,
//! exactly like `AuthSelection` resolves its indices.

use arbitrary::Arbitrary;

/// An arbitrary index into the run's `AddressPool`. Compose this into a
/// `Command` enum variant wherever the contract call needs an `Address`
/// argument (e.g. a token transfer's `to`), then resolve it via
/// `ctx.addresses.get(index.0 as usize)`.
#[derive(Arbitrary, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddressIndex(pub u8);
