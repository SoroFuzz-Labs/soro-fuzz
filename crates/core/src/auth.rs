//! Address pool generation and arbitrary auth-subset selection.
//!
//! The harness generates a fixed-size pool of addresses once per run (via
//! [`AddressPool::generate`]) and every [`Step`](crate::command::Step) picks
//! an arbitrary subset of that pool to have "signed" the step. Resolving an
//! [`AuthSelection`] against a concrete pool is deferred to run time so the
//! `Arbitrary` impl never needs to know the pool size.

use arbitrary::Arbitrary;
use soroban_sdk::{testutils::Address as _, Address, Env};

/// A fixed set of addresses generated once per harness run, shared by every
/// step. Contributors' [`ContractAdapter::setup`](crate::adapter::ContractAdapter::setup)
/// impls typically assign roles (e.g. "the admin is `addresses[0]`") out of
/// this pool.
#[derive(Debug, Clone)]
pub struct AddressPool {
    addresses: Vec<Address>,
}

impl AddressPool {
    /// Generates `size` fresh addresses in `env`. `size` must be at least 1;
    /// the harness builder enforces a sane default and minimum.
    pub fn generate(env: &Env, size: usize) -> Self {
        let addresses = (0..size.max(1)).map(|_| Address::generate(env)).collect();
        Self { addresses }
    }

    pub fn addresses(&self) -> &[Address] {
        &self.addresses
    }

    pub fn get(&self, index: usize) -> &Address {
        &self.addresses[index % self.addresses.len()]
    }

    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }

    /// Resolves an [`AuthSelection`] into the concrete addresses it refers
    /// to, deduplicated, in the order they first appear.
    pub fn resolve(&self, selection: &AuthSelection) -> Vec<Address> {
        let mut out: Vec<Address> = Vec::new();
        for &raw_index in &selection.indices {
            let addr = self.get(raw_index as usize).clone();
            if !out.contains(&addr) {
                out.push(addr);
            }
        }
        out
    }
}

/// An arbitrary subset of the run's [`AddressPool`], expressed as raw
/// indices so it can be generated independently of the pool's size (the
/// pool doesn't exist yet when `arbitrary()` runs). Indices are resolved
/// modulo the pool length at execution time via [`AddressPool::resolve`].
///
/// contributors: this is deliberately the *only* auth-generation primitive
/// core provides. Building the actual `MockAuth` invocation tree for a given
/// contract call is inherently specific to that call's function name and
/// argument shape, so it lives in each `Command::execute` impl instead.
#[derive(Arbitrary, Debug, Clone, Default)]
pub struct AuthSelection {
    indices: Vec<u8>,
}

impl AuthSelection {
    pub fn none() -> Self {
        Self {
            indices: Vec::new(),
        }
    }

    /// Builds a selection from explicit raw pool indices. Mainly useful for
    /// hand-written unit tests and proptest strategies that want to target
    /// a specific address rather than let `Arbitrary` pick one.
    pub fn from_indices(indices: Vec<u8>) -> Self {
        Self { indices }
    }
}
