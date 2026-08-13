//! How the harness registers a contract and gets its starting shadow state.

use crate::auth::AddressPool;
use crate::command::Command;
use crate::model::ReferenceModel;
use soroban_sdk::{Address, Env};

/// The top-level extension point: one impl per contract under test.
///
/// contributors: implement this for a small marker type representing your
/// contract (it doesn't need any fields). `setup` is where you register the
/// contract via `env.register(..)`, assign roles out of the address pool
/// (e.g. treat `addresses.get(0)` as the admin), and build the initial
/// shadow model to match whatever state the constructor left the contract
/// in.
pub trait ContractAdapter: Sized {
    type Command: Command<Self>;
    type Model: ReferenceModel;

    /// Deploy/register the contract into `env` and return its id plus the
    /// shadow model's initial state.
    fn setup(env: &Env, addresses: &AddressPool) -> (Address, Self::Model);
}
