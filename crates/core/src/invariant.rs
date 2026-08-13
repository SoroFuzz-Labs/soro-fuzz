//! Post-step invariant checking.

use crate::adapter::ContractAdapter;
use crate::auth::AddressPool;
use crate::command::Outcome;
use soroban_sdk::{Address, Env};

/// Everything an [`Invariant`] can look at after a step has executed.
pub struct InvariantCtx<'a, A: ContractAdapter> {
    pub env: &'a Env,
    pub contract_id: &'a Address,
    pub model: &'a A::Model,
    pub command: &'a A::Command,
    pub last_outcome: &'a Outcome,
    /// The addresses that authorized the step just executed (post pool
    /// resolution), so invariants like "mutation requires auth" can check
    /// who actually signed.
    pub authorizers: &'a [Address],
    /// The run's full address pool, so an invariant (or an accessor trait a
    /// command implements, e.g. `soro-fuzz-invariants`'s
    /// `RequiresAuthorizer`) can resolve pool-index-based command fields
    /// (like `soro_fuzz_strategies::AddressIndex`) the same way
    /// `Command::execute`/`apply_to_model` do.
    pub addresses: &'a AddressPool,
    pub step_index: usize,
}

/// A single invariant failure, reported with enough context to reproduce it.
#[derive(Debug, Clone)]
pub struct Violation {
    pub invariant: &'static str,
    pub message: String,
    pub step_index: usize,
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "invariant `{}` violated at step {}: {}",
            self.invariant, self.step_index, self.message
        )
    }
}

impl std::error::Error for Violation {}

/// A property that must hold after every step.
///
/// contributors: implement this either generically (see
/// `soro-fuzz-invariants` for reusable ones, gated behind small accessor
/// traits like `HasBalances` so they apply to any adapter whose model
/// exposes the right data) or inline in your example for
/// contract-specific checks.
pub trait Invariant<A: ContractAdapter> {
    fn name(&self) -> &'static str;
    fn check(&self, ctx: &InvariantCtx<A>) -> Result<(), Violation>;
}
