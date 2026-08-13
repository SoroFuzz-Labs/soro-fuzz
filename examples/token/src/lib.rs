//! A minimal fungible token, written for this repo rather than reusing an
//! existing published token contract (keeps licensing simple, and keeps
//! the example small enough to read in one sitting). It's deliberately not
//! a full SEP-41 implementation — no allowances, no TTL/bump plumbing —
//! just enough surface (`mint`, `burn`, `transfer`, `balance`) to give the
//! `no-negative-balance`, `supply-conservation`, and
//! `auth-required-for-mutation` invariants from `soro-fuzz-invariants`
//! something real to check.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

#[cfg(any(test, feature = "testutils"))]
pub mod harness;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    InsufficientBalance = 1,
    Overflow = 2,
    NegativeAmount = 3,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    TotalSupply,
    Balance(Address),
}

#[contract]
pub struct TokenContract;

#[contractimpl]
impl TokenContract {
    pub fn __constructor(env: Env, admin: Address, initial_holder: Address, initial_supply: i128) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TotalSupply, &initial_supply);
        env.storage()
            .persistent()
            .set(&DataKey::Balance(initial_holder), &initial_supply);
    }

    pub fn mint(env: Env, to: Address, amount: i128) -> Result<i128, Error> {
        if amount < 0 {
            return Err(Error::NegativeAmount);
        }
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let new_balance = Self::balance(env.clone(), to.clone())
            .checked_add(amount)
            .ok_or(Error::Overflow)?;
        let supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        let new_supply = supply.checked_add(amount).ok_or(Error::Overflow)?;

        env.storage().persistent().set(&DataKey::Balance(to), &new_balance);
        env.storage().instance().set(&DataKey::TotalSupply, &new_supply);
        Ok(new_balance)
    }

    pub fn burn(env: Env, from: Address, amount: i128) -> Result<i128, Error> {
        if amount < 0 {
            return Err(Error::NegativeAmount);
        }
        from.require_auth();

        // `checked_sub` only guards against `i128` overflow, not against the
        // result being merely negative (`100i128.checked_sub(200)` is a
        // perfectly valid `Some(-100)`) — insufficient balance has to be
        // its own explicit check, not inferred from `checked_sub` failing.
        let balance = Self::balance(env.clone(), from.clone());
        if amount > balance {
            return Err(Error::InsufficientBalance);
        }
        let new_balance = balance - amount;
        let supply: i128 = env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0);
        let new_supply = supply.checked_sub(amount).ok_or(Error::Overflow)?;

        env.storage().persistent().set(&DataKey::Balance(from), &new_balance);
        env.storage().instance().set(&DataKey::TotalSupply, &new_supply);
        Ok(new_balance)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        if amount < 0 {
            return Err(Error::NegativeAmount);
        }
        from.require_auth();

        // See the same comment in `burn`: `checked_sub` alone doesn't imply
        // "sufficient balance", since going negative is a valid `i128`.
        let from_balance = Self::balance(env.clone(), from.clone());
        if amount > from_balance {
            return Err(Error::InsufficientBalance);
        }
        let new_from_balance = from_balance - amount;
        let new_to_balance = Self::balance(env.clone(), to.clone())
            .checked_add(amount)
            .ok_or(Error::Overflow)?;

        env.storage().persistent().set(&DataKey::Balance(from), &new_from_balance);
        env.storage().persistent().set(&DataKey::Balance(to), &new_to_balance);
        Ok(())
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalSupply).unwrap_or(0)
    }
}
