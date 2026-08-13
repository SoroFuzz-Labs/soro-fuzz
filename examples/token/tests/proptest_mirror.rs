//! CI-friendly mirror of `fuzz/fuzz_targets/token_fuzz.rs`.

use proptest::prelude::*;
use proptest_arbitrary_interop::arb;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_token::harness::{TokenAdapter, TokenCommand};
use soro_fuzz_invariants::{AuthRequiredForMutation, NoNegativeBalance, SupplyConservation};

fn harness() -> Harness<TokenAdapter> {
    Harness::<TokenAdapter>::new()
        .with_invariant(NoNegativeBalance)
        .with_invariant(SupplyConservation)
        .with_invariant(AuthRequiredForMutation)
}

proptest! {
    #[test]
    fn token_balances_and_supply_stay_consistent(run in arb::<Run<TokenCommand>>()) {
        if let Err(violation) = harness().run(run) {
            prop_assert!(false, "{violation}");
        }
    }
}
