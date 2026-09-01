//! An edge-biased `Arbitrary` strategy for 32-byte arrays (hashes, keys),
//! for contracts taking a `BytesN<32>` argument.

use arbitrary::{Arbitrary, Unstructured};
use soroban_sdk::{BytesN, Env};

/// 32 arbitrary bytes, biased toward all-zero, all-ones, and fully random
/// about a quarter of the time — the boundary values most likely to trip
/// up hash/key comparison logic, the same rationale as [`BoundedI128`]'s
/// bias toward its range's edges.
///
/// `soroban_sdk::BytesN<32>` can't implement `Arbitrary` directly (like
/// `Address`/`Map`, it needs a live `Env` to construct via
/// `BytesN::from_array`), so this generates a plain `[u8; 32]` and hands
/// you [`Bytes32::to_bytes_n`] to build the real value once you have an
/// `Env` in hand (typically inside `Command::execute`, from `ctx.env`).
///
/// This is deliberately not redundant with `soroban-sdk`'s own
/// `testutils::arbitrary` support for `BytesN<N>`
/// (`<BytesN<32> as SorobanArbitrary>::Prototype`, `ArbitraryBytesN<N>`):
/// that type is a plain `#[derive(Arbitrary)]` over `[u8; N]` — uniform
/// random, no bias — so it rarely lands on the all-zero/all-ones edges a
/// hash or key comparison is most likely to mishandle. `Bytes32` exists
/// to bias toward exactly those edges.
///
/// [`BoundedI128`]: crate::BoundedI128
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bytes32(pub [u8; 32]);

impl Bytes32 {
    /// Builds the real `soroban_sdk::BytesN<32>`.
    pub fn to_bytes_n(&self, env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &self.0)
    }
}

impl<'a> Arbitrary<'a> for Bytes32 {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        if u.ratio(1u32, 4u32)? {
            let choice: u8 = u.int_in_range(0..=1)?;
            let value = match choice {
                0 => [0u8; 32],
                _ => [0xFFu8; 32],
            };
            Ok(Bytes32(value))
        } else {
            let mut bytes = [0u8; 32];
            u.fill_buffer(&mut bytes)?;
            Ok(Bytes32(bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_values_are_produced() {
        // A homogeneous buffer (all-0s, all-0xFFs) makes `Unstructured`'s
        // internal choices deterministic across every draw, so it can only
        // ever exercise one branch of the bias. Varied bytes are needed to
        // actually hit both the edge-case and uniform-random paths within a
        // reasonable sample count. So each draw gets its own `Unstructured`
        // over a buffer scrambled from its index (a fixed, reproducible
        // multiplicative hash, not real randomness).
        let mut saw_all_zero = false;
        let mut saw_all_ones = false;
        for seed in 0u32..256 {
            let mixed = seed.wrapping_mul(2654435761).wrapping_add(0x9E3779B9);
            let raw: std::vec::Vec<u8> = (0..64u32)
                .map(|i| (mixed.wrapping_add(i.wrapping_mul(2246822519)) >> 8) as u8)
                .collect();
            let mut u = Unstructured::new(&raw);
            let bytes = Bytes32::arbitrary(&mut u).unwrap().0;
            if bytes == [0u8; 32] {
                saw_all_zero = true;
            } else if bytes == [0xFFu8; 32] {
                saw_all_ones = true;
            }
        }
        assert!(saw_all_zero, "expected an all-zero draw across 256 samples");
        assert!(saw_all_ones, "expected an all-ones draw across 256 samples");
    }

    #[test]
    fn converts_to_soroban_bytes_n() {
        let env = Env::default();
        let bytes32 = Bytes32([7u8; 32]);
        let bytes_n = bytes32.to_bytes_n(&env);
        assert_eq!(bytes_n, BytesN::from_array(&env, &[7u8; 32]));
    }
}
