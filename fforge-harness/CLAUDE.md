# fforge-harness

The calibration and pathology harnesses. Each one drives the real `fforge-core` pipeline over
many seeds and reports emergent statistics against the bands its design note sets. This crate
depends on `fforge-core`; nothing depends on it.

| Module | Measures | Design note | Runner |
|---|---|---|---|
| `match_engine` | `StreamTelemetry` over match traces; head-to-head and squad-conditional tactics probes | `MATCH_MODEL.md` §10, `TACTICS_MODEL.md` §5, §8 | `bin/calibrate` |
| `career_arc` | career arcs (peak ages, PA attainment, veteran decline, wonderkid flops) and the PA-recoverability fits Phase 5's fog-of-war depends on | `DEVELOPMENT_MODEL.md` §6, §8 | `bin/career_arc` |
| `market` | `MarketTelemetry`: competitive balance, financial health, transfer volume | `TRANSFER_MODEL.md` §11 | `bin/market` |

## Rules

- **Report, never fit.** A harness prints readings. Moving a knob is a human decision recorded in
  the design note with the reading that justified it.
- **Drive the real pipeline.** Use `worldgen`, `Session`, `Command` and `play_match` as the game
  does. A harness that re-implements a piece of the simulation measures its own copy.
- **Read the public surface.** If a measurement needs an internal, make that one item `pub` in
  `fforge-core` and say in its doc comment that it is measurement surface. Do not copy the logic
  here.
- **Pool, and report spread.** Every reading is pooled over seeds with the per-seed spread shown;
  squad-conditional readings pool across worlds as well.
- **`bin/calibrate` has no `GameState`**, so it runs at the condition identity and cannot see
  fatigue-driven effects (`BACKLOG.md` §4.3). `career_arc` and `market` run real sessions.
- **The guards are wide bands, `#[ignore]`d without `--features slow-tests`.** They exist to catch
  a knob change that breaks something grossly, not to pin a value. Widening a band to make a guard
  pass needs the same note-first justification as moving a knob.
- Same determinism rules as the core: seed-derived RNG only, `BTreeMap` only, no wall clock.
