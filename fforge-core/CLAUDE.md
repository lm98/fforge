# fforge-core

Layer 2: the deterministic simulation core. The crate is a **pure fold over an append-only event
log**. `GameState` is the fold's accumulator, `commands::step` is the only place a proposal
becomes recorded events, and `Session` glues log, state and observers together. Saving is
serializing the log; loading is replaying it.

```
Command ──step()──▶ Vec<Event> ──apply()──▶ GameState
         impure:                 pure:
         RNG, engines,           assignment and
         validation              integer adds only
```

## Invariants

1. **`GameState::apply` is a total, pure fold.** No RNG, no I/O, no clock, no engine calls, no
   math beyond integer addition. Anything random or expensive is resolved in `commands::step` and
   recorded in the event as its *result* (an injury is days out, never a severity to re-roll; a
   development tick is attribute deltas, never a rate to re-integrate). This is what lets a model
   change without rewriting recorded history.
2. **All randomness is seed-derived.** Every `Rng` comes from `rng::derive_stream(seed, tag)`
   with a namespace constant (`*_NS`) OR'd with a local index. Never share an `Rng` across
   features, never seed from entropy or the clock.
3. **One stream per feature.** A match draws from four (`MatchRngs`: main, consistency, injury,
   foul). A feature sharing another's stream would shift every later draw when it is switched
   off, and the identity tests could not be written. A new stochastic feature gets its own
   namespace and its own stream, and draws in a fixed order whether or not the draw is used.
4. **Every feature layered on the match engine has an identity setting** (`Tactics::neutral()`,
   empty `bench`/`sub_plan`, `consistency_sigma_max: 0.0`, `injury_rate: 0.0`, `foul_rate: 0.0`,
   an empty `conditions` map) under which `play_match` reproduces the pinned Phase-2a baseline
   bit for bit (`match_engine::golden`). A new feature adds its identity to those tests. Note
   that `ai_pick_lineup` fills a real bench and plan, so a test wanting the identity clears them.
5. **Events versus Traces.** An `Event` is folded and persisted. A Trace is derived, rich and
   thrown away: `MatchOutcome.stream`, everything in `WindowOutcome` except `transfers`, and
   `NewsItem`s. Never fold a Trace, never persist one. Consumers that need one re-derive it from
   the same inputs (`player_match_preview`).
6. **Mutations shared between the fold and a working `World` have one encoding.** `step`
   sometimes has to advance a scratch `World` within a single command (development ticks, then
   finance, then the transfer window). It does so by calling the same `state::apply_*` function
   the fold arm calls. Likewise `commands::simulate_fixture` is the only way a recorded fixture
   is played, and `development::attr_rate`/`phys_lmax` are the only growth and aging laws —
   `valuation` projects with them rather than re-integrating.

## Module map

| Module | Owns |
|---|---|
| `event` | `Event` and every type it serializes that is not a domain type (`AttrStep`, `InjuryOutcome`, `Card`, `CardOutcome`, `TransferDecision`). The log's schema depends on no engine; engines re-export these under their own paths |
| `state` | `GameState`, `apply`/`replay`, `league_table`, the `pub(crate) apply_*` mutations, and the derived views `condition`, `available`, `is_suspended`, `suspended_players`, `lineup_conditions` |
| `commands` | `Command`, `step` and its validation; `advance_matchday` (matches, then monthly development and finance ticks, then any transfer window the date crossed); `player_match_preview` |
| `session` | `Session`, `save_log`/`load_log` (JSON lines) |
| `observer` | `EventObserver`, `SeasonTelemetry` — passive consumers that never write to the world |
| `news` | `NewsObserver`: event-derived items, state-condition items (`check_conditions`, edge-triggered) and rejected-bid items, each carrying `sources: Vec<EventRef>` provenance; `TemplateRenderer` |
| `rng` | xoshiro256** and `derive_stream` — the crate's only randomness |
| `schedule` | `double_round_robin` |
| `worldgen` | `generate`; `gen_player` draws PA first and seeds attributes on the age envelope beneath it through `SeedTables`, which calls `development`'s own ceiling machinery (`DEVELOPMENT_MODEL.md` §8) |
| `match_engine` | `play_match`, `MatchRngs`, `MatchOutcome`; AI selection (`ai_pick_lineup`, `ai_pick_lineup_available`, `ai_pick_lineup_vs`, `ai_pick_tactics`, `ai_pick_bench_and_plan`). Submodules: `zone`, `knobs`, `contest`, `tactics`, `resolve` (the possession loop), `ratings`, `stream`, `golden` (test-only baseline), `calibrate` (harness) |
| `condition` | the pure condition/recovery law; `GameState::condition` is its only caller |
| `development` | `DevKnobs`, the age envelope, `tick_changes`, `attr_rate`, `phys_lmax`, `apply_attr_step` |
| `career_arc` | development harness: career metrics, and the PA-recoverability fits Phase 5's fog-of-war depends on |
| `valuation` | `value`/`value_all`, `project_ca*`, `ValueKnobs`, `MarketContext` — prices, never decides |
| `club_ai` | the `ClubPolicy` trait (`ClubObservation` in, `TransferDecision`s out), `UtilityPolicy` for AI clubs, `RecordedPolicy` replaying the human's pre-committed plan |
| `market` | `resolve_window`: the deferred-acceptance clearing loop; window dates; `calibrate` (harness) |
| `finance` | `finance_deltas`, `FinanceKnobs` |
| `pool` | youth intake and retirement at the summer window, `PoolKnobs` |

`lib.rs` re-exports the public surface; `fforge-game` imports from the crate root.

## Things that will bite

- **The match engine has no `GameState`.** Whatever it needs from the season (condition, the
  suspended set, today's date) is computed by the caller and passed in. Keep it that way; it is
  why the engine can be driven by a harness with no season at all.
- **`bin/calibrate` therefore runs at the condition identity.** It cannot see fatigue-driven
  effects such as substitutions. Reading those needs a real-season harness (`BACKLOG.md` §4.3).
- **`UtilityKnobs::asking_markup` must stay `<= 1.0`.** Every club prices off the same
  `value()`, so an ask above value makes the surplus negative for every buyer and nothing clears.
- **`opp_mid_advance_mult` and `opp_penetrate_mult` each have two tenants**, Pressing and
  Mentality, and they stack. Changing either instruction moves both.
- **Affordability is checked at resolve time** (`market::filter_affordable`), not at submit
  time. `validate_transfer_decisions` only checks shape, because a human plan is replayed against
  a world that has moved on since it was submitted.
- **A suspension is one matchday wide by construction.** `is_suspended` compares against
  `current_matchday - 1` exactly, so there is no "ban served" flag to maintain.
- **The harness modules print.** `career_arc`, `market::calibrate` and `match_engine::calibrate`
  are reporting code that lives in the library so the guards can call them; nothing in the fold
  or in `step` may depend on them.
- **The pooled guards are `#[ignore]` without `--features slow-tests`.** A green fast suite says
  nothing about calibration.

History, measurements and the reasoning behind individual knob values are in the design notes
and `docs/IMPLEMENTATION_LOG.md`, not here.
