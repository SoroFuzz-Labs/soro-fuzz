//! A bounded, generatable set of key/value entries that converts into a
//! `soroban_sdk::Map<K, V>`.
//!
//! `soroban_sdk::Map` can't implement `Arbitrary` directly (like `Address`,
//! it needs a live `Env` to construct), so this generates plain Rust
//! `(K, V)` pairs and hands you [`BoundedEntries::to_map`] to build the
//! real `Map` once you have an `Env` in hand (typically inside
//! `Command::execute`, from `ctx.env`).

use arbitrary::{Arbitrary, Unstructured};
use soroban_sdk::{Env, IntoVal, Map, TryFromVal, Val};

/// Caps how many entries a generated map can have, so runs stay bounded
/// regardless of how much input data `arbitrary` has to work with.
pub const MAX_MAP_ENTRIES: usize = 8;

/// Arbitrary key/value pairs, capped at [`MAX_MAP_ENTRIES`] entries.
#[derive(Debug, Clone)]
pub struct BoundedEntries<K, V> {
    pub entries: std::vec::Vec<(K, V)>,
}

impl<'a, K, V> Arbitrary<'a> for BoundedEntries<K, V>
where
    K: Arbitrary<'a>,
    V: Arbitrary<'a>,
{
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.int_in_range(0..=MAX_MAP_ENTRIES)?;
        let mut entries = std::vec::Vec::with_capacity(len);
        for _ in 0..len {
            entries.push((K::arbitrary(u)?, V::arbitrary(u)?));
        }
        Ok(Self { entries })
    }
}

impl<K, V> BoundedEntries<K, V>
where
    K: Clone + IntoVal<Env, Val> + TryFromVal<Env, Val>,
    V: Clone + IntoVal<Env, Val> + TryFromVal<Env, Val>,
{
    /// Builds the real `soroban_sdk::Map`, last-write-wins on duplicate
    /// keys (matching `Map::set` semantics).
    pub fn to_map(&self, env: &Env) -> Map<K, V> {
        let mut map = Map::new(env);
        for (k, v) in &self.entries {
            map.set(k.clone(), v.clone());
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn caps_entry_count() {
        let raw = [1u8; 512];
        let mut u = Unstructured::new(&raw);
        for _ in 0..16 {
            let entries = BoundedEntries::<u32, i64>::arbitrary(&mut u).unwrap();
            assert!(entries.entries.len() <= MAX_MAP_ENTRIES);
        }
    }

    #[test]
    fn converts_to_soroban_map() {
        let env = Env::default();
        let entries = BoundedEntries::<u32, i64> {
            entries: std::vec![(1u32, 10i64), (2u32, 20i64)],
        };
        let map = entries.to_map(&env);
        assert_eq!(map.get(1).unwrap(), 10);
        assert_eq!(map.get(2).unwrap(), 20);
        assert_eq!(map.len(), 2);
    }
}
