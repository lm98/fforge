# Football Forge (fforge)

A deterministic, event-sourced football manager simulation in Rust.

```
fforge/
├── fforge-domain   layer 1: pure domain model
├── fforge-core     layer 2: simulation engines, a pure fold over an event log
└── fforge-game     layer 5: the CLI
```

Each crate has its own `CLAUDE.md` with its module map and local rules. Read the one for the
crate you are about to change.

## Where things stand

Phases 0–4 and 2e are complete, calibrated and guarded; set pieces are the one piece of 2e still
deferred. A slice of Phase 6 (UI) was pulled forward into `fforge-game`. **Next is Phase 5, the
agent layer**, and its first deliverable is the design note `AGENT_MODEL.md`.

`docs/BACKLOG.md` is the live status: what is done, what is next, the open calibration questions
and the filed doc-vs-code divergences. Read §3 before any Phase 5 work and §7 before picking up
anything. Do not restate status here; update the backlog.

## Design documents

Every subsystem has a design note in `docs/`, and the code is a transcription of it. Read the
note before changing the subsystem, and amend the note *first* when a change alters the
subsystem's shape.

| Note | Covers | Code |
|---|---|---|
| `DESIGN.md` | vision, five-layer architecture (§3), phases (§9) | whole workspace |
| `ATTRIBUTE_SCHEMA.md` | attributes, rating scale, role weights, CA/PA, Character | `fforge-domain` |
| `MATCH_MODEL.md` | match engine (§1–§10); Phase 2e: condition, injuries, cards, substitutions, ratings (§11–§18) | `match_engine`, `condition` |
| `TACTICS_MODEL.md` | the four instructions, their effects, the AI tactics policy | `match_engine::tactics` |
| `DEVELOPMENT_MODEL.md` | age envelope, growth law, envelope-consistent seeding (§8) | `development`, `worldgen`, `career_arc` |
| `TRANSFER_MODEL.md` | valuation, club AI, clearing loop, finances, player pool | `valuation`, `club_ai`, `market`, `finance`, `pool` |
| `UI_TOOLKIT_EVIDENCE.md` | what the CLI screens taught | `fforge-game` |
| `BACKLOG.md` | status, next work, open questions | — |
| `IMPLEMENTATION_LOG.md` | task-by-task history: why things are the way they are | — |

**When code and a design note disagree, the note is authoritative and the disagreement is a bug
to file** (`BACKLOG.md` §5). The exception is a note that is itself wrong; `DEVELOPMENT_MODEL.md`
§8.2 records the tie-break: resolve toward whichever side a real, identified downstream consumer
depends on.

## Hard constraints

1. **Derive, don't store.** CA exists only as `current_ability()`. Match condition, suspensions
   and availability are likewise views over recorded facts (`GameState::condition`,
   `is_suspended`, `available`). Never add a field that caches something derivable; a cached
   value is a value that can disagree with its source.
2. **`BTreeMap`/`BTreeSet` only, never `HashMap`/`HashSet`**, for anything that can reach game
   state. Determinism is load-bearing: reproducible bugs, replayable matches, calibration.
3. **Purity by layer.** `fforge-domain`: no I/O, no RNG, no clock. `fforge-core`: no I/O in the
   fold, no clock, and randomness only from `rng::derive_stream(seed, tag)`. `fforge-game` is the
   only crate that touches stdin, stdout and the wall clock.
4. **One encoding per rule.** If two call sites need the same rule, they call the same function.
   A second copy is a copy free to drift, and here drift means a replay that no longer matches.
5. **Old logs must keep loading.** A save is the event log. New fields on an `Event` or on a
   domain type that is serialized into one take `#[serde(default)]`.
6. **`#![deny(unsafe_code)]`** in every crate.

## Calibration discipline

The engines carry fitted knob tables (`Knobs`, `DevKnobs`, `ValueKnobs`, `FinanceKnobs`,
`PoolKnobs`, `UtilityKnobs`) and three harness binaries that measure them.

- **A harness reports; a human re-fits.** Never feed a harness reading back into a knob
  automatically, and never move a knob to make a guard pass without recording the reading in the
  design note.
- **Pool over seeds and report per-seed spread.** One synthetic league is noise. For
  squad-conditional effects pool across worlds as well, not only match seeds.
- **Write the prediction down before the change**, so the measurement can refute it.
- **A re-roll is not a re-fit.** Any change to `worldgen` or to RNG draw order re-rolls every
  world, so the golden baselines and `fforge-game` snapshots must be re-pinned. That is expected
  and says nothing about the engine.
- **Tactics have two lever classes** (`TACTICS_MODEL.md` §3): advance-class multipliers on raw
  transition probabilities move roughly four times the absolute probability that logit-class
  biases do. Two calibration passes failed by tuning the weak lever. Read §3 before touching an
  effect magnitude.

## Commands

```
cargo test                                             # fast suite
cargo test --workspace --features fforge-core/slow-tests  # plus the pooled calibration guards (minutes)
cargo clippy --all-targets -- -D warnings              # must be clean
cargo fmt --check                                      # must be clean
cargo run --bin fforge-game                            # play
cargo run --release --bin calibrate -- --seeds 8       # match harness
cargo run --release --bin career_arc                   # development harness
cargo run --release --bin market                       # transfer-market harness
```

Run the slow suite before finishing any change to a knob table, a simulation module, `worldgen`,
`rng`, `state`, `commands`, or anything in `fforge-domain/src`. CI runs it on those paths and
nightly.

## Testing

Plain `#[test]` functions in `#[cfg(test)]` blocks inside each module; no test framework. Assert
**invariants**, not a round-trip of the implementation. `fforge-domain/src/ability.rs` is the
model: uniform input gives uniform CA, weighting is position-relative, bounds hold at the
extremes. A test that would need its expected value recomputed whenever the code changes is
testing nothing.
