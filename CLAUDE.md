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
- **`docs/TACTICS_MODEL.md`** — the Phase 2e tactics design record, now *implemented and
  calibrated*: the four-instruction surface, per-side resolution into effective knobs, the
  neutral-tactics bit-for-bit invariant, the interaction model, the `Tactics`-on-`Lineup`
  event-log seam, and the AI tactics policy / Phase-5 seam (live —
  `match_engine::AI_TACTICS_ENABLED = true`). Read **§3's lever-class note** before touching
  any effect magnitude: §3's table mixes *advance-class* multipliers (on raw transition
  probabilities) with *logit-class* biases (additive, through a near-saturated sigmoid), and
  the former move ~4× the absolute probability the latter do. Two separate calibration passes
  failed by reaching for the weak lever before that was understood (§5's T7-R finding, §9
  items 6–7). The rest of Phase 2e (condition/recovery, injuries, fouls & cards,
  substitutions, ratings, character activation) is `MATCH_MODEL.md` §11–§18 and is likewise
  implemented; set pieces stay deferred beyond 2e (`MATCH_MODEL.md` §11).

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
against it, and guards the result with `favourite_discrimination_regression_guard`.

**Phase 2e is complete.** Everything deferred behind the `play_match` call site at 2a has
landed, in the sequencing `MATCH_MODEL.md` §11 set: the §12 `MatchOutcome`/`MatchPlayed`
boundary (resolved injuries/cards/ratings/minutes, the fold's derived-suspension bookkeeping,
the §13 rolling appearance window), then tactics (`TACTICS_MODEL.md`), consistency (§17),
condition & recovery (§13), injuries (§14), fouls/cards/suspensions (§15), substitutions
(§16), and match ratings & form (§18). Set pieces remain deferred beyond 2e (§11).

The tactics model needed two calibration passes after its first landing, and the reason is
worth carrying forward: §3's effect table mixes two lever classes of very different power
(see `TACTICS_MODEL.md` §3), which left `Tempo::Direct` and then `Mentality::Attacking`
strictly dominant. Both are fitted now — no instruction dominates, non-dominance is
squad-conditional (§9 item 6), and `AI_TACTICS_ENABLED` is `true`, so every AI-controlled
side picks real tactics. Pooled goals/match reads **2.50** (sd 0.31, 24 seeds) with the whole of
2e live, AI substitutions on, and the seeding fix below landed.

**AI substitutions are live** (`MATCH_MODEL.md` §16's "v1 AI bench-selection and
default-substitution-plan policy"): `ai_pick_lineup` fills a real bench and default `sub_plan` for
every AI-controlled side, closing T12's last deferred seam. Measured, it fires far less than the
design note predicted (0.13 subs/match against a predicted 3–5) — the refutations and their root
causes are recorded in §16's own prediction block rather than fitted away.

Phase 3 (player development, `DEVELOPMENT_MODEL.md`) is implemented in `fforge-core::development`
— a monthly `DevelopmentTick` records resolved attribute deltas the fold integer-adds. Its
harness (`fforge-core::career_arc`, `bin/career_arc`) drives real multi-season runs and has
re-fit the knob table, guarded by `career_arcs_are_in_a_believable_ballpark`.

**The wonderkid seeding fix is landed and Phase 5 is unblocked** (`DEVELOPMENT_MODEL.md` §8,
`BACKLOG.md` §2 — closed). `worldgen::gen_player` used to derive `potential = best_ca + headroom`,
CA first, while §2.1 had always specified the opposite; it now draws PA first and seeds attributes
on the age envelope beneath it, reusing the growth engine's own ceiling machinery. **This is the
project's first *note-wins* doc-vs-code reconciliation** — §8.2 records the tie-break criterion
(resolve toward whichever side a real, identified downstream consumer depends on) against two prior
code-wins precedents. `DevKnobs` was re-fit in the same change, since the old values were
compensations for the bug. The point of it: PA is no longer trivially recoverable from `(CA, age)`
(`residual_sd` 2.61 → 5.57), which is what Phase 5's scouting fog-of-war structurally needs.

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
extension (`Money`, `Contract`, `Finances`, `Club.reputation` — `TRANSFER_MODEL.md` §3) and
the sanctioned Phase 2e extension, now fully landed: `Tactics`/`Lineup.tactics`,
`Lineup.bench`/`Lineup.sub_plan` and the `substitution` module, `Character.natural_fitness`,
and `Player.injured_until` (`MATCH_MODEL.md` §12). Note the one divergence from §12's original
list: `Player.condition` was never added and should not be — condition is *derived* from
`GameState::recent_appearances` rather than stored (`MATCH_MODEL.md` §13), the same
"derive, don't store" rule CA already follows. Beyond these, not open-ended new features.

**Next: Phase 5, the agent layer.** Its only blocker (§2 of `BACKLOG.md`) is closed, so
`AGENT_MODEL.md` is the next thing to write — see `BACKLOG.md` §3 for what it must resolve. One
finding from the seeding fix belongs in that conversation: the *naive* attack on PA (fit on
`(CA, age)`) and the *competent* one (fit on the per-`DevCategory` composites plus age) are
near-identical in accuracy at ages 16–18, and only diverge at 19–21. Fog now exists, but at the
wonderkid ages there is little skill in seeing through it — which the ablation's
decision-quality axis has to reckon with rather than assume.
