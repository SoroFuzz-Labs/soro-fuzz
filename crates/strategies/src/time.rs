//! An arbitrary ledger-time advance, for contracts with deadline/TTL logic
//! (see the escrow example).

use arbitrary::{Arbitrary, Unstructured};

/// Seconds to advance the ledger's timestamp by, biased toward `0`, `1`,
/// and a large multi-year jump — the boundary values most likely to trip
/// up deadline comparisons (`<` vs `<=`) and TTL bumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeAdvance(pub u32);

/// About five years in seconds — deliberately far past any reasonable
/// contract deadline, to exercise "well past expiry" behavior.
pub const LARGE_JUMP_SECS: u32 = 60 * 60 * 24 * 365 * 5;

impl<'a> Arbitrary<'a> for TimeAdvance {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        if u.ratio(1u32, 4u32)? {
            let choice: u8 = u.int_in_range(0..=2)?;
            let value = match choice {
                0 => 0,
                1 => 1,
                _ => LARGE_JUMP_SECS,
            };
            Ok(TimeAdvance(value))
        } else {
            Ok(TimeAdvance(u.arbitrary()?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_without_error() {
        let raw = [7u8; 64];
        let mut u = Unstructured::new(&raw);
        for _ in 0..16 {
            let _ = TimeAdvance::arbitrary(&mut u).unwrap();
        }
    }
}
