//! Wiring the escrow contract into `soro-fuzz-core`. Note `AdvanceTime` is
//! modeled as its own `Command` variant (composing
//! `soro-fuzz-strategies::TimeAdvance`) rather than relying only on
//! `Step::advance_time` — the deadline logic needs edge-biased jumps (0, 1,
//! a multi-year jump) interleaved with the other commands in the same
//! generated sequence, which is exactly what `TimeAdvance` is for.

use arbitrary::Arbitrary;
use soroban_sdk::{
    testutils::{Ledger as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use soro_fuzz_core::{
    AddressPool, Command, ContractAdapter, ExecContext, Invariant, InvariantCtx, Outcome,
    ReferenceModel, Violation,
};
use soro_fuzz_strategies::{NonNegativeI128, TimeAdvance};

use crate::{EscrowContract, EscrowContractArgs, EscrowContractClient, Status};

const DEPOSITOR_INDEX: usize = 0;
const BENEFICIARY_INDEX: usize = 1;
const DEADLINE_OFFSET_SECS: u64 = 1000;

pub struct EscrowAdapter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Empty,
    Deposited,
    Released,
    Refunded,
}

#[derive(Debug, Clone)]
pub struct EscrowModel {
    pub depositor: Address,
    pub status: ModelStatus,
}

impl ReferenceModel for EscrowModel {}

/// The escrow's action space.
#[derive(Arbitrary, Debug, Clone)]
pub enum EscrowCommand {
    Deposit(NonNegativeI128),
    /// Depositor-authorized; only legal before the deadline.
    Release,
    /// Permissionless; only legal at/after the deadline.
    Refund,
    /// Advances the ledger clock — see the module doc for why this is a
    /// `Command` variant rather than relying on `Step::advance_time`.
    AdvanceTime(TimeAdvance),
}

impl ContractAdapter for EscrowAdapter {
    type Command = EscrowCommand;
    type Model = EscrowModel;

    fn setup(env: &Env, addresses: &AddressPool) -> (Address, Self::Model) {
        let depositor = addresses.get(DEPOSITOR_INDEX).clone();
        let beneficiary = addresses.get(BENEFICIARY_INDEX).clone();
        let deadline = env
            .ledger()
            .timestamp()
            .saturating_add(DEADLINE_OFFSET_SECS);
        let contract_id = env.register(
            EscrowContract,
            EscrowContractArgs::__constructor(&depositor, &beneficiary, &deadline),
        );
        let model = EscrowModel {
            depositor,
            status: ModelStatus::Empty,
        };
        (contract_id, model)
    }
}

impl Command<EscrowAdapter> for EscrowCommand {
    fn execute(&self, ctx: &ExecContext, authorizers: &[Address]) -> Outcome {
        let client = EscrowContractClient::new(ctx.env, ctx.contract_id);
        let depositor = ctx.addresses.get(DEPOSITOR_INDEX).clone();

        match self {
            EscrowCommand::Deposit(amount) => {
                let amount = amount.get();
                let auth_satisfied = mock_single_auth(
                    ctx.env,
                    ctx.contract_id,
                    &depositor,
                    authorizers,
                    "deposit",
                    soroban_sdk::vec![ctx.env, amount.into_val(ctx.env)],
                );
                Outcome::from_try_result(client.try_deposit(&amount), auth_satisfied, |e| e as u32)
            }
            EscrowCommand::Release => {
                let auth_satisfied = mock_single_auth(
                    ctx.env,
                    ctx.contract_id,
                    &depositor,
                    authorizers,
                    "release",
                    soroban_sdk::vec![ctx.env],
                );
                Outcome::from_try_result(client.try_release(), auth_satisfied, |e| e as u32)
            }
            EscrowCommand::Refund => {
                // Permissionless by design (see the contract doc comment):
                // no auth to mock, and none required, so `Abort` here can
                // only mean an undeclared bug.
                Outcome::from_try_result(client.try_refund(), true, |e| e as u32)
            }
            EscrowCommand::AdvanceTime(advance) => {
                let now = ctx.env.ledger().timestamp();
                ctx.env
                    .ledger()
                    .set_timestamp(now.saturating_add(advance.0 as u64));
                Outcome::Ok
            }
        }
    }

    fn apply_to_model(&self, model: &mut EscrowModel, _addresses: &AddressPool, outcome: &Outcome) {
        if !outcome.is_ok() {
            return;
        }
        match self {
            EscrowCommand::Deposit(_) => model.status = ModelStatus::Deposited,
            EscrowCommand::Release => model.status = ModelStatus::Released,
            EscrowCommand::Refund => model.status = ModelStatus::Refunded,
            EscrowCommand::AdvanceTime(_) => {}
        }
    }
}

/// Contract-specific invariant, same pattern as the counter's
/// `CounterValueMatchesModel`: the on-chain status must always match what
/// the shadow model expects.
pub struct EscrowStatusMatchesModel;

impl Invariant<EscrowAdapter> for EscrowStatusMatchesModel {
    fn name(&self) -> &'static str {
        "escrow-status-matches-model"
    }

    fn check(&self, ctx: &InvariantCtx<EscrowAdapter>) -> Result<(), Violation> {
        let client = EscrowContractClient::new(ctx.env, ctx.contract_id);
        let actual = client.status();
        let expected = match ctx.model.status {
            ModelStatus::Empty => Status::Empty,
            ModelStatus::Deposited => Status::Deposited,
            ModelStatus::Released => Status::Released,
            ModelStatus::Refunded => Status::Refunded,
        };
        if actual != expected {
            return Err(Violation {
                invariant: self.name(),
                message: format!("on-chain status {actual:?} != model status {expected:?}"),
                step_index: ctx.step_index,
            });
        }
        Ok(())
    }
}

/// Mocks `signer`'s authorization for `contract_id.fn_name(args)` iff
/// `signer` is one of the addresses this step authorized; otherwise mocks
/// nothing, so the contract's `require_auth()` legitimately fails. Returns
/// whether auth was actually satisfied, so the caller can pass it to
/// `Outcome::from_try_result` and correctly distinguish an expected auth
/// rejection from an undeclared panic.
fn mock_single_auth(
    env: &Env,
    contract_id: &Address,
    signer: &Address,
    authorizers: &[Address],
    fn_name: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) -> bool {
    let satisfied = authorizers.contains(signer);
    if satisfied {
        let invoke = MockAuthInvoke {
            contract: contract_id,
            fn_name,
            args,
            sub_invokes: &[],
        };
        env.mock_auths(&[MockAuth {
            address: signer,
            invoke: &invoke,
        }]);
    } else {
        env.mock_auths(&[]);
    }
    satisfied
}

#[cfg(test)]
mod tests {
    use super::*;
    use soro_fuzz_core::{AuthSelection, Harness, Run, Step};

    fn harness() -> Harness<EscrowAdapter> {
        Harness::<EscrowAdapter>::new().with_invariant(EscrowStatusMatchesModel)
    }

    fn step(command: EscrowCommand, authorized_indices: Vec<u8>) -> Step<EscrowCommand> {
        Step {
            command,
            authorized: AuthSelection::from_indices(authorized_indices),
            advance_time: None,
        }
    }

    #[test]
    fn deposit_then_release_before_deadline_succeeds() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(EscrowCommand::Release, vec![DEPOSITOR_INDEX as u8]),
            ],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn refund_before_deadline_is_declared_error_not_a_panic() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(EscrowCommand::Refund, vec![]),
            ],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn refund_after_deadline_succeeds_without_auth() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(
                    EscrowCommand::AdvanceTime(TimeAdvance(DEADLINE_OFFSET_SECS as u32 + 1)),
                    vec![],
                ),
                step(EscrowCommand::Refund, vec![]),
            ],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn release_after_deadline_is_declared_error_not_a_panic() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(
                    EscrowCommand::AdvanceTime(TimeAdvance(DEADLINE_OFFSET_SECS as u32 + 1)),
                    vec![],
                ),
                step(EscrowCommand::Release, vec![DEPOSITOR_INDEX as u8]),
            ],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn double_deposit_is_declared_error_not_a_panic() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(50)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
            ],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn unauthorized_release_is_rejected_not_a_panic() {
        let run = Run {
            steps: vec![
                step(
                    EscrowCommand::Deposit(NonNegativeI128::new(100)),
                    vec![DEPOSITOR_INDEX as u8],
                ),
                step(EscrowCommand::Release, vec![]),
            ],
        };
        assert!(harness().run(run).is_ok());
    }
}
