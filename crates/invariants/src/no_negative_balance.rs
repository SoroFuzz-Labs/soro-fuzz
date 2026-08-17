use soro_fuzz_core::{ContractAdapter, Invariant, InvariantCtx, Violation};
use soroban_sdk::Address;

/// contributors: implement this on your `ReferenceModel` to make
/// [`NoNegativeBalance`] (and [`super::SupplyConservation`]) applicable to
/// your contract.
pub trait HasBalances {
    /// Every address the model is currently tracking a balance for.
    fn balance_holders(&self) -> std::vec::Vec<Address>;
    fn balance_of(&self, who: &Address) -> i128;
}

/// No tracked balance may ever go negative.
pub struct NoNegativeBalance;

impl NoNegativeBalance {
    const NAME: &'static str = "no-negative-balance";
}

impl<A> Invariant<A> for NoNegativeBalance
where
    A: ContractAdapter,
    A::Model: HasBalances,
{
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn check(&self, ctx: &InvariantCtx<A>) -> Result<(), Violation> {
        for holder in ctx.model.balance_holders() {
            let balance = ctx.model.balance_of(&holder);
            if balance < 0 {
                return Err(Violation {
                    invariant: Self::NAME,
                    message: format!("balance of {holder:?} went negative: {balance}"),
                    step_index: ctx.step_index,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{FakeAdapter, FakeCommand};
    use soro_fuzz_core::{AddressPool, InvariantCtx, Outcome};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[derive(Debug, Clone, Default)]
    struct FakeModel {
        balances: std::vec::Vec<(Address, i128)>,
    }

    impl soro_fuzz_core::ReferenceModel for FakeModel {}

    impl HasBalances for FakeModel {
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

    fn ctx_for<'a>(
        env: &'a Env,
        contract_id: &'a Address,
        model: &'a FakeModel,
        command: &'a FakeCommand,
        outcome: &'a Outcome,
        addresses: &'a AddressPool,
    ) -> InvariantCtx<'a, FakeAdapter<FakeCommand, FakeModel>> {
        InvariantCtx {
            env,
            contract_id,
            model,
            command,
            last_outcome: outcome,
            authorizers: &[],
            addresses,
            step_index: 0,
        }
    }

    #[test]
    fn flags_negative_balance() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let addr = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel {
            balances: std::vec![(addr, -1)],
        };
        let ctx = ctx_for(
            &env,
            &contract_id,
            &model,
            &FakeCommand,
            &Outcome::Ok,
            &pool,
        );
        assert!(NoNegativeBalance.check(&ctx).is_err());
    }

    #[test]
    fn allows_zero_and_positive_balances() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel {
            balances: std::vec![(a, 0), (b, 100)],
        };
        let ctx = ctx_for(
            &env,
            &contract_id,
            &model,
            &FakeCommand,
            &Outcome::Ok,
            &pool,
        );
        assert!(NoNegativeBalance.check(&ctx).is_ok());
    }
}
