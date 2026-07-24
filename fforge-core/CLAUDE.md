# fforge-core

Layer 2 of the fforge workspace: the deterministic simulation core, consuming
`fforge-domain`. The crate is a pure fold over an append-only event log — `GameState`
*is* the fold's accumulator, `Session` glues log + state + observers together, and
`commands::step` is the only place proposals turn into recorded events.

## Current phase

Phase 1 (full season loop, league table) is complete. `match_engine` runs the Phase 2a
event-based possession engine (`MATCH_MODEL.md`), replacing the old crude Poisson engine
behind the same `play_match` call site, calibrated and guarded by
`match_engine::calibrate`/`bin/calibrate`.

Phase 3 player development (`DEVELOPMENT_MODEL.md`) is implemented in the `development`
module — a monthly `Event::DevelopmentTick` records resolved attribute deltas the fold
integer-adds, and `Command::StartNextSeason` rolls the developed world into a fresh
season — calibrated and guarded by the `career_arc` harness/`bin/career_arc`.

Phase 4 (`TRANSFER_MODEL.md`) is complete end to end. The event-log seam (§4) adds six
events (`TransferCompleted`, `PlayerReleased`, `ContractRenewed`, `YouthIntake`,
`PlayerRetired`, `FinanceTick`) and their `state::apply` fold arms. The Layer-3 club
decision AI (§6, §6.1) is implemented in `club_ai` — a `ClubPolicy` trait and its v1
`UtilityPolicy` implementation, producing `TransferDecision`s from a `ClubObservation`.
`market::resolve_window` runs §5's simultaneous, deferred-acceptance clearing loop over
`club_ai`'s decisions and folds winning bids into `Event::TransferCompleted`;
`commands::advance_matchday` fires it on the §7 window boundaries (summer/winter), the
same tick mechanism development and finance use — no new command. The player pool
closes at both ends (§8) via the `pool` module: annual youth intake and age/CA-driven
retirement, both firing at the summer window alongside the market. §10's human-decision
seam is also promoted, not deferred: `Command::SubmitTransferDecision` → two more events
(`TransferDecisionSubmitted`, `TransferWindowClosed`) and `club_ai::RecordedPolicy` — a
second `ClubPolicy` that replays a pre-committed plan verbatim every round, the human
club's substitute for `UtilityPolicy`. Deferred beyond v1: loans, negotiation rounds,
transfer clauses. The Phase-4 pathology harness (§11) is implemented in
`market::calibrate` (`MarketTelemetry`/`MarketReport`, `bin/market.rs`,
`market_is_in_a_believable_ballpark`) — the transfer-market sibling of
`match_engine::calibrate` and `career_arc`. It drove the re-fit of `ValueKnobs::beta`
(ln2/6 → ln2/8) and `FinanceKnobs::revenue_per_reputation` (150k → 500k) recorded in
`TRANSFER_MODEL.md` §9; the harness caught the market at those starting values dead
(universal insolvency, ~0.2 transfers/club/window). It also needed a small compensating
fix once §10 landed: `player_club` (the harness has no real human) now submits its own
`UtilityPolicy`-equivalent plan each window so it keeps behaving like every other AI
club, not a silently-passive one (`calibrate::submit_player_clubs_ai_equivalent_plan`).

R2's `news` module is implemented: a Trace-side, structured, replay-safe notification
stream (`NewsItem { date, kind: NewsKind, sources: Vec<EventRef>, salience, audience }`)
plus a deterministic `TemplateRenderer`, homed in `fforge-core::news`. `NewsObserver` has
three entry points, none of which widen `EventObserver`: `on_event` (category 1,
event-derived — match results, transfers completed, youth intake, retirements),
`check_conditions(&GameState)` (category 2, state-condition — contracts expiring,
finance warnings, role-coverage gaps — the same "sees state, not events" seam
`market::calibrate::MarketTelemetry` established for `record_season_end`), and
`observe_rejected_bids` (a third, narrower path for `WindowOutcome`'s Trace, which is
never an `Event` and never a `GameState` fact — sourced live, the same way
`player_match_preview` re-derives `MatchOutcome`'s commentary rather than persisting it,
so a cold replay never re-populates that one slice of the inbox). Wiring this into the
live game loop (`commands.rs` calling `check_conditions` after every command,
`fforge-game` rendering the inbox) is explicitly out of scope here — that is
B2.5/Batch 4's job; this task is the module and its own test suite only.

Phase 2e has begun with `MATCH_MODEL.md` §11's sequencing step 1 — the §12 boundary
extension, plumbing only. `MatchOutcome` and `Event::MatchPlayed` now carry resolved
per-player consequences for all three consumers at once (`injuries: Vec<InjuryOutcome>`
— the days out, never a severity to re-roll; `cards: Vec<CardOutcome>` — the card
itself, never a foul to re-resolve; `ratings` in tenths), all `#[serde(default)]` (and
skipped when empty) so pre-2e logs load and empty saves keep the pre-2e byte shape. The
fold consumes them: injuries → `Player.injured_until` (the first sanctioned 2e domain
field; never shortened by a later shorter layoff), cards → `GameState::season_cards`
(recorded truth only — a suspension is *derived* from cards, never stored and never its
own event, per §12's derived-suspension rule), ratings → `GameState::recent_ratings` (a
`RATING_FORM_WINDOW`-capped form window). `GameState::recent_appearances` is the §13
rolling appearance window (pruned to `CONDITION_WINDOW_DAYS` as the date advances) —
distinct from `appearances_since_tick`, which stays monthly and tick-reset. The engine
emits all three vectors empty (`boundary_consequences_stay_empty_until_the_2e_models_land`
pins this), so no RNG draw and no calibration reading moved; the §14/§15/§18 models that
fill them are still design-gated.

`fforge-core` is the active development front.

## Module map

| Module | Owns |
|---|---|
| `event` | `Event` enum — the append-only log's payload types, including the Phase-4 transfer/contract/finance/pool events (`TRANSFER_MODEL.md` §4), §10's `TransferDecisionSubmitted`/`TransferWindowClosed`, and `MatchPlayed`'s Phase-2e boundary fields (`MATCH_MODEL.md` §12: `injuries`/`cards`/`ratings`, serde-defaulted for pre-2e logs) |
| `market` | Phase-4 clearing loop and window mechanics (`TRANSFER_MODEL.md` §5, §7): `resolve_window` — freeze the valuation cache once, then simultaneous rounds of `club_ai`-decided bids/listings (`human_club: Option<ClubId>` substitutes `club_ai::RecordedPolicy` for that one club, §10's pre-commitment seam; every other club runs `UtilityPolicy`), contention resolved by the selling club's ranking (fee, buyer reputation, `ClubId`) then player consent (`MarketKnobs`'s wage/reputation-threshold roll), refused pairs never re-proposed (classic deferred acceptance — the actual convergence mechanism; `MAX_ROUNDS = 12` is the adversarial-input cap, not the normal exit). `filter_affordable` applies the same resolve-time affordability/squad-bounds/GK-floor/availability gate to every club's decisions regardless of which policy produced them — a no-op for `UtilityPolicy` (already compliant by construction), the actual gate for a `RecordedPolicy` plan that bypasses that producer-side filtering; it also re-validates a `Bid`'s claimed seller against the round's live observation, closing a staleness gap a static replay can hit that `UtilityPolicy`'s always-fresh decisions never could. Returns `WindowOutcome { transfers, rejected_bids, valuations, unfilled_needs, rounds_used }` — only `transfers` folds into `Event::TransferCompleted`; the rest is a Trace, exactly `MatchOutcome.stream`'s shape (`MATCH_MODEL.md` §7). `summer_window_close`/`winter_window_close` derive window boundaries from the season (never day-of-year constants); `commands::transfer_window_events` fires resolution when `advance_matchday` crosses one, using `TRANSFER_STREAM_NS \| window_index` as its RNG stream. Its `calibrate` submodule (re-exported at `market::{MarketTelemetry, MarketReport, run_market_calibration, print_report}`) is the §11 pathology harness: since `WindowOutcome`'s rich Trace never survives the fold, `MarketTelemetry` reads competitive-balance/financial-health metrics off the folded `World` at each season boundary (`record_season_end`, via `state::league_table` and `valuation::value_all`) while consuming `TransferCompleted`/`YouthIntake`/etc. as an `EventObserver` for fee/volume data — pooled over many seeds × ~15 seasons, exactly the multi-seed-pooling discipline `career_arc` and `match_engine::calibrate` established. `submit_player_clubs_ai_equivalent_plan` keeps the harness's own `player_club` behaving like every other AI club post-§10 (see "Current phase" above). Harness plumbing only; never feeds back into `ValueKnobs`/`FinanceKnobs` by itself |
| `club_ai` | Phase-4 Layer-3 club decision AI (`TRANSFER_MODEL.md` §6, §6.1): the `ClubPolicy` trait (`ClubObservation` in, `Vec<TransferDecision>` out — the Gym-shaped seam `ai_pick_lineup`'s doc comment anticipated), `UtilityPolicy` (`need(club, role)` = depth + quality-vs-own-reputation-target + succession risk from `valuation::project_ca_batch`; buy shortlists ranked by `need · (value − asking_price)`, with a role-coverage **override** ahead of that ranking (`TRANSFER_MODEL.md` §11's hard stabilizer): a candidate in a role currently below its §6 hard minimum (`hard_minimum_violations` — today just `Gk` below `min_goalkeepers`) ranks first regardless of `need · surplus` elsewhere, exempt from the positive-surplus filter too, so no ordinarily-attractive opportunity can outbid it; the cash/wage/squad-ceiling stabilizers still gate it, so a club with no headroom must sell first; sell lists from §6's first two triggers plus a third, squad-size pressure term — `UtilityKnobs::squad_pressure_start`/`_exponent`/`_max_listings` — that makes at-template (not yet genuinely surplus) roles listable through a bounded, continuously-growing quota as the squad approaches `squad_max`, addressing the §9 "squads pin at the ceiling" residual; GK is excluded from this term since its template sits only one above `min_goalkeepers`), and `observe()` (builds a `ClubObservation` off `World` + the `value_all` cache — the only place in this module that reads `World`). `RecordedPolicy` (§10) is the second `ClubPolicy`: replays a pre-committed `Vec<TransferDecision>` verbatim on every call, ignoring `ClubObservation` entirely — never adapting is the point, and an empty plan yields no decisions rather than falling back to `UtilityPolicy`. Squad bounds `[18, 30]`, `≥2` GK, cash and wage headroom are hard stabilizers, not utility terms — the pressure term never touches them. **`UtilityKnobs::asking_markup` must stay `<= 1.0`**: with every club pricing off the same omniscient `value()` (§2.6 — no private valuations in v1), an ask *above* value makes `need · (value − asking_price)` negative for every buyer regardless of need, so no trade can ever clear — filed as a corrected divergence from §12 item 6's literal "markup" phrasing, caught by `club_ai::tests::real_observed_candidates_can_actually_produce_a_bid`. Decisions only — the clearing loop lives in `market` |
| `state` | `GameState` — pure fold (`apply`/`replay`), `TableRow`, `league_table()`. The Phase-4 fold arms (six from §4, two more from §10's `TransferDecisionSubmitted`/`TransferWindowClosed`) are pure integer/assignment operations only (no RNG, no math beyond addition, no engine calls) and keep club rosters sorted after mutation, so replay-path equality holds. `pending_transfer_decisions: Vec<TransferDecision>` holds the current pre-commitment (§10) — set on submission, cleared on window close, good for exactly one window. `apply_transfer_completed`/`apply_finance_deltas` are `pub(crate)` free functions so `market`/`commands` can apply the identical mutation to a working `World` without a second encoding. The Phase-2e `MatchPlayed` arm (`MATCH_MODEL.md` §12) additionally folds `injuries` → `Player.injured_until`, `cards` → `season_cards` (cleared on `SeasonStarted`; bans are derived, never stored), `ratings` → `recent_ratings` (capped at `RATING_FORM_WINDOW`), and every XI appearance → `recent_appearances`, the §13 rolling window pruned to `CONDITION_WINDOW_DAYS` wherever the fold moves the date |
| `commands` | `Command` enum, `step()` — validates a proposal and produces the events for it; `player_match_preview()` — a pure query, re-deriving the same lineup selection and RNG stream `advance_matchday` is about to use, for live-viewing the human's own fixture before it's recorded. `Command::SubmitTransferDecision` (§10) runs `validate_transfer_decisions` (submit-time shape only: targets exist, aren't already owned, prices aren't negative, sell targets are the club's own) before recording `Event::TransferDecisionSubmitted` — affordability is resolve-time, inside `market::filter_affordable`, not here. `dev_ticks_between` returns its compounded working `World` alongside the events, so `transfer_window_events` (fired from `advance_matchday` on a §7 boundary crossing) resolves against this advance's developed attributes and finance deltas, not the pre-tick world, passes `Some(state.player_club)`/`state.pending_transfer_decisions` through to `resolve_window`, and emits `Event::TransferWindowClosed` for every crossed boundary regardless of outcome so a pre-committed plan expires on schedule; `season_start_date` derives the season's kickoff from `state.date`/`current_matchday` rather than storing it |
| `session` | `Session` — owns the log + folded state, routes commands, notifies observers; `save_log`/`load_log` (JSON-lines) |
| `observer` | `EventObserver` trait, `SeasonTelemetry` — passive event-stream consumers (trace/telemetry spine) |
| `news` | The R2 notification Trace: `NewsItem`/`NewsKind`/`EventRef`/`Audience`, `NewsRenderer` + `TemplateRenderer`, and `NewsObserver` (`EventObserver` for event-derived news; `check_conditions(&GameState)` for state-condition news; `observe_rejected_bids` for `WindowOutcome`'s Trace). Maintains small incremental indices (fixture→clubs, squad membership, each player's/club's most recent contract/finance/squad-affecting `EventRef`) purely from events already seen, so `check_conditions` — which only ever sees `&GameState`, never the log — can still attach real provenance to a state-condition item. `warned_*` sets make every state-condition check edge-triggered (fires once per newly-true condition, re-arms on recovery) so a season-long inbox stays bounded rather than repeating the same fact every call. Not wired into `commands.rs`/`session.rs`/`fforge-game` yet — a self-contained module + test suite, by explicit scope fence |
| `match_engine` | Phase-2a engine: `play_match` (`MatchOutcome { home_goals, away_goals, stream }` plus the §12 boundary fields `injuries`/`cards`/`ratings` — emitted empty until the 2e models land), `lineup_strength`, `ai_pick_lineup`. Submodules: `zone` (five-zone state + role→zone presence table), `knobs` (the fitted `Knobs` table), `contest` (attribute→contest maps, the logistic resolver, fatigue), `resolve` (the possession loop), `stream` (`MatchEvent` schema + commentary rendering) |
| `development` | Phase-3 growth engine (`DEVELOPMENT_MODEL.md` §2–§5): the `DevKnobs` table (sibling of `match_engine::Knobs`), the per-category age envelope, PA-scaled targets, `resolve_dev_profile`/`resolve_coaching` (worldgen edge), and `tick_changes` — the growth math producing a `DevelopmentTick`'s resolved deltas. The per-attribute rate law is factored into `attr_rate`, shared verbatim with `valuation`'s projection so there is one law (no second integrator to drift). All RNG/math lives here; `apply` only integer-adds via `apply_attr_step` |
| `valuation` | Phase-4 centralized value function (`TRANSFER_MODEL.md` §2): `value` / `value_all` (the §2.7 per-window `BTreeMap<PlayerId, Money>` cache), `project_ca` (runs `development::attr_rate` forward, jitter off, minutes/coaching neutral), `project_ca_batch` (many players, one shared knob-derived `DevTables` — `club_ai::observe`'s per-squad projection), the `ValueKnobs` §9 table (plausibility-picked, sibling of `DevKnobs`), and `MarketContext` (bounded league-wide role scarcity). `value_with` integrates each player's whole 0..=horizon_years trajectory in one pass (`project_ca_series`) rather than once per year — same numbers, no redundant re-integration of the shared prefix. A pure Layer-2 function — prices, never decides; no market/club-AI here (Phase 4 §5–§6) |
| `career_arc` | Phase-3 career-arc harness (`DEVELOPMENT_MODEL.md` §6): the development sibling of `match_engine::calibrate`. Drives the real worldgen + development-fold pipeline over many seeds × a decade-plus and reports the §6 metrics (peak ages, PA attainment + tail, veteran decline slopes, wonderkid hit/flop) with per-seed spread. `bin/career_arc` is the runner; `career_arcs_are_in_a_believable_ballpark` is the wide-band regression guard. Harness plumbing, never fed back into `DevKnobs` by itself — the re-fit is a human reading the numbers |
| `finance` | Phase-4 finance tick (`TRANSFER_MODEL.md` §4): `finance_deltas` resolves monthly revenue (∝ `Club.reputation`) minus the monthly share of committed wages into per-club deltas; `FinanceKnobs` (plausibility-picked, sibling of `DevKnobs`/`ValueKnobs`). RNG-free — both inputs are already-resolved world state, unlike `tick_changes`'s jitter. `commands::dev_ticks_between` calls it on the same 30-day boundary crossing `DevelopmentTick` fires on, emitting `Event::FinanceTick` alongside it |
| `pool` | Phase-4 player-pool lifecycle (`TRANSFER_MODEL.md` §8): `summer_pool_events` — one `YouthIntake` per club with roster headroom (reusing `worldgen::gen_player` with a 16-18 age band, quality anchored on `reputation` × `coaching_milli`), then every qualifying `PlayerRetired` (age ≥ `min_retirement_age` and best-role CA below `relevance_floor`, or a full season unsigned via `GameState::unsigned_since`). `PoolKnobs` (plausibility-picked, sibling of the others — but re-tuned against a real 15-season run, not left at a naive guess: the aging law lets CA plateau rather than crash, so a too-low floor leaves veterans immortal, squads permanently full, and mean age climbing unchecked). Intake is capped to `squad_max` headroom so it can never walk a club through the market's own hard squad-bound stabilizer. `commands::transfer_window_events` calls it only on the summer (even) window index, before `market::resolve_window`, so new prospects are on the books and retirees are already excluded from valuation when the clearing loop runs |
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