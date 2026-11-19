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
   an empty `conditions` map, and for everything that reads players, `ScoutKnobs::omniscient()`) under which `play_match` reproduces the pinned Phase-2a baseline
   bit for bit (`match_engine::golden`). A new feature adds its identity to those tests. Note
   that `ai_pick_lineup` fills a real bench and plan, so a test wanting the identity clears them.
   The baseline is pinned at `Knobs::phase_2a()`, not `Knobs::default()`: it guards wiring, so a
   re-calibration does not move it and a moved baseline is always a bug.
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
| `scouting` | fog-of-war (`AGENT_MODEL.md` §5): `ScoutKnobs`, `Scout::sight` (one club's view of one player), `seen_world`/`seen_squad`, `Range`. Pure and derived; nothing is stored |
| `worldgen` | `generate`; `gen_player` draws PA first and seeds attributes on the age envelope beneath it through `SeedTables`, which calls `development`'s own ceiling machinery (`DEVELOPMENT_MODEL.md` §8) |
| `match_engine` | `play_match`, `MatchRngs`, `MatchOutcome`; AI selection (`ai_pick_lineup`, `ai_pick_lineup_available`, `ai_pick_lineup_vs`, `ai_pick_tactics`, `ai_pick_bench_and_plan`). Submodules: `zone`, `knobs`, `contest`, `tactics`, `resolve` (the possession loop; `Kickoff`, `Team`, `Pitch`, `Attack` and `Ledger` are its context structs — extend those rather than adding parameters), `ratings`, `stream`, `golden` (test-only baseline) |
| `condition` | the pure condition/recovery law; `GameState::condition` is its only caller |
| `development` | `DevKnobs`, the age envelope, `tick_changes`, `attr_rate`, `phys_lmax`, `apply_attr_step` |
| `valuation` | `value`/`value_all`, `project_ca*`, `ValueKnobs`, `MarketContext` — prices, never decides |
| `club_ai` | the `ClubPolicy` trait (`ClubObservation` in, `TransferDecision`s out), `UtilityPolicy` for AI clubs, `RecordedPolicy` replaying the human's pre-committed plan |
| `market` | `resolve_window`: the deferred-acceptance clearing loop; `WindowView`, the window's frozen picture of what every club believes and asks; window dates |
| `finance` | `finance_deltas`, `FinanceKnobs` |
| `pool` | youth intake and retirement at the summer window, `PoolKnobs` |

`lib.rs` re-exports the public surface; `fforge-game` imports from the crate root.

## Things that will bite

- **The match engine has no `GameState`.** Whatever it needs from the season (condition, the
  suspended set, today's date) is computed by the caller and passed in. Keep it that way; it is
  why the engine can be driven by a harness with no season at all.
- **`bin/calibrate` therefore runs at the condition identity.** It cannot see fatigue-driven
  effects such as substitutions. Reading those needs a real-season harness (`BACKLOG.md` §4.3).
- **`Knobs::s` is the scale of every contest**, and so of every effect that works by scaling
  attributes: fatigue, condition, Consistency, injury impairment. It also sets how unequal a
  league is. Moving it is a five-knob re-fit, not a tweak (`MATCH_MODEL.md` §8.1).
- **`UtilityKnobs::asking_markup` must stay `<= 1.0`.** Every club prices off the same
  `value()`, so an ask above value makes the surplus negative for every buyer and nothing clears.
- **`opp_mid_advance_mult` and `opp_penetrate_mult` each have two tenants**, Pressing and
  Mentality, and they stack. Changing either instruction moves both.
- **`UtilityPolicy` buys only against a named need** (`need_filled`: cover, depth, upgrade,
  heir), and a squad at or over the template's headcount has no depth need. `need()` alone is
  positive almost everywhere and filters nothing. A new buying rule goes through `need_filled`,
  so `fforge-harness` can still say why every transfer happened.
- **A player moves at most once per window**, enforced in `market::resolve_window`, not in any
  policy (`TRANSFER_MODEL.md` §5 step 4). Policies are memoryless across clearing rounds; before
  this rule a signing was re-listed the round after he arrived and passed down the league.
- **Affordability is checked at resolve time** (`market::filter_affordable`), not at submit
  time. `validate_transfer_decisions` only checks shape, because a human plan is replayed against
  a world that has moved on since it was submitted.
- **Every decider is fogged** (`AGENT_MODEL.md` §5). A policy never reads another club's
  player from `World` directly: it reads him through `scouting` (`GameState::squad_as_seen_by`,
  `market::WindowView::observe`). Reading the true world in a new policy silently makes that
  policy omniscient. Truth is for the fold, the engines and the harness's `info`.
- **A club appraises only players within its means.** `Candidate.value` is `Money(0)` for one
  it could not pay for (`club_ai::observe_as`); valuing a player is the market's hot path and
  under fog every club has to do its own. `ClubObservation::within_means` is the one test.
- **Any club can submit a team sheet or a transfer plan; what happens when one doesn't depends
  on the seat** (`AGENT_MODEL.md` §3.2). The human's club reuses its last sheet and makes no
  transfers; every other club falls back to the built-in policy. `commands::effective_lineup`
  and `commands::recorded_transfer_plans` are the only two places that rule is written. The core
  never records who is agent-controlled.
- **A suspension is one matchday wide by construction.** `is_suspended` compares against
  `current_matchday - 1` exactly, so there is no "ban served" flag to maintain.
- **The harnesses live in `fforge-harness`, not here.** This crate prints nothing and must not
  depend on that one. A few items are `pub` only because a harness reads them
  (`match_engine::mirrored_zone`, `development::EnvTables`/`norms_by_role`/
  `tick_changes_with_clip_stats`); they are measurement surface, not API to build on.
- **The pooled guards are `#[ignore]` without the `slow-tests` features** (one here for the
  whole-season sanity bands, one in `fforge-harness` for the calibration guards). A green fast
  suite says nothing about calibration.

History, measurements and the reasoning behind individual knob values are in the design notes
and `docs/IMPLEMENTATION_LOG.md`, not here.
