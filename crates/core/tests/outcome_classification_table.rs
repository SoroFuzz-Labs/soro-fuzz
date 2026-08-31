//! Table-driven direct-unit coverage for `Outcome::from_try_result`'s
//! auth-vs-undeclared-panic classification (`crates/core/src/command.rs`).
//! `undeclared_panic_is_a_finding.rs` proves one path end-to-end through a
//! live `Harness::run`; this table exercises every classification branch
//! directly against the function itself, the way `command.rs`'s own inline
//! unit tests do, just gathered into one table instead of four functions.

use core::convert::Infallible;

use soro_fuzz_core::Outcome;
use soroban_sdk::InvokeError;

type ProbeResult = Result<Result<i64, Infallible>, Result<u32, InvokeError>>;

struct Case {
    name: &'static str,
    required_auth_satisfied: bool,
    result: ProbeResult,
    expect: fn(&Outcome) -> bool,
}

#[test]
fn from_try_result_classifies_every_branch() {
    let cases = [
        Case {
            name: "auth-not-required",
            required_auth_satisfied: true,
            result: Ok(Ok(1)),
            expect: |o| matches!(o, Outcome::Ok),
        },
        // Same code path as "auth-not-required" above: `from_try_result`
        // takes a single `required_auth_satisfied` bool and has no way to
        // tell "no auth needed" apart from "auth needed and satisfied" —
        // both are passed as `true`. Kept as its own row for documentation
        // of the two real-world scenarios, not because the function
        // distinguishes them.
        Case {
            name: "auth-required-and-satisfied",
            required_auth_satisfied: true,
            result: Ok(Ok(2)),
            expect: |o| matches!(o, Outcome::Ok),
        },
        Case {
            name: "auth-required-and-withheld",
            required_auth_satisfied: false,
            result: Err(Err(InvokeError::Abort)),
            expect: |o| matches!(o, Outcome::Rejected(_)),
        },
        Case {
            name: "declared-error passthrough",
            required_auth_satisfied: true,
            result: Err(Ok(7)),
            expect: |o| matches!(o, Outcome::DeclaredError(7)),
        },
        // The case this function exists to fix: `soroban-sdk` returns the
        // exact same `InvokeError::Abort` for a genuine undeclared contract
        // panic as it does for a failed require_auth. With auth satisfied,
        // an Abort can only be the former and must be flagged as a finding.
        Case {
            name: "undeclared panic despite auth satisfied",
            required_auth_satisfied: true,
            result: Err(Err(InvokeError::Abort)),
            expect: |o| matches!(o, Outcome::UndeclaredPanic(_)),
        },
    ];

    for case in cases {
        let outcome = Outcome::from_try_result(case.result, case.required_auth_satisfied, |e| e);
        assert!(
            (case.expect)(&outcome),
            "case {}: unexpected outcome {outcome:?}",
            case.name,
        );
    }
}
