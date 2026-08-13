#![no_main]

use libfuzzer_sys::fuzz_target;

use soro_fuzz_core::{Harness, Run};
use soro_fuzz_example_counter::harness::{CounterAdapter, CounterCommand, CounterValueMatchesModel};

// libFuzzer/cargo-fuzz only registers a crash for a run that panics, so
// unlike the proptest mirror (which can just assert!/return a failed
// TestCaseError), a `Violation` here has to become an actual panic.
fuzz_target!(|run: Run<CounterCommand>| {
    let harness = Harness::<CounterAdapter>::new().with_invariant(CounterValueMatchesModel);
    if let Err(violation) = harness.run(run) {
        panic!("{violation}");
    }
});
