use crate::no_negative_balance::HasBalances;
use soro_fuzz_core::{ContractAdapter, Invariant, InvariantCtx, Violation};

/// contributors: implement alongside [`HasBalances`](super::HasBalances) to
/// make [`SupplyConservation`] applicable to your contract.
pub trait HasTotalSupply {
    fn total_supply(&self) -> i128;
}

/// The sum of every tracked balance must always equal the tracked total
/// supply — no value created or destroyed outside of declared mint/burn
/// paths the model itself accounts for.
pub struct SupplyConservation;

impl SupplyConservation {
    const NAME: &'static str = "supply-conservation";
}

impl<A> Invariant<A> for SupplyConservation
where
    A: ContractAdapter,
    A::Model: HasBalances + HasTotalSupply,
{
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn check(&self, ctx: &InvariantCtx<A>) -> Result<(), Violation> {
        let sum: i128 = ctx
            .model
            .balance_holders()
            .iter()
            .map(|holder| ctx.model.balance_of(holder))
            .sum();
        let total = ctx.model.total_supply();
        if sum != total {
            return Err(Violation {
                invariant: Self::NAME,
                message: format!("sum of balances ({sum}) != total supply ({total})"),
                step_index: ctx.step_index,
            });
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
    use soroban_sdk::{Address, Env};

    #[derive(Debug, Clone, Default)]
    struct FakeModel {
        balances: std::vec::Vec<(Address, i128)>,
        total_supply: i128,
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

    impl HasTotalSupply for FakeModel {
        fn total_supply(&self) -> i128 {
            self.total_supply
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
    fn flags_mismatched_supply() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let a = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel {
            balances: std::vec![(a, 100)],
            total_supply: 50,
        };
        let ctx = ctx_for(&env, &contract_id, &model, &FakeCommand, &Outcome::Ok, &pool);
        assert!(SupplyConservation.check(&ctx).is_err());
    }

    #[test]
    fn allows_matching_supply() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel {
            balances: std::vec![(a, 40), (b, 60)],
            total_supply: 100,
        };
        let ctx = ctx_for(&env, &contract_id, &model, &FakeCommand, &Outcome::Ok, &pool);
        assert!(SupplyConservation.check(&ctx).is_ok());
    }
}
