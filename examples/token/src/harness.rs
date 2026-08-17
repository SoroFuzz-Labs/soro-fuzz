//! Wiring the token contract into `soro-fuzz-core`. This is the "prove
//! generality" example: unlike the counter, it wires the *shared*
//! `soro-fuzz-invariants` (no negative balance, supply conservation, auth
//! required for mutation) instead of writing its own from scratch —
//! `soro-fuzz-core`/`soro-fuzz-invariants` needed no changes to support it.

use arbitrary::Arbitrary;
use soroban_sdk::{
    testutils::{MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use soro_fuzz_core::{AddressPool, Command, ContractAdapter, ExecContext, Outcome, ReferenceModel};
use soro_fuzz_invariants::{HasBalances, HasTotalSupply, RequiresAuthorizer};
use soro_fuzz_strategies::{AddressIndex, NonNegativeI128};

use crate::{TokenContract, TokenContractArgs, TokenContractClient};

/// Address pool role convention for this contract (see `setup`): index 0 is
/// always the admin, index 1 always starts holding the initial supply.
const ADMIN_INDEX: usize = 0;
const INITIAL_HOLDER_INDEX: usize = 1;
const INITIAL_SUPPLY: i128 = 1_000_000_000;

pub struct TokenAdapter;

#[derive(Debug, Clone)]
pub struct TokenModel {
    pub admin: Address,
    balances: std::vec::Vec<(Address, i128)>,
    total_supply: i128,
}

impl TokenModel {
    fn balance_mut(&mut self, who: &Address) -> &mut i128 {
        if let Some(pos) = self.balances.iter().position(|(a, _)| a == who) {
            &mut self.balances[pos].1
        } else {
            self.balances.push((who.clone(), 0));
            &mut self.balances.last_mut().unwrap().1
        }
    }
}

impl ReferenceModel for TokenModel {}

impl HasBalances for TokenModel {
    fn balance_holders(&self) -> std::vec::Vec<Address> {
        self.balances.iter().map(|(a, _)| a.clone()).collect()
    }
    fn balance_of(&self, who: &Address) -> i128 {
        self.balances
            .iter()
            .find(|(a, _)| a == who)
            .map(|(_, b)| *b)
            .unwrap_or(0)
    }
}

impl HasTotalSupply for TokenModel {
    fn total_supply(&self) -> i128 {
        self.total_supply
    }
}

/// The token's action space. Amounts use `NonNegativeI128` (the contract
/// itself also rejects negative amounts with a declared error — this
/// strategy choice just means the fuzzer spends its budget on the
/// interesting overflow/insufficient-balance edges instead of always
/// hitting the trivial negative-amount rejection); addresses are picked out
/// of the pool via `AddressIndex`.
#[derive(Arbitrary, Debug, Clone)]
pub enum TokenCommand {
    Mint {
        to: AddressIndex,
        amount: NonNegativeI128,
    },
    Burn {
        from: AddressIndex,
        amount: NonNegativeI128,
    },
    Transfer {
        from: AddressIndex,
        to: AddressIndex,
        amount: NonNegativeI128,
    },
}

impl ContractAdapter for TokenAdapter {
    type Command = TokenCommand;
    type Model = TokenModel;

    fn setup(env: &Env, addresses: &AddressPool) -> (Address, Self::Model) {
        let admin = addresses.get(ADMIN_INDEX).clone();
        let initial_holder = addresses.get(INITIAL_HOLDER_INDEX).clone();
        let contract_id = env.register(
            TokenContract,
            TokenContractArgs::__constructor(&admin, &initial_holder, &INITIAL_SUPPLY),
        );
        let model = TokenModel {
            admin,
            balances: std::vec![(initial_holder, INITIAL_SUPPLY)],
            total_supply: INITIAL_SUPPLY,
        };
        (contract_id, model)
    }
}

impl Command<TokenAdapter> for TokenCommand {
    fn execute(&self, ctx: &ExecContext, authorizers: &[Address]) -> Outcome {
        let client = TokenContractClient::new(ctx.env, ctx.contract_id);

        match self {
            TokenCommand::Mint { to, amount } => {
                let admin = ctx.addresses.get(ADMIN_INDEX).clone();
                let to_addr = ctx.addresses.get(to.0 as usize).clone();
                let amount = amount.get();
                mock_single_auth(
                    ctx.env,
                    ctx.contract_id,
                    &admin,
                    authorizers,
                    "mint",
                    soroban_sdk::vec![ctx.env, to_addr.into_val(ctx.env), amount.into_val(ctx.env)],
                );
                match client.try_mint(&to_addr, &amount) {
                    Ok(Ok(_)) => Outcome::Ok,
                    Err(Ok(e)) => Outcome::DeclaredError(e as u32),
                    Err(Err(invoke_err)) => Outcome::Rejected(format!("{invoke_err:?}")),
                    Ok(Err(conv_err)) => {
                        panic!("mint: unexpected value conversion error: {conv_err:?}")
                    }
                }
            }
            TokenCommand::Burn { from, amount } => {
                let from_addr = ctx.addresses.get(from.0 as usize).clone();
                let amount = amount.get();
                mock_single_auth(
                    ctx.env,
                    ctx.contract_id,
                    &from_addr,
                    authorizers,
                    "burn",
                    soroban_sdk::vec![
                        ctx.env,
                        from_addr.into_val(ctx.env),
                        amount.into_val(ctx.env)
                    ],
                );
                match client.try_burn(&from_addr, &amount) {
                    Ok(Ok(_)) => Outcome::Ok,
                    Err(Ok(e)) => Outcome::DeclaredError(e as u32),
                    Err(Err(invoke_err)) => Outcome::Rejected(format!("{invoke_err:?}")),
                    Ok(Err(conv_err)) => {
                        panic!("burn: unexpected value conversion error: {conv_err:?}")
                    }
                }
            }
            TokenCommand::Transfer { from, to, amount } => {
                let from_addr = ctx.addresses.get(from.0 as usize).clone();
                let to_addr = ctx.addresses.get(to.0 as usize).clone();
                let amount = amount.get();
                mock_single_auth(
                    ctx.env,
                    ctx.contract_id,
                    &from_addr,
                    authorizers,
                    "transfer",
                    soroban_sdk::vec![
                        ctx.env,
                        from_addr.into_val(ctx.env),
                        to_addr.into_val(ctx.env),
                        amount.into_val(ctx.env)
                    ],
                );
                match client.try_transfer(&from_addr, &to_addr, &amount) {
                    Ok(Ok(_)) => Outcome::Ok,
                    Err(Ok(e)) => Outcome::DeclaredError(e as u32),
                    Err(Err(invoke_err)) => Outcome::Rejected(format!("{invoke_err:?}")),
                    Ok(Err(conv_err)) => {
                        panic!("transfer: unexpected value conversion error: {conv_err:?}")
                    }
                }
            }
        }
    }

    fn apply_to_model(&self, model: &mut TokenModel, addresses: &AddressPool, outcome: &Outcome) {
        if !outcome.is_ok() {
            return;
        }
        match self {
            TokenCommand::Mint { to, amount } => {
                let to_addr = addresses.get(to.0 as usize).clone();
                let amount = amount.get();
                *model.balance_mut(&to_addr) += amount;
                model.total_supply += amount;
            }
            TokenCommand::Burn { from, amount } => {
                let from_addr = addresses.get(from.0 as usize).clone();
                let amount = amount.get();
                *model.balance_mut(&from_addr) -= amount;
                model.total_supply -= amount;
            }
            TokenCommand::Transfer { from, to, amount } => {
                let from_addr = addresses.get(from.0 as usize).clone();
                let to_addr = addresses.get(to.0 as usize).clone();
                let amount = amount.get();
                *model.balance_mut(&from_addr) -= amount;
                *model.balance_mut(&to_addr) += amount;
            }
        }
    }
}

impl RequiresAuthorizer<TokenAdapter> for TokenCommand {
    fn required_authorizer(&self, model: &TokenModel, addresses: &AddressPool) -> Option<Address> {
        match self {
            TokenCommand::Mint { .. } => Some(model.admin.clone()),
            TokenCommand::Burn { from, .. } | TokenCommand::Transfer { from, .. } => {
                Some(addresses.get(from.0 as usize).clone())
            }
        }
    }
}

/// Mocks `signer`'s authorization for `contract_id.fn_name(args)` iff
/// `signer` is one of the addresses this step authorized; otherwise mocks
/// nothing, so the contract's `require_auth()` legitimately fails.
fn mock_single_auth(
    env: &Env,
    contract_id: &Address,
    signer: &Address,
    authorizers: &[Address],
    fn_name: &str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) {
    if authorizers.contains(signer) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use soro_fuzz_core::{AuthSelection, Harness, Run, Step};
    use soro_fuzz_invariants::{AuthRequiredForMutation, NoNegativeBalance, SupplyConservation};

    fn harness() -> Harness<TokenAdapter> {
        Harness::<TokenAdapter>::new()
            .with_invariant(NoNegativeBalance)
            .with_invariant(SupplyConservation)
            .with_invariant(AuthRequiredForMutation)
    }

    fn step(command: TokenCommand, authorized_indices: Vec<u8>) -> Step<TokenCommand> {
        Step {
            command,
            authorized: AuthSelection::from_indices(authorized_indices),
            advance_time: None,
        }
    }

    #[test]
    fn transfer_with_auth_succeeds() {
        let run = Run {
            steps: vec![step(
                TokenCommand::Transfer {
                    from: AddressIndex(INITIAL_HOLDER_INDEX as u8),
                    to: AddressIndex(2),
                    amount: NonNegativeI128::new(100),
                },
                vec![INITIAL_HOLDER_INDEX as u8],
            )],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn unauthorized_transfer_is_rejected_not_a_panic() {
        let run = Run {
            steps: vec![step(
                TokenCommand::Transfer {
                    from: AddressIndex(INITIAL_HOLDER_INDEX as u8),
                    to: AddressIndex(2),
                    amount: NonNegativeI128::new(100),
                },
                vec![], // nobody authorized
            )],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn overdrawing_transfer_is_a_declared_error_not_a_panic() {
        let run = Run {
            steps: vec![step(
                TokenCommand::Transfer {
                    from: AddressIndex(INITIAL_HOLDER_INDEX as u8),
                    to: AddressIndex(2),
                    amount: NonNegativeI128::new(INITIAL_SUPPLY + 1),
                },
                vec![INITIAL_HOLDER_INDEX as u8],
            )],
        };
        assert!(harness().run(run).is_ok());
    }

    #[test]
    fn admin_mint_then_holder_burn_round_trips() {
        let run = Run {
            steps: vec![
                step(
                    TokenCommand::Mint {
                        to: AddressIndex(2),
                        amount: NonNegativeI128::new(500),
                    },
                    vec![ADMIN_INDEX as u8],
                ),
                step(
                    TokenCommand::Burn {
                        from: AddressIndex(2),
                        amount: NonNegativeI128::new(200),
                    },
                    vec![2],
                ),
            ],
        };
        assert!(harness().run(run).is_ok());
    }
}
