//! The generated action model: a [`Command`] the harness can execute, bundled
//! with an arbitrary auth subset and an optional time-advance into a
//! [`Step`], and a bounded sequence of steps into a [`Run`].

use crate::adapter::ContractAdapter;
use crate::auth::{AddressPool, AuthSelection};
use arbitrary::Arbitrary;
use soroban_sdk::{Address, Env};

/// Everything a [`Command`] needs to execute itself against the live
/// contract.
pub struct ExecContext<'a> {
    pub env: &'a Env,
    pub contract_id: &'a Address,
    pub addresses: &'a AddressPool,
}

/// What happened when a [`Command`] was executed.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The call succeeded.
    Ok,
    /// The call failed with an error the contract declares (via
    /// `#[contracterror]` / `panic_with_error!`) — this is expected,
    /// well-behaved failure, not a finding.
    DeclaredError(u32),
    /// The host rejected the call without reaching the contract's own
    /// error type and without panicking — most commonly a failed
    /// `require_auth` (the authorized-address subset didn't include the
    /// address the contract required), or a malformed/oversized argument.
    /// Like `DeclaredError`, this is an expected failure mode, not a
    /// finding: it's what `Command::execute` should produce for a
    /// `try_*` call whose outer `Result` is `Err` for a reason other than
    /// a declared contract error.
    Rejected(String),
    /// The call panicked without going through the contract's declared
    /// error type. The harness's run loop sets this automatically when
    /// [`Command::execute`] unwinds; it is always a finding.
    UndeclaredPanic(String),
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok)
    }
}

/// A single generatable, executable action against the contract under test.
///
/// contributors: implement this for your contract's action enum. Derive
/// `Arbitrary` on that enum (composing field types from
/// `soro-fuzz-strategies` where useful) so the harness can generate it; this
/// trait describes how to run one and how it affects the shadow model.
pub trait Command<A: ContractAdapter>: core::fmt::Debug + Clone {
    /// Execute this command against the live contract in `ctx.env`.
    ///
    /// `authorizers` is the (already pool-resolved) set of addresses this
    /// step should authorize. Implementations build their own `MockAuth`
    /// invocation tree from it (via `ctx.env.mock_auths(..)`) since the
    /// shape of that tree is inherently specific to the function being
    /// called, then invoke the contract, preferring the generated client's
    /// `try_*` method so a declared error surfaces as `Result::Err` rather
    /// than an unwind. Any panic that nonetheless escapes is caught by the
    /// harness's run loop and reported as [`Outcome::UndeclaredPanic`].
    fn execute(&self, ctx: &ExecContext, authorizers: &[Address]) -> Outcome;

    /// Update the shadow model to reflect this command having been applied,
    /// given what actually happened. `outcome` matters: e.g. a
    /// `DeclaredError` outcome usually means the model should *not* be
    /// updated as though the call succeeded. `addresses` is provided so
    /// commands that only carry pool indices (e.g. built on
    /// `soro_fuzz_strategies::AddressIndex`) can resolve them to the same
    /// concrete `Address` keys the model uses, without having to
    /// re-derive or cache that mapping themselves.
    fn apply_to_model(&self, model: &mut A::Model, addresses: &AddressPool, outcome: &Outcome);
}

/// One generated step in a run: a command, who authorizes it, and an
/// optional ledger-time advance to apply immediately before executing it.
#[derive(Arbitrary, Debug, Clone)]
pub struct Step<C> {
    pub command: C,
    pub authorized: AuthSelection,
    /// Seconds to advance the ledger timestamp by before executing, if any.
    /// Bounded to a moderate range so runs don't trivially jump to u64::MAX.
    pub advance_time: Option<u32>,
}

/// A bounded sequence of steps to execute against one fresh contract
/// instance. Length is generated unbounded by `Arbitrary` and truncated to
/// `Harness`'s configured `commands_per_run` at run time (see
/// [`crate::harness::Harness::run`]), since `Arbitrary` generation has no
/// access to the harness's runtime configuration.
#[derive(Arbitrary, Debug, Clone)]
pub struct Run<C> {
    pub steps: Vec<Step<C>>,
}
