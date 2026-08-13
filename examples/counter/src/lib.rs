//! A minimal counter contract: the simplest possible vehicle for wiring up
//! `soro-fuzz` end to end before anything more interesting (tokens, escrow)
//! gets involved.
//!
//! `increment`/`decrement` are open to anyone and can overflow/underflow
//! (a declared error, not a panic); `reset`/`set` are admin-gated to give
//! the harness something to exercise auth against.

#![allow(clippy::inconsistent_digit_grouping)]

use soroban_sdk::{contract, contracterror, contractimpl, symbol_short, Address, Env, Symbol};

#[cfg(any(test, feature = "testutils"))]
pub mod harness;

const COUNT: Symbol = symbol_short!("COUNT");
const ADMIN: Symbol = symbol_short!("ADMIN");

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Overflow = 1,
    Underflow = 2,
}

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&COUNT, &0i64);
    }

    pub fn increment(env: Env) -> Result<i64, Error> {
        let count: i64 = env.storage().instance().get(&COUNT).unwrap_or(0);
        let new_count = count.checked_add(1).ok_or(Error::Overflow)?;
        env.storage().instance().set(&COUNT, &new_count);
        Ok(new_count)
    }

    pub fn decrement(env: Env) -> Result<i64, Error> {
        let count: i64 = env.storage().instance().get(&COUNT).unwrap_or(0);
        let new_count = count.checked_sub(1).ok_or(Error::Underflow)?;
        env.storage().instance().set(&COUNT, &new_count);
        Ok(new_count)
    }

    pub fn reset(env: Env) -> i64 {
        let admin: Address = env.storage().instance().get(&ADMIN).unwrap();
        admin.require_auth();
        env.storage().instance().set(&COUNT, &0i64);
        0
    }

    pub fn set(env: Env, value: i64) -> i64 {
        let admin: Address = env.storage().instance().get(&ADMIN).unwrap();
        admin.require_auth();
        env.storage().instance().set(&COUNT, &value);
        value
    }

    pub fn get(env: Env) -> i64 {
        env.storage().instance().get(&COUNT).unwrap_or(0)
    }
}
