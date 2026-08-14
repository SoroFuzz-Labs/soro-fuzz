//! The generated action model: a [`Command`] the harness can execute, bundled
//! with an arbitrary auth subset and an optional time-advance into a
//! [`Step`], and a bounded sequence of steps into a [`Run`].

use crate::adapter::ContractAdapter;
use crate::auth::{AddressPool, AuthSelection};
use arbitrary::Arbitrary;
use soroban_sdk::{Address, Env, InvokeError};

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
    /// The host rejected the call for a reason other than a declared
    /// contract error, *and* the harness had deliberately withheld auth the
    /// call required — the expected, benign shape of a failed
    /// `require_auth`. Like `DeclaredError`, this is an expected failure
    /// mode, not a finding.
    ///
    /// `soroban-sdk` doesn't actually give a `try_*` caller enough
    /// information to tell "auth was correctly rejected" apart from "the
    /// contract panicked" — both surface identically as
    /// `InvokeError::Abort` — so this variant must only be constructed when
    /// the caller *independently* knows auth was withheld (see
    /// [`Outcome::from_try_result`]). Constructing it any other way
    /// reintroduces the false-negative this type exists to prevent.
    Rejected(String),
    /// The call panicked without going through the contract's declared
    /// error type, or the host aborted it for a reason that can't be
    /// attributed to auth being deliberately withheld. Set automatically by
    /// the harness's run loop when [`Command::execute`] unwinds, and by
    /// [`Outcome::from_try_result`] when a `try_*` call comes back
    /// `InvokeError::Abort` despite every required address having been
    /// authorized. Always a finding.
    UndeclaredPanic(String),
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Outcome::Ok)
    }

    /// Classifies a generated client `try_*` call's result into an
    /// [`Outcome`].
    ///
    /// `soroban-sdk` represents both a failed `require_auth` and an
    /// undeclared contract panic (a bare `panic!`, an `unwrap()`, an
    /// overflow, ...) identically: `Err(Err(InvokeError::Abort))`. There is
    /// no way to tell them apart from the `InvokeError` value alone — the
    /// harness has to supply the missing information itself, via
    /// `required_auth_satisfied`: whether *this specific call* needed no
    /// auth at all, or needed auth and every required address was in the
    /// step's `authorized` set. If so, an `Abort` cannot legitimately be an
    /// auth rejection, so it's classified as [`Outcome::UndeclaredPanic`] —
    /// a finding — instead of being silently absorbed as
    /// [`Outcome::Rejected`].
    ///
    /// contributors: use this from every `Command::execute` arm that calls
    /// a generated `try_*` client method, passing the same boolean your
    /// auth-mocking helper used to decide whether to mock real auth or
    /// `env.mock_auths(&[])` (see any example's `harness.rs`) — don't
    /// hand-match the nested `Result` yourself, since that's exactly how
    /// this false negative was introduced.
    pub fn from_try_result<T, ConvErr, E>(
        result: Result<Result<T, ConvErr>, Result<E, InvokeError>>,
        required_auth_satisfied: bool,
        code_of: impl FnOnce(E) -> u32,
    ) -> Outcome
    where
        ConvErr: core::fmt::Debug,
    {
        match result {
            Ok(Ok(_)) => Outcome::Ok,
            Ok(Err(conv_err)) => {
                panic!("unexpected value conversion error: {conv_err:?}")
            }
            Err(Ok(e)) => Outcome::DeclaredError(code_of(e)),
            Err(Err(invoke_err)) => {
                if required_auth_satisfied {
                    Outcome::UndeclaredPanic(format!(
                        "host aborted the call ({invoke_err:?}) despite every required \
                         address being authorized — soroban-sdk represents both a failed \
                         require_auth and an undeclared contract panic as \
                         InvokeError::Abort, and auth was satisfied here, so this can only \
                         be the latter"
                    ))
                } else {
                    Outcome::Rejected(format!("{invoke_err:?}"))
                }
            }
        }
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
    /// called, then invoke the contract via the generated client's `try_*`
    /// method and turn the result into an [`Outcome`] with
    /// [`Outcome::from_try_result`] — not by hand-matching the nested
    /// `Result`, since that's the mistake that makes undeclared panics
    /// indistinguishable from legitimate auth rejections (see that
    /// function's doc comment). Any panic that nonetheless escapes is
    /// caught by the harness's run loop and reported as
    /// [`Outcome::UndeclaredPanic`].
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::convert::Infallible;

    type ProbeResult = Result<Result<i64, Infallible>, Result<u32, InvokeError>>;

    #[test]
    fn success_is_ok() {
        let result: ProbeResult = Ok(Ok(42));
        assert!(Outcome::from_try_result(result, true, |e| e).is_ok());
    }

    #[test]
    fn declared_error_is_declared_error() {
        let result: ProbeResult = Err(Ok(7));
        let outcome = Outcome::from_try_result(result, true, |e| e);
        assert!(matches!(outcome, Outcome::DeclaredError(7)));
    }

    #[test]
    fn abort_with_auth_withheld_is_a_benign_rejection() {
        // The case a legitimate failed require_auth produces: the harness
        // knows it didn't satisfy the auth this call needed.
        let result: ProbeResult = Err(Err(InvokeError::Abort));
        let outcome = Outcome::from_try_result(result, false, |e| e);
        assert!(matches!(outcome, Outcome::Rejected(_)));
    }

    #[test]
    fn abort_despite_satisfied_auth_is_an_undeclared_panic() {
        // The case this function exists to fix: `soroban-sdk` returns the
        // exact same `InvokeError::Abort` for a genuine undeclared contract
        // panic as it does for a failed require_auth. When the harness
        // knows auth was satisfied (or wasn't needed), an Abort can only be
        // the former, and must be flagged as a finding rather than silently
        // absorbed as `Rejected`.
        let result: ProbeResult = Err(Err(InvokeError::Abort));
        let outcome = Outcome::from_try_result(result, true, |e| e);
        assert!(matches!(outcome, Outcome::UndeclaredPanic(_)));
    }
}
