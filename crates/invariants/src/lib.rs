//! `soro-fuzz-invariants`: a small library of ready-made [`Invariant`]
//! implementations, generic over any [`ContractAdapter`] whose
//! [`ReferenceModel`] exposes the right accessor trait.
//!
//! `no-undeclared-panic` is deliberately *not* one of these: the harness
//! enforces it unconditionally as a structural guarantee (see
//! `soro-fuzz-core::Harness::run`), not as something a contributor could
//! accidentally leave disabled.
//!
//! contributors: add a new invariant here as its own module + a small
//! accessor trait describing what it needs from a `ReferenceModel` (see
//! `no_negative_balance::HasBalances` for the pattern), then re-export it
//! below. One invariant per issue — see CONTRIBUTING.md.

pub mod auth_required_for_mutation;
pub mod no_negative_balance;
pub mod supply_conservation;

pub use auth_required_for_mutation::{AuthRequiredForMutation, RequiresAuthorizer};
pub use no_negative_balance::{HasBalances, NoNegativeBalance};
pub use supply_conservation::{HasTotalSupply, SupplyConservation};

// contributors: add a new invariant here — a struct + an `impl<A>
// Invariant<A> for YourInvariant where A: ContractAdapter, A::Model:
// YourAccessorTrait` block, gated on a small trait describing what it needs
// from the model rather than on any specific contract.

/// Minimal fake [`ContractAdapter`]/[`Command`](soro_fuzz_core::Command) so
/// each invariant's unit tests can build a real [`InvariantCtx`] against a
/// throwaway model, without needing an actual running contract.
#[cfg(test)]
pub(crate) mod test_support {
    use soro_fuzz_core::{AddressPool, Command, ContractAdapter, ExecContext, Outcome, ReferenceModel};
    use soroban_sdk::Address;
    use std::marker::PhantomData;

    /// Generic over both the command and model type so each invariant's
    /// tests can plug in whatever fake command behavior they need (e.g.
    /// `auth_required_for_mutation`'s tests need a command that sometimes
    /// requires a specific authorizer) while sharing one adapter shape.
    pub(crate) struct FakeAdapter<C, M>(PhantomData<(C, M)>);

    impl<C, M> ContractAdapter for FakeAdapter<C, M>
    where
        C: Command<FakeAdapter<C, M>>,
        M: ReferenceModel,
    {
        type Command = C;
        type Model = M;

        fn setup(
            _env: &soroban_sdk::Env,
            _addresses: &soro_fuzz_core::AddressPool,
        ) -> (Address, Self::Model) {
            unimplemented!("test_support::FakeAdapter is only used to build InvariantCtx values directly")
        }
    }

    #[derive(Debug, Clone)]
    pub(crate) struct FakeCommand;

    impl<M: ReferenceModel> Command<FakeAdapter<FakeCommand, M>> for FakeCommand {
        fn execute(&self, _ctx: &ExecContext, _authorizers: &[Address]) -> Outcome {
            Outcome::Ok
        }
        fn apply_to_model(&self, _model: &mut M, _addresses: &AddressPool, _outcome: &Outcome) {}
    }
}
