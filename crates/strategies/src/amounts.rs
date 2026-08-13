//! A bounded `i128` that composes into a contract's `Command` enum wherever
//! an amount, balance delta, or similar bounded quantity is needed.

use arbitrary::{Arbitrary, Unstructured};

/// An `i128` arbitrarily generated within `[MIN, MAX]` (inclusive), biased
/// toward the boundaries and near-boundaries about a quarter of the time —
/// overflow/underflow and off-by-one bugs cluster at the edges of a range,
/// and pure uniform sampling over a wide range rarely lands on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoundedI128<const MIN: i128, const MAX: i128>(i128);

impl<const MIN: i128, const MAX: i128> BoundedI128<MIN, MAX> {
    pub fn get(self) -> i128 {
        self.0
    }

    /// Hand-constructs a value, for tests and proptest strategies that want
    /// a specific value rather than whatever `Arbitrary` picks.
    ///
    /// # Panics
    /// Panics if `value` is outside `[MIN, MAX]`.
    pub fn new(value: i128) -> Self {
        assert!(
            (MIN..=MAX).contains(&value),
            "BoundedI128::new: {value} is outside [{MIN}, {MAX}]"
        );
        Self(value)
    }
}

impl<const MIN: i128, const MAX: i128> From<BoundedI128<MIN, MAX>> for i128 {
    fn from(value: BoundedI128<MIN, MAX>) -> Self {
        value.0
    }
}

impl<'a, const MIN: i128, const MAX: i128> Arbitrary<'a> for BoundedI128<MIN, MAX> {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        debug_assert!(MIN <= MAX, "BoundedI128: MIN must be <= MAX");
        if u.ratio(1u32, 4u32)? {
            let choice: u8 = u.int_in_range(0..=4)?;
            let value = match choice {
                0 => MIN,
                1 => MAX,
                2 => 0i128.clamp(MIN, MAX),
                3 => MIN.saturating_add(1),
                _ => MAX.saturating_sub(1),
            };
            Ok(BoundedI128(value))
        } else {
            Ok(BoundedI128(u.int_in_range(MIN..=MAX)?))
        }
    }
}

/// A non-negative `i128` in `[0, i128::MAX]`, for the common case of a
/// balance or transfer amount that can never legitimately be negative.
pub type NonNegativeI128 = BoundedI128<0, { i128::MAX }>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_within_bounds() {
        let raw = [0u8; 64];
        let mut u = Unstructured::new(&raw);
        for _ in 0..16 {
            let v = BoundedI128::<-10, 10>::arbitrary(&mut u).unwrap();
            assert!((-10..=10).contains(&v.get()));
        }
    }

    #[test]
    fn full_range_edge_cases_dont_overflow() {
        let raw = [255u8; 64];
        let mut u = Unstructured::new(&raw);
        let v = BoundedI128::<{ i128::MIN }, { i128::MAX }>::arbitrary(&mut u).unwrap();
        assert!(v.get() >= i128::MIN && v.get() <= i128::MAX);
    }
}
