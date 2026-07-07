# Football Forge (fforge)
Rust-based football manager simulation environment.

## Workspace layout

```
fforge/
├── fforge-domain   (Layer 1: domain model)
├── fforge-core     (Layer 2: simulation engines — depends on fforge-domain)
└── fforge-game     (CLI binary — depends on both)
```

`fforge-domain` provides the core domain types of the simulator. `fforge-core` is the primary consumer; it is currently implementing the Phase 1 crude
Poisson match engine. `fforge-game` wires everything into the CLI.

## Design documents

All design decisions originate in two files at the workspace root:

- **`docs/ATTRIBUTE_SCHEMA.md`** — attribute list, rating scale, role→attribute weight
  table, CA/PA semantics, Character fields. The code in `fforge-domain` is a transcription of this document.
- **`docs/DESIGN.md`** — project vision, five-layer architecture, simulation subsystem
  specs, LLM agent interface, development phases. Read §3 (architecture) and §9 (phases)
  before adding anything new.

When the code and the design docs diverge, treat the design docs as authoritative and
file the discrepancy as a bug.

## Hard constraints — never violate these

1. **CA is derived, never stored.** `current_ability()` is the only path to a CA value.
   Attributes are the source of truth; CA is a view. There is no sync bug possible by
   construction; storing CA would break that guarantee.

2. **`BTreeMap` only — never `HashMap`.** Determinism is load-bearing for the entire
   architecture (reproducible bugs, replayable matches, calibration). `HashMap`
   iteration order is nondeterministic. Use `BTreeMap` for any collection that could
   affect game state.

3. **No I/O, RNG, or wall-clock time in this crate.** All impure sources live at the
   edges (in `fforge-core` or `fforge-game`). If you need randomness or a timestamp,
   it is passed in as an argument; it is never sourced here.

## Lint policy

The crates enforce `#![deny(unsafe_code)]`. Do not add `unsafe` blocks. Keep Clippy
warnings clean (`cargo clippy -- -D warnings`).

## Testing

Tests live in `#[cfg(test)]` blocks inside each module. No external test framework —
plain `#[test]` functions. Tests should assert **invariants**, not round-trip the
implementation: the existing tests in `ability.rs` are the model (uniform input → uniform
CA; position-relative weighting; bounds hold at extremes).

## Common commands

```
cargo test                      # run all tests
cargo clippy -- -D warnings     # lint (must be clean)
cargo fmt                       # format
cargo doc --no-deps --open      # browse generated docs
```

Run these from either this crate's directory or the workspace root (add `-p fforge-domain`
in the latter case).

## Current phase

Phase 0 (design & data model) is complete. `fforge-core` is the active development
front, building the Phase 1 walking skeleton. Changes to `fforge-domain` at this stage
are corrections or clarifications to the Phase 0 deliverable, not new features.
