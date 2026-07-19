# fforge-core

Layer 2 of the fforge workspace: the deterministic simulation core, consuming
`fforge-domain`. The crate is a pure fold over an append-only event log — `GameState`
*is* the fold's accumulator, `Session` glues log + state + observers together, and
`commands::step` is the only place proposals turn into recorded events. Phase 1 (full
season loop, league table) is complete; `match_engine` now runs the Phase 2a
event-based possession engine (`MATCH_MODEL.md`), replacing the old crude Poisson
engine behind the same `play_match` call site. Phase 3 player development
(`DEVELOPMENT_MODEL.md`) is implemented in the `development` module — a monthly
`Event::DevelopmentTick` records resolved attribute deltas the fold integer-adds,
and `Command::StartNextSeason` rolls the developed world into a fresh season. Phase 4's
event-log seam (`TRANSFER_MODEL.md` §4) is implemented — six events
(`TransferCompleted`, `PlayerReleased`, `ContractRenewed`, `YouthIntake`,
`PlayerRetired`, `FinanceTick`) and their `state::apply` fold arms — but only the
seam: no decision logic, clearing loop, or valuation call site produces them yet.

## Module map

| Module | Owns |
|---|---|
| `event` | `Event` enum — the append-only log's payload types, including the Phase-4 transfer/contract/finance/pool events (`TRANSFER_MODEL.md` §4) |
| `state` | `GameState` — pure fold (`apply`/`replay`), `TableRow`, `league_table()`. The six Phase-4 fold arms are pure integer operations only (no RNG, no math beyond addition, no engine calls) and keep club rosters sorted after mutation, so replay-path equality holds |
| `commands` | `Command` enum, `step()` — validates a proposal and produces the events for it; `player_match_preview()` — a pure query, re-deriving the same lineup selection and RNG stream `advance_matchday` is about to use, for live-viewing the human's own fixture before it's recorded |
| `session` | `Session` — owns the log + folded state, routes commands, notifies observers; `save_log`/`load_log` (JSON-lines) |
| `observer` | `EventObserver` trait, `SeasonTelemetry` — passive event-stream consumers (trace/telemetry spine) |
| `match_engine` | Phase-2a engine: `play_match` (`MatchOutcome { home_goals, away_goals, stream }`), `lineup_strength`, `ai_pick_lineup`. Submodules: `zone` (five-zone state + role→zone presence table), `knobs` (the fitted `Knobs` table), `contest` (attribute→contest maps, the logistic resolver, fatigue), `resolve` (the possession loop), `stream` (`MatchEvent` schema + commentary rendering) |
| `development` | Phase-3 growth engine (`DEVELOPMENT_MODEL.md` §2–§5): the `DevKnobs` table (sibling of `match_engine::Knobs`), the per-category age envelope, PA-scaled targets, `resolve_dev_profile`/`resolve_coaching` (worldgen edge), and `tick_changes` — the growth math producing a `DevelopmentTick`'s resolved deltas. The per-attribute rate law is factored into `attr_rate`, shared verbatim with `valuation`'s projection so there is one law (no second integrator to drift). All RNG/math lives here; `apply` only integer-adds via `apply_attr_step` |
| `valuation` | Phase-4 centralized value function (`TRANSFER_MODEL.md` §2): `value` / `value_all` (the §2.7 per-window `BTreeMap<PlayerId, Money>` cache), `project_ca` (runs `development::attr_rate` forward, jitter off, minutes/coaching neutral), the `ValueKnobs` §9 table (plausibility-picked, sibling of `DevKnobs`), and `MarketContext` (bounded league-wide role scarcity). A pure Layer-2 function — prices, never decides; no market/club-AI here (Phase 4 §5–§6) |
| `career_arc` | Phase-3 career-arc harness (`DEVELOPMENT_MODEL.md` §6): the development sibling of `match_engine::calibrate`. Drives the real worldgen + development-fold pipeline over many seeds × a decade-plus and reports the §6 metrics (peak ages, PA attainment + tail, veteran decline slopes, wonderkid hit/flop) with per-seed spread. `bin/career_arc` is the runner; `career_arcs_are_in_a_believable_ballpark` is the wide-band regression guard. Harness plumbing, never fed back into `DevKnobs` by itself — the re-fit is a human reading the numbers |
| `finance` | Phase-4 finance tick (`TRANSFER_MODEL.md` §4): `finance_deltas` resolves monthly revenue (∝ `Club.reputation`) minus the monthly share of committed wages into per-club deltas; `FinanceKnobs` (plausibility-picked, sibling of `DevKnobs`/`ValueKnobs`). RNG-free — both inputs are already-resolved world state, unlike `tick_changes`'s jitter. `commands::dev_ticks_between` calls it on the same 30-day boundary crossing `DevelopmentTick` fires on, emitting `Event::FinanceTick` alongside it |
| `rng` | Seeded xoshiro256** + `derive_stream` — the crate's only source of randomness |
| `schedule` | `double_round_robin()` — deterministic fixture generation |
| `worldgen` | `generate()` — seeded new-game world/schedule/start date, recorded once into `GameStarted` |

`match_engine`'s trace (`MatchOutcome::stream`) is a Trace, not a fold input
(`MATCH_MODEL.md` §7): `commands::advance_matchday` folds only the score into
`Event::MatchPlayed` and discards the stream; nothing here persists it. Live-viewing
consumers reach the trace two ways: `fforge-game`'s friendly viewer calls `play_match`
directly (unrecorded, no `Event` at all), while its main game loop calls
`commands::player_match_preview` on the pre-advance `GameState` to get the human's own
fixture's trace, then executes `Command::AdvanceMatchday` as normal — same inputs, same
RNG derivation, so the previewed trace's score can never disagree with what gets
recorded.

`lib.rs` re-exports the public surface; consumers (`fforge-game`) import from the crate
root.

## Invariants to preserve

1. **All randomness is seed-derived.** Every `Rng` comes from `rng::derive_stream(seed,
   tag)`. Never construct a shared/global `Rng`, never seed from system entropy or wall-
   clock time — that breaks the same-seed-same-season guarantee the test suite checks.
2. **`GameState::apply` (and therefore `replay`) is a total, pure fold.** No RNG, no I/O,
   no wall-clock branching inside it. All impure work — RNG draws, match simulation,
   validation — happens in `commands::step`, which only *produces* `Event`s for `apply`
   to consume.