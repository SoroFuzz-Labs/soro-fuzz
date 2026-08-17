# Contributing to soro-fuzz

Thanks for looking at this. The skill floor here is higher than a typical
CRUD-app issue — you need a working mental model of `soroban-sdk`'s test
environment (host-env execution, `mock_auths`, declared vs. undeclared
errors) to make a useful change. That's exactly why issues here are scoped
narrowly: **one invariant, one strategy, or one example contract per
issue**, each with a concrete contract to run it against so you have
something to check your work with immediately.

## Setup

```sh
git clone <this repo>
cd soro-fuzz
cargo build --workspace   # stable toolchain, no extra flags needed
cargo test --workspace    # runs the proptest mirrors
```

If you also want to run the `cargo-fuzz` targets directly (optional —
CI never does this): nightly toolchain, `cargo install cargo-fuzz`, and
Linux/macOS/WSL (`cargo-fuzz` doesn't build on native Windows — see the
README's Windows caveat). Then: `cargo +nightly fuzz run counter_fuzz`.

Read the README's "Wire your own contract" walkthrough and
`examples/counter/src/harness.rs` before your first PR — every extension
point below maps directly to something in that one file.

## Adding a reusable invariant

Lives in `crates/invariants/src/<name>.rs`. Pattern (see
`no_negative_balance.rs` for the smallest complete example):

1. Define a small accessor trait describing what your invariant needs from
   a `ReferenceModel` (e.g. `HasBalances`) — never depend on a specific
   contract's model type directly.
2. Define your invariant as a unit struct implementing
   `Invariant<A> where A: ContractAdapter, A::Model: YourAccessorTrait`.
3. Add 2+ unit tests using `crate::test_support::FakeAdapter` (a throwaway
   `ContractAdapter` genericized over a fake command/model, so you don't
   need a real running contract to test the invariant's logic in
   isolation) — one that proves it fires on a real violation, one that
   proves it doesn't false-positive on legitimate state.
4. Re-export it from `crates/invariants/src/lib.rs`.
5. Wire it into at least one example (`.with_invariant(YourInvariant)` in
   that example's `harness()` test helper and its `proptest_mirror.rs`) so
   there's a real contract proving it isn't a false positive/negative
   generator.

Good invariant ideas are ones that would be genuinely surprising to violate
if the contract is correct, and that don't already exist in `soroban-sdk`
or duplicate the harness's own built-in "no undeclared panic" guarantee
(that one's structural, not a pluggable `Invariant` — see the README).

## Adding a reusable strategy

Lives in `crates/strategies/src/<name>.rs`. Pattern (see `amounts.rs`):

1. Define your type and implement `arbitrary::Arbitrary` for it by hand
   (not `#[derive]`) if you want edge-case biasing — most numeric/boundary
   strategies should spend a meaningful fraction of generation (roughly a
   quarter, via `Unstructured::ratio`) on boundary values rather than pure
   uniform sampling, since that's where off-by-one and
   overflow/underflow bugs cluster.
2. Add unit tests proving generated values stay in bounds and boundary
   values are actually reachable.
3. Re-export it from `crates/strategies/src/lib.rs`.
4. Compose it into at least one example's `Command` enum field to prove it
   actually plugs in (see `examples/counter/src/harness.rs`'s `SetValue`
   for the smallest example, or `examples/escrow`'s `TimeAdvance` usage for
   one that models something other than a plain argument — an explicit
   `Command` variant).

`soroban_sdk::Address` and `soroban_sdk::Map`/`Vec` can't implement
`Arbitrary` directly — they need a live `Env` to construct, which
`arbitrary()` doesn't have. The established pattern (see `address_ref.rs`,
`map.rs`) is: generate a plain-Rust-typed proxy (an index, a `Vec<(K, V)>`),
and resolve it into the real Soroban type inside `Command::execute`/
`apply_to_model`, where an `Env` and `AddressPool` are available.

## Wiring a new example contract

Lives in `examples/<name>/`. Follow the counter walkthrough in the README
step by step. Checklist:

- [ ] Contract in `src/lib.rs`, no fuzzing-specific code in it.
- [ ] `#[cfg(any(test, feature = "testutils"))] pub mod harness;` in `lib.rs`.
- [ ] `Cargo.toml` has `default = ["testutils"]` /
      `testutils = ["soroban-sdk/testutils"]` and `crate-type = ["rlib"]`
      (see `examples/counter/Cargo.toml`'s comment for why not `cdylib`).
- [ ] `ContractAdapter`, `Command`, `ReferenceModel` impls in `src/harness.rs`.
      Classify every `try_*` call's result with
      `Outcome::from_try_result(result, required_auth_satisfied, code_of)` —
      not a hand-written match on the nested `Result` — so a real undeclared
      panic can't get misclassified as a benign auth rejection (see the
      README's "Execution model" section for why those are otherwise
      indistinguishable).
- [ ] At least one invariant wired (inline, from `soro-fuzz-invariants`, or
      both).
- [ ] Unit tests in `harness.rs` covering: a successful path, an
      unauthorized call being `Rejected` (not a panic), and a declared-error
      path (not a panic).
- [ ] `tests/proptest_mirror.rs` driving the same `Harness` via
      `proptest_arbitrary_interop::arb`.
- [ ] A `fuzz/fuzz_targets/<name>_fuzz.rs` + a matching `[[bin]]` entry in
      `fuzz/Cargo.toml`.
- [ ] Added to the root `Cargo.toml` workspace `members`.

Pick a contract with a state machine that's actually different from the
existing examples (counter: single counter, no cross-address concerns;
token: balances/supply conservation; escrow: single-shot deposit with a
deadline) — an example that's structurally identical to an existing one
doesn't add much proof that the harness generalizes.

## Alternative fuzz engines / WASM-level fuzzing

Explicitly future work, not in scope for a first PR — see the README's
"Future work" section. If you want to pursue one, open an issue to discuss
the approach first; these are larger, more architectural changes than the
invariant/strategy/example issues above.
