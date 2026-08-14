//! Regression test for `Outcome::from_try_result`: an undeclared panic
//! inside a contract method that declares `Result<T, E>` must surface as a
//! `Violation`, not be silently absorbed as a benign `Outcome::Rejected`.
//! Before `from_try_result` existed, every example classified `Err(Err(_))`
//! from a `try_*` call as `Rejected` unconditionally — which is also what
//! `soroban-sdk` returns for a real, undeclared `panic!` inside the
//! contract, since it represents a failed `require_auth` and an undeclared
//! panic identically as `InvokeError::Abort`. This drives a full
//! `Harness::run` against a live (host-registered) buggy contract to prove
//! the fix actually closes that gap end to end, not just at the unit level.

use soroban_sdk::{contract, contracterror, contractimpl, Address, Env};

use soro_fuzz_core::{
    AddressPool, AuthSelection, Command, ContractAdapter, ExecContext, Harness, Outcome,
    ReferenceModel, Run, Step,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
enum BuggyError {
    Never = 1,
}

#[contract]
struct BuggyContract;

#[contractimpl]
impl BuggyContract {
    // No auth involved at all, so any `InvokeError::Abort` coming out of a
    // call to this can only mean the contract panicked.
    pub fn boom(_env: Env) -> Result<i64, BuggyError> {
        panic!("undeclared bug for the regression test");
    }
}

struct BuggyAdapter;

#[derive(Debug, Clone, Default)]
struct BuggyModel;
impl ReferenceModel for BuggyModel {}

#[derive(Debug, Clone)]
struct Boom;

impl Command<BuggyAdapter> for Boom {
    fn execute(&self, ctx: &ExecContext, _authorizers: &[Address]) -> Outcome {
        let client = BuggyContractClient::new(ctx.env, ctx.contract_id);
        Outcome::from_try_result(client.try_boom(), true, |e| e as u32)
    }

    fn apply_to_model(&self, _model: &mut BuggyModel, _addresses: &AddressPool, _outcome: &Outcome) {}
}

impl ContractAdapter for BuggyAdapter {
    type Command = Boom;
    type Model = BuggyModel;

    fn setup(env: &Env, _addresses: &AddressPool) -> (Address, Self::Model) {
        (env.register(BuggyContract, ()), BuggyModel)
    }
}

#[test]
fn undeclared_panic_is_flagged_as_a_violation() {
    let run = Run {
        steps: vec![Step {
            command: Boom,
            authorized: AuthSelection::none(),
            advance_time: None,
        }],
    };
    let violation = Harness::<BuggyAdapter>::new()
        .run(run)
        .expect_err("an undeclared panic must be reported as a Violation, not run clean");
    assert_eq!(violation.invariant, "no-undeclared-panic");
}
