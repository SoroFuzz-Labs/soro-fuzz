//! CI-friendly mirror of `fuzz/fuzz_targets/escrow_fuzz.rs`.

use proptest::prelude::*;
use proptest_arbitrary_interop::arb;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_escrow::harness::{EscrowAdapter, EscrowCommand, EscrowStatusMatchesModel};

fn harness() -> Harness<EscrowAdapter> {
    Harness::<EscrowAdapter>::new().with_invariant(EscrowStatusMatchesModel)
}

proptest! {
    #[test]
    fn escrow_status_never_diverges_from_model(run in arb::<Run<EscrowCommand>>()) {
        if let Err(violation) = harness().run(run) {
            prop_assert!(false, "{violation}");
        }
    }
}
