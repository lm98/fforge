# Football Forge (fforge)
Rust-based football manager simulation environment.

## Workspace layout

```
fforge/
├── fforge-domain   (Layer 1: domain model)
├── fforge-core     (Layer 2: simulation engines — depends on fforge-domain)
└── fforge-game     (CLI binary — depends on both)
```

`fforge-domain` provides the core domain types of the simulator. `fforge-core` is the primary consumer; it runs the Phase 2a
event-based possession match engine (`play_match`, in `fforge-core::match_engine`), the Phase 3 monthly development fold
(`fforge-core::development`), and the Phase 4 transfer market (valuation, club decision AI, the deferred-acceptance clearing
loop, and the player pool — `fforge-core::{valuation, club_ai, market, pool}`). `fforge-game` wires everything into the CLI.

## Design documents

All design decisions originate in these files at the workspace root:

- **`docs/ATTRIBUTE_SCHEMA.md`** — attribute list, rating scale, role→attribute weight
  table, CA/PA semantics, Character fields. The code in `fforge-domain` is a transcription of this document.
- **`docs/DESIGN.md`** — project vision, five-layer architecture, simulation subsystem
  specs, LLM agent interface, development phases. Read §3 (architecture) and §9 (phases)
  before adding anything new.
- **`docs/MATCH_MODEL.md`** — the Phase 2a match engine design record: the five-zone
  state space, actor-centric resolution model, the wide route, the role→zone presence
  table, and the calibration knobs/targets. `fforge-core::match_engine` is a Rust
  transcription of this document (and of the notebook it pins).
- **`docs/DEVELOPMENT_MODEL.md`** — the Phase 3 player-development design record: the
  PA-scaled age envelope, per-`DevCategory` curve parameters, the `DevelopmentTick`
  event-log seam, and the career-arc calibration harness. `fforge-core::development` is
  a Rust transcription of this document.
- **`docs/TRANSFER_MODEL.md`** — the Phase 4 transfer-market design record: the
  centralized valuation function, club decision AI, the simultaneous deferred-acceptance
  clearing loop, club finances, the player pool (youth intake/retirement), and the market
  pathology harness. `fforge-core::{valuation, club_ai, market, pool}` is a Rust
  transcription of this document.
- **`docs/TACTICS_MODEL.md`** — the Phase 2e tactics design record (*drafted,
  pre-implementation*): the four-instruction surface, per-side resolution into effective
  knobs, the neutral-tactics bit-for-bit invariant, the structural rock-paper-scissors
  interaction model, the `Tactics`-on-`Lineup` event-log seam, and the AI tactics policy /
  Phase-5 seam. The rest of Phase 2e (condition/recovery, injuries, fouls & cards,
  substitutions, ratings, character activation) is drafted as `MATCH_MODEL.md` §11–§18. No
  2e Rust exists yet — these notes are the gate (design-note-first).

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

Phase 0 (design & data model) is complete. Phase 1 (walking skeleton) is complete.
Phase 2a (the event-based possession match engine, `MATCH_MODEL.md`) is implemented and
calibrated: the Rust harness (`fforge-core::match_engine::calibrate`, `bin/calibrate`) runs
real `worldgen` + `ai_pick_lineup` + `play_match` pooled over many seeds, re-fit `b_beat`
against it, and guards the result with `favourite_discrimination_regression_guard`. Deferred
to Phase 2e behind the same `play_match` call site: tactics, cards/fouls, injuries, set
pieces, substitutions, and the character/hidden attributes. The 2e *design* is now drafted
(`TACTICS_MODEL.md`; `MATCH_MODEL.md` §11–§18) — no 2e Rust yet; set pieces stay deferred
beyond 2e (`MATCH_MODEL.md` §11).

Phase 3 (player development, `DEVELOPMENT_MODEL.md`) is implemented in `fforge-core::development`
— a monthly `DevelopmentTick` records resolved attribute deltas the fold integer-adds. Its
harness (`fforge-core::career_arc`, `bin/career_arc`) drives real multi-season runs and has
re-fit the knob table, guarded by `career_arcs_are_in_a_believable_ballpark`.

Phase 4 (transfer market, `TRANSFER_MODEL.md`) is implemented end to end: the centralized
valuation function (`fforge-core::valuation`), club decision AI (`club_ai`), the simultaneous
deferred-acceptance clearing loop and window mechanics (`market`), club finances (`finance`),
and the player pool — youth intake and retirement (`pool`). Its pathology harness
(`fforge-core::market::calibrate`, `bin/market`) pools many seeds × ~15 seasons and drove the
re-fit of `ValueKnobs::beta` and `FinanceKnobs::revenue_per_reputation` (`TRANSFER_MODEL.md` §9),
guarded by `market_is_in_a_believable_ballpark`. Deferred beyond v1: human transfer decisions,
loans, negotiation rounds, transfer clauses (`TRANSFER_MODEL.md` §1).

`fforge-core` is the active development front. Changes to `fforge-domain` at this stage are
corrections or clarifications to the Phase 0 deliverable plus the sanctioned Phase 4 finance
extension (`Money`, `Contract`, `Finances`, `Club.reputation` — `TRANSFER_MODEL.md` §3) and,
once Phase 2e implementation begins, the sanctioned 2e extension (`Tactics`/`Lineup.tactics`,
`Lineup.bench`, `Character.natural_fitness`, `Player.condition`/`Player.injured_until` —
`MATCH_MODEL.md` §12), not open-ended new features.
