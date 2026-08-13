//! A minimal deadline-gated escrow: `deposit` once, then either `release`
//! (depositor-authorized, only before the deadline) or `refund`
//! (permissionless, only at/after the deadline). Funds are tracked as a
//! plain internal ledger amount rather than a real token transfer, to keep
//! the example focused on the state-machine + deadline shape rather than
//! cross-contract token integration.
//!
//! This is the third example specifically to prove the harness isn't
//! secretly token-shaped: its state machine (single deposit, then exactly
//! one of two terminal transitions, gated by ledger time rather than
//! balances) has nothing in common with the counter or token examples.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[cfg(any(test, feature = "testutils"))]
pub mod harness;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyDeposited = 1,
    NotDeposited = 2,
    NotYetExpired = 3,
    AlreadyExpired = 4,
}

#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Empty,
    Deposited,
    Released,
    Refunded,
}

// `symbol_short!` only accepts up to 9 characters, hence "BENEFICI" rather
// than the full "BENEFICIARY".
const DEPOSITOR: Symbol = symbol_short!("DEPOSITOR");
const BENEFICIARY: Symbol = symbol_short!("BENEFICI");
const DEADLINE: Symbol = symbol_short!("DEADLINE");
const STATUS: Symbol = symbol_short!("STATUS");
const AMOUNT: Symbol = symbol_short!("AMOUNT");

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    pub fn __constructor(env: Env, depositor: Address, beneficiary: Address, deadline: u64) {
        env.storage().instance().set(&DEPOSITOR, &depositor);
        env.storage().instance().set(&BENEFICIARY, &beneficiary);
        env.storage().instance().set(&DEADLINE, &deadline);
        env.storage().instance().set(&STATUS, &Status::Empty);
        env.storage().instance().set(&AMOUNT, &0i128);
    }

    pub fn deposit(env: Env, amount: i128) -> Result<(), Error> {
        let depositor: Address = env.storage().instance().get(&DEPOSITOR).unwrap();
        depositor.require_auth();

        let status: Status = env.storage().instance().get(&STATUS).unwrap();
        if status != Status::Empty {
            return Err(Error::AlreadyDeposited);
        }
        env.storage().instance().set(&AMOUNT, &amount);
        env.storage().instance().set(&STATUS, &Status::Deposited);
        Ok(())
    }

    /// Depositor-authorized early release to the beneficiary, only while
    /// the deadline hasn't passed yet.
    pub fn release(env: Env) -> Result<(), Error> {
        let depositor: Address = env.storage().instance().get(&DEPOSITOR).unwrap();
        depositor.require_auth();

        let status: Status = env.storage().instance().get(&STATUS).unwrap();
        if status != Status::Deposited {
            return Err(Error::NotDeposited);
        }
        let deadline: u64 = env.storage().instance().get(&DEADLINE).unwrap();
        if env.ledger().timestamp() >= deadline {
            return Err(Error::AlreadyExpired);
        }
        env.storage().instance().set(&STATUS, &Status::Released);
        Ok(())
    }

    /// Permissionless: once the deadline has passed, anyone can trigger the
    /// refund back to the depositor (nobody should have to trust a
    /// particular party to unblock stuck funds after a timeout).
    pub fn refund(env: Env) -> Result<(), Error> {
        let status: Status = env.storage().instance().get(&STATUS).unwrap();
        if status != Status::Deposited {
            return Err(Error::NotDeposited);
        }
        let deadline: u64 = env.storage().instance().get(&DEADLINE).unwrap();
        if env.ledger().timestamp() < deadline {
            return Err(Error::NotYetExpired);
        }
        env.storage().instance().set(&STATUS, &Status::Refunded);
        Ok(())
    }

    pub fn status(env: Env) -> Status {
        env.storage().instance().get(&STATUS).unwrap()
    }
}
