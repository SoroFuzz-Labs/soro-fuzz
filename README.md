# soro-fuzz

A reusable fuzzing and property-testing harness for [Soroban](https://developers.stellar.org/docs/build/smart-contracts/overview)
contracts: declare a command model, a shadow (reference) model, and a set of
invariants for your contract, and get both coverage-guided fuzzing
(`cargo-fuzz`) and CI-friendly property tests (`proptest`) out of the same
declaration — instead of hand-rolling a bespoke fuzz crate per contract.

## Why this exists

Soroban already ships real building blocks for this:

- `soroban-sdk`'s `testutils::arbitrary` module, the `SorobanArbitrary`
  trait, and `proptest-arbitrary-interop` give you structured, `Arbitrary`-driven
  input generation for Soroban types, bridged into `proptest`.
- `cargo-fuzz` + `libfuzzer-sys` already work against contracts running in
  the host environment (`Env::default()`), the same way `soroban-sdk` unit
  tests do.
- [brson's `soroban-token-fuzzer`](https://github.com/brson/soroban-token-fuzzer)
  already proves out the "reusable driver + per-contract wiring" pattern —
  for token contracts specifically.

None of that gives you a *contract-agnostic* command-sequence-plus-shadow-model-plus-invariant
harness. `soro-fuzz` is the generic version of the token-fuzzer's pattern:
implement four small traits for your contract and you get command
generation, arbitrary auth-subset generation, shadow-state tracking, and a
reusable invariant library that goes beyond token semantics (no negative
balances, supply conservation, auth-required-for-mutation — plus your own).
If a check already lives in the SDK or only makes sense for tokens, it
doesn't belong in `crates/core`; that crate has zero contract-specific
assumptions.

**Scope**: contracts run in the host environment (`Env::default()`,
`env.register(..)`) — exactly like `soroban-sdk` unit tests, *not* the guest
WASM VM. Guest/WASM-level fuzzing is explicitly out of scope; see
[Future work](#future-work).

## Execution model

```
   ┌─────────────────────────────────────────────────────────────────┐
   │  arbitrary input (bytes)                                        │
   └───────────────────────────────┬─────────────────────────────────┘
                                    │  Arbitrary::arbitrary
                                    ▼
   ┌─────────────────────────────────────────────────────────────────┐
   │  Run<Command>  =  Vec<Step>                                     │
   │    Step = { command, authorized: AuthSelection, advance_time }  │
   └───────────────────────────────┬─────────────────────────────────┘
                                    │  Harness::run  (per Step, in order)
                                    ▼
        ┌───────────────────────────────────────────────────┐
        │ 1. advance ledger time, if this step asks for it   │
        │ 2. resolve AuthSelection -> concrete Addresses     │
        │ 3. Command::execute(ctx, authorizers)              │
        │      -> mocks auth, calls the contract's try_*     │
        │      -> Outcome::{Ok, DeclaredError, Rejected,     │
        │                    UndeclaredPanic}                │
        │ 4. Command::apply_to_model(model, addresses, out)  │
        │ 5. run every registered Invariant::check(ctx)      │
        └───────────────────────────────────────────────────┘
                                    │
                     UndeclaredPanic or a failed Invariant
                                    ▼
                         Err(Violation)  — a finding
```

The same `Run<Command>` type and the same `Harness` drive both entry points:

- `fuzz/fuzz_targets/*.rs` — coverage-guided, via `cargo-fuzz` (nightly,
  Linux/macOS/WSL — see the [Windows caveat](#windows--cargo-fuzz-caveat)).
  A `Violation` is turned into a `panic!` here, since that's what libFuzzer
  detects as a crash.
- `examples/*/tests/proptest_mirror.rs` — the same generation logic driven by
  `proptest` via `proptest-arbitrary-interop`, runs under plain `cargo test`
  on stable. **This is what CI runs.**

`panic_with_error!`/declared-error semantics: the only acceptable failure
modes are the contract's own declared errors (`Outcome::DeclaredError`) and
host-level rejections like a failed `require_auth` (`Outcome::Rejected`).
Anything else that panics is caught by the harness (`catch_unwind`) and is
*always* a finding — this is enforced unconditionally in `Harness::run`, not
left as an optional/disable-able `Invariant`.

## Workspace layout

| Path | What |
|---|---|
| `crates/core` | The engine: `Harness`, the `ContractAdapter`/`Command`/`ReferenceModel`/`Invariant` traits, address-pool + auth generation. Zero contract-specific assumptions. |
| `crates/strategies` | Reusable `Arbitrary` types for common Soroban shapes: `BoundedI128` (edge-biased bounded amounts), `AddressIndex` (pool references), `BoundedEntries` (capped `Map<K,V>` generation), `TimeAdvance` (edge-biased ledger-time jumps). |
| `crates/invariants` | A small library of ready-made `Invariant` impls, gated behind accessor traits (`HasBalances`, `HasTotalSupply`, `RequiresAuthorizer`) so they apply to any adapter whose model exposes the right data. |
| `examples/counter` | Simplest possible contract: `increment`/`decrement`/admin-gated `reset`/`set`. The vehicle for the walkthrough below. |
| `examples/token` | A small fungible token (mint/burn/transfer). Wires the shared invariants — proves the harness generalizes past a toy example. |
| `examples/escrow` | Deposit-then-release-or-refund with a deadline. A state machine and a strategy (`TimeAdvance`) with nothing in common with the token example, to prove the harness isn't secretly token-shaped. |
| `fuzz/` | The `cargo-fuzz` driver crate — one `fuzz_targets/*.rs` per example. Deliberately its own workspace root (see its `Cargo.toml`), excluded from the main workspace and from CI. |

## The traits you implement (the extension surface)

- **`ContractAdapter`** — how to register your contract (`setup`) and build
  its initial shadow state from the run's address pool.
- **`Command`** — one generatable, executable action. `execute` calls the
  contract (mocking whatever auth it needs from the `authorizers` it's
  given); `apply_to_model` updates the shadow state to match.
- **`ReferenceModel`** — a marker trait for your shadow-state struct. No
  `Default` bound: your model's initial state comes from
  `ContractAdapter::setup`, not `Default::default()` — most non-trivial
  models need to store at least one `Address`, which has no meaningful
  default without a live `Env`.
- **`Invariant`** — `check(&self, ctx) -> Result<(), Violation>`, run after
  every step. Write one inline for contract-specific checks (see
  `CounterValueMatchesModel`), or add a reusable one to
  `soro-fuzz-invariants` behind a small accessor trait.

Strategy registration isn't a runtime registry — it's just composition: add
a field of the strategy's type (e.g. `soro_fuzz_strategies::AddressIndex`)
to your `Command` enum and derive `Arbitrary` on the enum as normal.

## Wire your own contract: the counter walkthrough

1. **Write the contract** (`examples/counter/src/lib.rs`) as you normally
   would with `soroban-sdk`. Nothing fuzzing-specific here.
2. **Gate a `harness` module behind `testutils`**
   (`examples/counter/src/lib.rs`):
   ```rust
   #[cfg(any(test, feature = "testutils"))]
   pub mod harness;
   ```
3. **Define your shadow model and command enum**
   (`examples/counter/src/harness.rs`):
   ```rust
   #[derive(Debug, Clone, Default)]
   pub struct CounterModel { pub expected_count: i64 }
   impl ReferenceModel for CounterModel {}

   #[derive(Arbitrary, Debug, Clone)]
   pub enum CounterCommand { Increment, Decrement, Reset, Set(SetValue) }
   ```
4. **Implement `ContractAdapter::setup`** — register the contract, assign
   roles out of the address pool (by convention, index 0 is "the admin"),
   build the initial model.
5. **Implement `Command::execute`** — mock whatever auth the call needs
   (`env.mock_auths(&[MockAuth { .. }])`) based on whether the required
   signer is in `authorizers`, then call the generated client's `try_*`
   method and classify the result into an `Outcome`.
6. **Implement `Command::apply_to_model`** — mirror what a successful call
   does to the contract's real state.
7. **Add an invariant** — either inline (contract-specific) or from
   `soro-fuzz-invariants` (reusable).
8. **Wire the two entry points**, both driving the same `Harness`:
   - `examples/counter/tests/proptest_mirror.rs` (`proptest!` +
     `proptest_arbitrary_interop::arb`)
   - `fuzz/fuzz_targets/counter_fuzz.rs` (`libfuzzer_sys::fuzz_target!`,
     turning a `Violation` into a `panic!`)

Read `examples/counter/src/harness.rs` end to end — it's short and every
step above maps to a specific block in that file.

## The `testutils` setup gotcha

This is the single most common mistake wiring a new contract in: the
`testutils` feature has to be enabled on **both** `soroban-sdk` *and your
contract crate* for `Address::generate`, `env.mock_auths`, and the
`Arbitrary`/`SorobanArbitrary` derivations to exist at all. Every example
crate here handles this via its own `testutils` feature that forwards to
`soroban-sdk/testutils`, on by default:

```toml
# examples/<name>/Cargo.toml
[features]
default = ["testutils"]
testutils = ["soroban-sdk/testutils"]
```

with the harness module itself gated the same way in `lib.rs`:

```rust
#[cfg(any(test, feature = "testutils"))]
pub mod harness;
```

`default = ["testutils"]` is what makes `cargo build`/`cargo test` just work
without extra flags. If you're adapting this pattern for a contract you
intend to actually deploy, build the release wasm with
`--no-default-features` so none of the test-only code ships in the on-chain
binary.

Relatedly: these example crates deliberately build as `rlib` only (not
`cdylib`) — see the comment in `examples/counter/Cargo.toml`. Building
`cdylib` natively on Windows/MinGW hits a linker export-ordinal limit given
how large `soroban-env-host`'s dependency tree is; if you want an actual
deployable `.wasm` from a contract like these, add `cdylib` back and build
with `--target wasm32-unknown-unknown`.

## Windows / cargo-fuzz caveat

`cargo-fuzz`/`libfuzzer-sys` need LLVM sanitizer-coverage instrumentation,
which isn't supported on native Windows (`error: address sanitizer is not
supported for this target`) — this was confirmed while building this repo,
not a hypothetical. Everything else (`cargo build --workspace`,
`cargo test --workspace`, the proptest mirrors) works natively on Windows.
To actually run `cargo +nightly fuzz run <target>`, use WSL, Linux, or
macOS. There's also a historical linking issue on macOS/arm64 with older
`cargo-fuzz` versions; if `cargo fuzz build` fails to link there, try
updating `cargo-fuzz` first (`cargo install cargo-fuzz --force`).

## Quick start

```sh
# Build everything except fuzz/ (stable toolchain)
cargo build --workspace

# Run the proptest mirrors — this is what CI runs
cargo test --workspace

# Run a fuzz target for real (nightly, Linux/macOS/WSL only)
cargo +nightly fuzz run counter_fuzz
```

## Future work (explicitly out of scope for now)

- Guest/WASM-level fuzzing (running the actual compiled `.wasm` through the
  VM, rather than the host environment).
- Alternative fuzz engines (`afl`, `honggfuzz`).
- More reusable invariants and strategies — see CONTRIBUTING.md; this is
  meant to be the easiest kind of issue to pick up.

## License

Apache-2.0 — see [LICENSE](LICENSE).
