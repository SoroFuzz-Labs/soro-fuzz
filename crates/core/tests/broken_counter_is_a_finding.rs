//! Regression test for state-divergence detection: an `Invariant` comparing
//! on-chain state against a shadow `ReferenceModel` must actually fail
//! `Harness::run` when a contract's real behavior diverges from what the
//! model predicts — not just when the invariant's `check()` is called
//! directly against a hand-built wrong model (see
//! `CounterValueMatchesModel`'s own unit test in `examples/counter`). This
//! drives a full `Harness::run` against a live, deliberately buggy counter
//! contract (`increment` adds 2 instead of 1) to prove that end to end,
//! mirroring what `undeclared_panic_is_a_finding.rs` does for the panic
//! path.

use soroban_sdk::{contract, contracterror, contractimpl, symbol_short, Address, Env, Symbol};

use soro_fuzz_core::{
    AddressPool, AuthSelection, Command, ContractAdapter, ExecContext, Harness, Invariant,
    InvariantCtx, Outcome, ReferenceModel, Run, Step, Violation,
};

const COUNT: Symbol = symbol_short!("COUNT");

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
enum CounterError {
    Overflow = 1,
}

#[contract]
struct BrokenCounterContract;

#[contractimpl]
impl BrokenCounterContract {
    // Planted bug: increments by 2 instead of 1.
    pub fn increment(env: Env) -> Result<i64, CounterError> {
        let count: i64 = env.storage().instance().get(&COUNT).unwrap_or(0);
        let new_count = count.checked_add(2).ok_or(CounterError::Overflow)?;
        env.storage().instance().set(&COUNT, &new_count);
        Ok(new_count)
    }

    pub fn get(env: Env) -> i64 {
        env.storage().instance().get(&COUNT).unwrap_or(0)
    }
}

struct BrokenCounterAdapter;

#[derive(Debug, Clone, Default)]
struct BrokenCounterModel {
    expected_count: i64,
}
impl ReferenceModel for BrokenCounterModel {}

#[derive(Debug, Clone)]
struct Increment;

impl Command<BrokenCounterAdapter> for Increment {
    fn execute(&self, ctx: &ExecContext, _authorizers: &[Address]) -> Outcome {
        let client = BrokenCounterContractClient::new(ctx.env, ctx.contract_id);
        Outcome::from_try_result(client.try_increment(), true, |e| e as u32)
    }

    fn apply_to_model(
        &self,
        model: &mut BrokenCounterModel,
        _addresses: &AddressPool,
        outcome: &Outcome,
    ) {
        if outcome.is_ok() {
            model.expected_count += 1;
        }
    }
}

impl ContractAdapter for BrokenCounterAdapter {
    type Command = Increment;
    type Model = BrokenCounterModel;

    fn setup(env: &Env, _addresses: &AddressPool) -> (Address, Self::Model) {
        (env.register(BrokenCounterContract, ()), BrokenCounterModel::default())
    }
}

struct CountMatchesModel;

impl Invariant<BrokenCounterAdapter> for CountMatchesModel {
    fn name(&self) -> &'static str {
        "broken-counter-value-matches-model"
    }

    fn check(&self, ctx: &InvariantCtx<BrokenCounterAdapter>) -> Result<(), Violation> {
        let client = BrokenCounterContractClient::new(ctx.env, ctx.contract_id);
        let actual = client.get();
        if actual != ctx.model.expected_count {
            return Err(Violation {
                invariant: self.name(),
                message: format!(
                    "on-chain count {actual} != model's expected count {}",
                    ctx.model.expected_count
                ),
                step_index: ctx.step_index,
            });
        }
        Ok(())
    }
}

#[test]
fn broken_increment_diverges_from_model_and_is_caught() {
    let run = Run {
        steps: vec![Step {
            command: Increment,
            authorized: AuthSelection::none(),
            advance_time: None,
        }],
    };
    let violation = Harness::<BrokenCounterAdapter>::new()
        .with_invariant(CountMatchesModel)
        .run(run)
        .expect_err(
            "a contract that increments by 2 instead of 1 must diverge from the shadow model \
             and be caught as a Violation",
        );
    assert_eq!(violation.invariant, "broken-counter-value-matches-model");
}
