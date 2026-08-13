//! The `Harness` builder and run loop: the single place that turns a
//! generated [`Run`] into a sequence of executions, shadow-model updates,
//! and invariant checks. Both the `cargo-fuzz` targets and the `proptest`
//! mirrors in the example crates call the same [`Harness::run`].

use std::panic::{catch_unwind, AssertUnwindSafe};

use soroban_sdk::{testutils::Ledger as _, Env};

use crate::adapter::ContractAdapter;
use crate::auth::AddressPool;
use crate::command::{Command, ExecContext, Outcome, Run};
use crate::invariant::{Invariant, InvariantCtx, Violation};

const DEFAULT_COMMANDS_PER_RUN: usize = 32;
const DEFAULT_ADDRESS_POOL_SIZE: usize = 4;

/// Builds and runs a fuzzing/property-testing session for one
/// [`ContractAdapter`].
pub struct Harness<A: ContractAdapter> {
    commands_per_run: usize,
    address_pool_size: usize,
    invariants: Vec<Box<dyn Invariant<A>>>,
}

impl<A: ContractAdapter> Default for Harness<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: ContractAdapter> Harness<A> {
    pub fn new() -> Self {
        Self {
            commands_per_run: DEFAULT_COMMANDS_PER_RUN,
            address_pool_size: DEFAULT_ADDRESS_POOL_SIZE,
            invariants: Vec::new(),
        }
    }

    /// Upper bound on how many steps of a generated [`Run`] are actually
    /// executed; extra generated steps are truncated. Bounding happens here
    /// rather than in `Run`'s `Arbitrary` impl because the bound is a
    /// runtime `Harness` setting, not something `arbitrary()` has access to.
    pub fn commands_per_run(mut self, n: usize) -> Self {
        self.commands_per_run = n.max(1);
        self
    }

    /// How many addresses to generate into the shared pool each run.
    pub fn address_pool_size(mut self, n: usize) -> Self {
        self.address_pool_size = n.max(1);
        self
    }

    // contributors: add a strategy-registry style hook here if you need to
    // plug in custom per-type generation policy beyond what `arbitrary`
    // derives give you for free; for now composing `soro-fuzz-strategies`
    // types directly into your `Command` enum covers the common cases.

    /// Registers an invariant to be checked after every step.
    pub fn with_invariant(mut self, invariant: impl Invariant<A> + 'static) -> Self {
        self.invariants.push(Box::new(invariant));
        self
    }

    /// Runs one generated sequence to completion or to the first violation.
    ///
    /// Deploys a fresh contract in a fresh `Env` (host-environment
    /// execution, matching how `soroban-sdk` unit tests run contracts — not
    /// guest/wasm-level execution), then for each step: optionally advances
    /// ledger time, resolves the arbitrary auth subset against the address
    /// pool, executes the command (catching any panic as an
    /// `UndeclaredPanic` finding), updates the shadow model, and checks
    /// every registered invariant.
    pub fn run(&self, mut run: Run<A::Command>) -> Result<(), Violation> {
        run.steps.truncate(self.commands_per_run);

        let env = Env::default();
        let addresses = AddressPool::generate(&env, self.address_pool_size);
        let (contract_id, mut model) = A::setup(&env, &addresses);

        for (step_index, step) in run.steps.into_iter().enumerate() {
            if let Some(secs) = step.advance_time {
                let now = env.ledger().timestamp();
                env.ledger().set_timestamp(now.saturating_add(secs as u64));
            }

            let authorizers = addresses.resolve(&step.authorized);
            let ctx = ExecContext {
                env: &env,
                contract_id: &contract_id,
                addresses: &addresses,
            };

            log::debug!(
                "step {step_index}: {:?} authorized_by={:?}",
                step.command,
                authorizers
            );

            let command = step.command;
            let outcome = match catch_unwind(AssertUnwindSafe(|| command.execute(&ctx, &authorizers))) {
                Ok(outcome) => outcome,
                Err(payload) => Outcome::UndeclaredPanic(panic_message(payload)),
            };

            if let Outcome::UndeclaredPanic(message) = &outcome {
                return Err(Violation {
                    invariant: "no-undeclared-panic",
                    message: message.clone(),
                    step_index,
                });
            }

            command.apply_to_model(&mut model, &addresses, &outcome);

            let invariant_ctx = InvariantCtx {
                env: &env,
                contract_id: &contract_id,
                model: &model,
                command: &command,
                last_outcome: &outcome,
                authorizers: &authorizers,
                addresses: &addresses,
                step_index,
            };
            for invariant in &self.invariants {
                invariant.check(&invariant_ctx)?;
            }
        }

        Ok(())
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}
