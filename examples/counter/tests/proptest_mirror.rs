//! CI-friendly mirror of `fuzz/fuzz_targets/counter_fuzz.rs`: same `Run<C>`
//! type, same `Harness`, driven by `proptest-arbitrary-interop` instead of
//! `cargo-fuzz` so it runs under plain `cargo test` on stable.

use proptest::prelude::*;
use proptest_arbitrary_interop::arb;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_counter::harness::{
    CounterAdapter, CounterCommand, CounterValueMatchesModel,
};

fn harness() -> Harness<CounterAdapter> {
    Harness::<CounterAdapter>::new().with_invariant(CounterValueMatchesModel)
}

proptest! {
    #[test]
    fn counter_never_diverges_from_model(run in arb::<Run<CounterCommand>>()) {
        if let Err(violation) = harness().run(run) {
            prop_assert!(false, "{violation}");
        }
    }
}
