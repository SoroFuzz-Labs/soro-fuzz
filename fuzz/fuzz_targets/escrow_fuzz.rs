#![no_main]

use libfuzzer_sys::fuzz_target;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_escrow::harness::{EscrowAdapter, EscrowCommand, EscrowStatusMatchesModel};

fuzz_target!(|run: Run<EscrowCommand>| {
    let harness = Harness::<EscrowAdapter>::new().with_invariant(EscrowStatusMatchesModel);
    if let Err(violation) = harness.run(run) {
        panic!("{violation}");
    }
});
