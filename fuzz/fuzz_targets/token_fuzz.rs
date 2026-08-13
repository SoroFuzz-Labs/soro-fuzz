#![no_main]

use libfuzzer_sys::fuzz_target;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_token::harness::TokenAdapter;
use soro_fuzz_example_token::harness::TokenCommand;
use soro_fuzz_invariants::{AuthRequiredForMutation, NoNegativeBalance, SupplyConservation};

fuzz_target!(|run: Run<TokenCommand>| {
    let harness = Harness::<TokenAdapter>::new()
        .with_invariant(NoNegativeBalance)
        .with_invariant(SupplyConservation)
        .with_invariant(AuthRequiredForMutation);
    if let Err(violation) = harness.run(run) {
        panic!("{violation}");
    }
});
