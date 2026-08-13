use soro_fuzz_core::{AddressPool, ContractAdapter, Invariant, InvariantCtx, Violation};
use soroban_sdk::Address;

/// contributors: implement this on your `Command` enum to make
/// [`AuthRequiredForMutation`] applicable to your contract. Return the
/// address that must have authorized this command for it to legitimately
/// succeed, or `None` for commands that are intentionally open to anyone
/// (e.g. a token's `balance` read, or `increment` on the counter example).
/// `addresses` is provided (mirroring `Command::apply_to_model`) so
/// commands built on pool indices (e.g.
/// `soro_fuzz_strategies::AddressIndex`) can resolve the specific address
/// this invocation targeted, not just addresses the model already knows
/// about.
pub trait RequiresAuthorizer<A: ContractAdapter> {
    fn required_authorizer(&self, model: &A::Model, addresses: &AddressPool) -> Option<Address>;
}

/// If a command that declares a required authorizer nonetheless succeeded,
/// that authorizer must have been in the step's authorized set. This
/// catches auth-check bugs (a mutation that should be gated but isn't) —
/// it's a backstop *in addition to* the contract's own `require_auth`
/// calls, not a replacement for them, so it only fires when a bug lets a
/// call through that shouldn't have.
pub struct AuthRequiredForMutation;

impl AuthRequiredForMutation {
    const NAME: &'static str = "auth-required-for-mutation";
}

impl<A> Invariant<A> for AuthRequiredForMutation
where
    A: ContractAdapter,
    A::Command: RequiresAuthorizer<A>,
{
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn check(&self, ctx: &InvariantCtx<A>) -> Result<(), Violation> {
        if !ctx.last_outcome.is_ok() {
            return Ok(());
        }
        if let Some(required) = ctx.command.required_authorizer(ctx.model, ctx.addresses) {
            if !ctx.authorizers.contains(&required) {
                return Err(Violation {
                    invariant: Self::NAME,
                    message: format!(
                        "command succeeded without authorization from required address {required:?}"
                    ),
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
    use crate::test_support::FakeAdapter;
    use soro_fuzz_core::{Command, ExecContext, InvariantCtx, Outcome, ReferenceModel};
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[derive(Debug, Clone, Default)]
    struct FakeModel;
    impl ReferenceModel for FakeModel {}

    #[derive(Debug, Clone)]
    struct RequireCmd(Option<Address>);

    type Adapter = FakeAdapter<RequireCmd, FakeModel>;

    impl Command<Adapter> for RequireCmd {
        fn execute(&self, _ctx: &ExecContext, _authorizers: &[Address]) -> Outcome {
            Outcome::Ok
        }
        fn apply_to_model(
            &self,
            _model: &mut FakeModel,
            _addresses: &soro_fuzz_core::AddressPool,
            _outcome: &Outcome,
        ) {
        }
    }

    impl RequiresAuthorizer<Adapter> for RequireCmd {
        fn required_authorizer(&self, _model: &FakeModel, _addresses: &AddressPool) -> Option<Address> {
            self.0.clone()
        }
    }

    fn ctx_for<'a>(
        env: &'a Env,
        contract_id: &'a Address,
        model: &'a FakeModel,
        command: &'a RequireCmd,
        authorizers: &'a [Address],
        addresses: &'a AddressPool,
    ) -> InvariantCtx<'a, Adapter> {
        InvariantCtx {
            env,
            contract_id,
            model,
            command,
            last_outcome: &Outcome::Ok,
            authorizers,
            addresses,
            step_index: 0,
        }
    }

    #[test]
    fn flags_success_without_required_authorizer() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let admin = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel;
        let command = RequireCmd(Some(admin));
        let ctx = ctx_for(&env, &contract_id, &model, &command, &[], &pool);
        assert!(AuthRequiredForMutation.check(&ctx).is_err());
    }

    #[test]
    fn allows_success_with_required_authorizer_present() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let admin = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel;
        let command = RequireCmd(Some(admin.clone()));
        let authorizers = [admin];
        let ctx = ctx_for(&env, &contract_id, &model, &command, &authorizers, &pool);
        assert!(AuthRequiredForMutation.check(&ctx).is_ok());
    }

    #[test]
    fn allows_commands_that_dont_require_auth() {
        let env = Env::default();
        let contract_id = Address::generate(&env);
        let pool = AddressPool::generate(&env, 1);
        let model = FakeModel;
        let command = RequireCmd(None);
        let ctx = ctx_for(&env, &contract_id, &model, &command, &[], &pool);
        assert!(AuthRequiredForMutation.check(&ctx).is_ok());
    }
}
