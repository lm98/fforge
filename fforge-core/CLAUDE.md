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

The batch-3 handoff has since carried Phase 2e further: `natural_fitness` on `Character`
(T3, domain + worldgen only), true minutes on `MatchPlayed` and minutes-share development
bands (T4), the pinned Phase-2a golden baseline `match_engine::golden` (T5), and tactics
at neutral (T6) — `fforge_domain::Tactics` rides `Lineup.tactics`, resolved once per match
by `match_engine::tactics::{SideEffects, resolve_tactics}` into three deformation types
(probability multipliers, the shared logistic's bias slot, a fatigue-rate multiplier), with
both `TACTICS_MODEL.md` §4 identity tests green and every pooled calibration guard
unchanged. T7 (`ai_pick_tactics`/`AiTacticKnobs`, the `run_head_to_head` triangle harness)
landed but found `TACTICS_MODEL.md` §5's triangle does not close, even after two targeted
zone-profiling fixes (`TACTICS_MODEL.md` §5's finding, §9 item 6) — `ai_pick_lineup_vs`
gates the policy behind `match_engine::AI_TACTICS_ENABLED` (currently `false`), so every
AI-controlled match still runs `Tactics::neutral()` in practice, and T8-T13 can proceed
against T5's stable baseline while §9 item 6 (cyclic vs. squad-conditional non-dominance)
is an open design decision.

T8 (Consistency, `MATCH_MODEL.md` §17) landed: `match_engine::resolve::build_xi` draws one
per-match multiplier per player (`Knobs::consistency_sigma_max`, its own `CONSISTENCY_NS`
RNG stream, identity `0.0`) and applies it to every attribute uniformly for the match;
`play_match` gained `consistency_rng`/`k: &Knobs` parameters so the T5/T6 identity tests can
pin the identity value independent of the real production default (`0.25`). Finding: pooled
goals/match moved +9.5% at that default (more than "roughly unchanged" predicted) — recorded
in `MATCH_MODEL.md` §17 for T14 rather than chased, per T8's own scope fence.
`favourite_discrimination_regression_guard`'s pool widened 8→24 seeds (Consistency's added
variance was tripping the guard's sparsest tail bins) — test-pool sizing, not a knob change.

T9 (Condition & recovery, `MATCH_MODEL.md` §13) landed: the new `condition` module derives
pre-match condition from `GameState::recent_appearances` (§12's window, landed ahead of this
task) plus hidden `natural_fitness` and age — no new stored field, no new event. Because
`match_engine` has no `GameState`, condition arrives at `play_match` as an externally-computed
`conditions: &BTreeMap<PlayerId, f64>` (`GameState::condition(pid)` per player, built by
`commands::lineup_conditions`) rather than an in-engine knob like Consistency's; `build_xi`
attaches it to a new `XiPlayer.condition` field (absent-from-map defaults to `1.0`, the
identity), and `contest::fatigue_mult` gained a `condition` parameter scaling its minute-0
starting point rather than every attribute uniformly. `bin/calibrate` and all four pooled
guards stay at the identity setting by construction (no season-tracking `GameState` in that
harness) and are confirmed unchanged rather than merely close. The §2.4 aging blend also
landed in `development::phys_lmax` (shared with `valuation`'s two projections, one law, no
second integrator to drift) — a new `DevKnobs::aging_prof_weight` (`w`, identity `1.0`,
production `0.5`) blends Professionalism with Natural Fitness in the physical-aging term;
`career_arc` confirmed no reading moved outside its already-banked band at `w = 0.5`.

T10 (Injuries, `MATCH_MODEL.md` §14) landed: two hazard channels in `match_engine::resolve`,
both scaled by a new `Knobs::injury_rate` (identity `0.0`) and drawn from their own `INJURY_NS`
stream (`play_match` gained `injury_rng: &mut Rng`) — contact (a failed take-on, a headed
shot's aerial duel; `maybe_contact_injury`, rolled inline at each event) and ambient (muscle/
overload scaled by `(1 − condition)` and age; pre-rolled once per player at kickoff in
`build_xi` as one Bernoulli trial with a uniformly-drawn onset minute, fired by
`fire_due_ambient_injuries` as the possession loop's clock reaches it — a documented
simplification of a per-minute hazard, since expected count is what §14's targets care about).
Severity is a skewed categorical draw (`roll_injury`), resolved into `InjuryOutcome.days_out` —
`MatchOutcome.injuries` is populated for the first time (`cards`/`ratings` still empty pending
§15/§18). An injured player continues at reduced effectiveness for the rest of the match
(`impairment_mult`, identity `1.0`, production `0.6`) — no substitution yet, T12's job.
`GameState::available` is the new derived view `ai_pick_lineup_available` (a squad-filtering
sibling of `ai_pick_lineup`, `ai_pick_lineup_vs` gained a `today: GameDate` parameter to use
it), `validate_lineup` (a new `CommandError::PlayerUnavailable`), and
`effective_player_lineup`'s staleness check (falls back past a remembered lineup naming a
now-unavailable player) all read. Finding: the first magnitude pick read 3.88 match-missing
(`days_out >= 7`) injuries/club/season pooled over a real 6-seed season run, above §14's
1.5-2.5 target; scaling `injury_base_contact`/`injury_ambient_base` down by ~0.515 landed at
2.14, inside it — recorded in `MATCH_MODEL.md` §14 alongside the still-unverified sub-targets.

T11 (Fouls, cards, suspensions, `MATCH_MODEL.md` §15) landed: a foul check (`maybe_foul`)
rolls after every `TakeOn` resolution (either outcome) and after a failed `Pass` outside the
actor's own `Def` zone, on its own `FOUL_NS` stream (`play_match` gained `foul_rng: &mut Rng`,
`Knobs::foul_rate` identity `0.0`). A fired foul is a non-mirroring turnover — the beat returns
`(poss, zone)` unchanged rather than the take-on/pass's own outcome — and a severity draw sets
`Card::{Yellow, SecondYellow, Red}` onto the new `MatchOutcome.cards` (populated for the first
time; `ratings` still empty pending §18). A red sets `XiPlayer.sent_off_from_minute`;
`sample_by_presence` and `team_means` (both gained a `minute` parameter and now filter
`on_pitch`) shrink the XI from there on, with `team_means` recomputing per-tick only once a
side has actually lost a player (a cheap fast path reuses the once-per-match reading
otherwise); `current_gk` resolves a red-carded keeper by falling back to the lowest-indexed
on-pitch outfielder. `GameState::is_suspended`/`suspended_players` derive bans straight from
`season_cards` (a red/second-yellow at `current_matchday - 1`, or a 5th/10th/... accumulated
yellow there — self-clearing by construction, no "served" flag), joining `available()`
alongside the injury check; `ai_pick_lineup_vs`/`ai_pick_lineup_available` gained a `suspended:
&BTreeSet<PlayerId>` parameter so AI-controlled sides respect bans too. A sent-off player's
`MatchPlayed` minutes now stop at his dismissal instead of running to a flat 90. Finding: the
first knob pick (`foul_base: -3.3`) undershot badly (9.5 fouls/game, 1.06 yellows/team/match
vs §15's ~20-25/~2-3 targets); raising it to `-2.5` fixed the volume, but reusing the
fresh-yellow formula (plus its repeat/aggression bumps) for a second yellow let one repeat
fouler's foul count snowball into an implausible 0.39 reds/team/match — split into its own flat
`foul_second_yellow_base` knob, landing the final pick (16-seed pool) at fouls/game 20.2,
yellows/team/match 2.20, reds/team/match 0.038, all inside band. `ATTRIBUTE_SCHEMA.md` §9 item
3 resolves: Aggression alone sufficed — only the foul contest's own knobs needed tuning, never
the duel contests' weights, so the split tripwire never fired.

T12 (Substitutions, `MATCH_MODEL.md` §16) landed: `fforge_domain` gained a `substitution`
module (`SubCondition`, `SubAction`, `SubRule`, `ScoreState`) and `Lineup.bench: Vec<PlayerId>`/
`Lineup.sub_plan: Vec<SubRule>` (both `#[serde(default)]`, the identity). In
`match_engine::resolve`, `simulate` now owns `home`/`away`/their bench counterparts as mutable
`Vec<XiPlayer>` instead of borrowing — a substitution replaces a slot's `XiPlayer` outright —
while `step`/`take_shot`/`sample_by_presence`/`team_means` still only ever see a borrowed
`&[XiPlayer]` for one segment between decision points, so none of their signatures changed.
Decision points (half-time, the fixed `SUB_CHECKPOINTS` `[60, 70, 80]`, or forced immediately
when a tick's injury/card count grows for a given side) call `evaluate_decision_point`, which
walks `sub_plan`'s rules in list order and fires an action once every `SubCondition` clause
holds — fully RNG-free, since `build_xi_player` (the per-player body factored out of `build_xi`,
now shared with a new `build_bench`) pre-rolls every dressed player's Consistency multiplier and
ambient-injury check at kickoff, starters and bench alike. `XiPlayer` gained `entered_at_minute`
(`0.0` for a starter, the identity) so a substitute's `fatigue_mult` clock starts at his own
entry; a departed player's final minutes are captured into a side accumulator before his slot is
overwritten, since his `XiPlayer` no longer exists afterward to read them back from.
**No AI default-plan/bench-selection policy landed** — `ai_pick_lineup`/`ai_pick_lineup_vs`
field every AI-controlled side with an empty bench and plan (the substitution identity), mirroring
`ai_pick_tactics`'s own still-open seam; confirmed every pooled calibration guard reads
unchanged with the mechanism live, since AI-vs-AI league play never touches it. Finding: the
batch-3 T12 task spec pins the substitution cap at **3**, not the **5** `MATCH_MODEL.md` §16
first drafted at T2 — implemented as `fforge_domain::MAX_SUBSTITUTIONS`, recorded as a
divergence-correction in §16 rather than silently building to the newer number.

T13 (Match ratings & form, `MATCH_MODEL.md` §18) landed: a new `match_engine::ratings` module
(`compute_ratings`, `RatedPlayer`) folds an already-resolved `MatchOutcome.stream` plus final
minutes/score into a per-player 3.0-10.0 rating (base 6.0, event deltas, a minutes-share
regression toward base) — pure and RNG-free, called at the end of `simulate` once the stream is
built. `MatchOutcome.ratings` is populated for the first time; T12's `departed` accumulator
gained `Role` (needed for the clean-sheet gate). Two documented approximations: the
"caused-a-turnover" blame rule credits the *most recent* failed action for the conceding side,
not a full possession-chain reconstruction; and §18's own "1.0–10.0"/"`[3.0, 10.0]`" prose
mismatch is resolved in favour of the clamp (corrected in the doc). Closes
`TRANSFER_MODEL.md` §2.5's form deferral: `valuation::MarketContext` gained a `form` field built
from `GameState.recent_ratings` (verbatim, no second encoding), multiplied into `value()` as
`form_mult` (new `ValueKnobs::form_scale`/`form_bound`, bounded to ±12%, identity `1.0` for a
player with no recent ratings) — `MarketContext::from_world` and `market::resolve_window` both
gained a `recent_ratings` parameter, empty (the identity) for every harness/test with no real
`GameState`, `&state.recent_ratings` at the one production call site
(`commands::transfer_window_events`).

`fforge-core` is the active development front.

## Module map

| Module | Owns |
|---|---|
| `event` | `Event` enum — the append-only log's payload types, including the Phase-4 transfer/contract/finance/pool events (`TRANSFER_MODEL.md` §4), §10's `TransferDecisionSubmitted`/`TransferWindowClosed`, and `MatchPlayed`'s Phase-2e boundary fields (`MATCH_MODEL.md` §12: `injuries`/`cards`/`ratings`, serde-defaulted for pre-2e logs) |
| `market` | Phase-4 clearing loop and window mechanics (`TRANSFER_MODEL.md` §5, §7): `resolve_window` (gained a `recent_ratings: &BTreeMap<PlayerId, Vec<u8>>` parameter, T13 — threaded straight into `MarketContext::from_world`; `commands::transfer_window_events` passes `state.recent_ratings`, every other caller/test passes empty) — freeze the valuation cache once, then simultaneous rounds of `club_ai`-decided bids/listings (`human_club: Option<ClubId>` substitutes `club_ai::RecordedPolicy` for that one club, §10's pre-commitment seam; every other club runs `UtilityPolicy`), contention resolved by the selling club's ranking (fee, buyer reputation, `ClubId`) then player consent (`MarketKnobs`'s wage/reputation-threshold roll), refused pairs never re-proposed (classic deferred acceptance — the actual convergence mechanism; `MAX_ROUNDS = 12` is the adversarial-input cap, not the normal exit). `filter_affordable` applies the same resolve-time affordability/squad-bounds/GK-floor/availability gate to every club's decisions regardless of which policy produced them — a no-op for `UtilityPolicy` (already compliant by construction), the actual gate for a `RecordedPolicy` plan that bypasses that producer-side filtering; it also re-validates a `Bid`'s claimed seller against the round's live observation, closing a staleness gap a static replay can hit that `UtilityPolicy`'s always-fresh decisions never could. Returns `WindowOutcome { transfers, rejected_bids, valuations, unfilled_needs, rounds_used }` — only `transfers` folds into `Event::TransferCompleted`; the rest is a Trace, exactly `MatchOutcome.stream`'s shape (`MATCH_MODEL.md` §7). `summer_window_close`/`winter_window_close` derive window boundaries from the season (never day-of-year constants); `commands::transfer_window_events` fires resolution when `advance_matchday` crosses one, using `TRANSFER_STREAM_NS \| window_index` as its RNG stream. Its `calibrate` submodule (re-exported at `market::{MarketTelemetry, MarketReport, run_market_calibration, print_report}`) is the §11 pathology harness: since `WindowOutcome`'s rich Trace never survives the fold, `MarketTelemetry` reads competitive-balance/financial-health metrics off the folded `World` at each season boundary (`record_season_end`, via `state::league_table` and `valuation::value_all`) while consuming `TransferCompleted`/`YouthIntake`/etc. as an `EventObserver` for fee/volume data — pooled over many seeds × ~15 seasons, exactly the multi-seed-pooling discipline `career_arc` and `match_engine::calibrate` established. `submit_player_clubs_ai_equivalent_plan` keeps the harness's own `player_club` behaving like every other AI club post-§10 (see "Current phase" above). Harness plumbing only; never feeds back into `ValueKnobs`/`FinanceKnobs` by itself |
| `club_ai` | Phase-4 Layer-3 club decision AI (`TRANSFER_MODEL.md` §6, §6.1): the `ClubPolicy` trait (`ClubObservation` in, `Vec<TransferDecision>` out — the Gym-shaped seam `ai_pick_lineup`'s doc comment anticipated), `UtilityPolicy` (`need(club, role)` = depth + quality-vs-own-reputation-target + succession risk from `valuation::project_ca_batch`; buy shortlists ranked by `need · (value − asking_price)`, with a role-coverage **override** ahead of that ranking (`TRANSFER_MODEL.md` §11's hard stabilizer): a candidate in a role currently below its §6 hard minimum (`hard_minimum_violations` — today just `Gk` below `min_goalkeepers`) ranks first regardless of `need · surplus` elsewhere, exempt from the positive-surplus filter too, so no ordinarily-attractive opportunity can outbid it; the cash/wage/squad-ceiling stabilizers still gate it, so a club with no headroom must sell first; sell lists from §6's first two triggers plus a third, squad-size pressure term — `UtilityKnobs::squad_pressure_start`/`_exponent`/`_max_listings` — that makes at-template (not yet genuinely surplus) roles listable through a bounded, continuously-growing quota as the squad approaches `squad_max`, addressing the §9 "squads pin at the ceiling" residual; GK is excluded from this term since its template sits only one above `min_goalkeepers`), and `observe()` (builds a `ClubObservation` off `World` + the `value_all` cache — the only place in this module that reads `World`). `RecordedPolicy` (§10) is the second `ClubPolicy`: replays a pre-committed `Vec<TransferDecision>` verbatim on every call, ignoring `ClubObservation` entirely — never adapting is the point, and an empty plan yields no decisions rather than falling back to `UtilityPolicy`. Squad bounds `[18, 30]`, `≥2` GK, cash and wage headroom are hard stabilizers, not utility terms — the pressure term never touches them. **`UtilityKnobs::asking_markup` must stay `<= 1.0`**: with every club pricing off the same omniscient `value()` (§2.6 — no private valuations in v1), an ask *above* value makes `need · (value − asking_price)` negative for every buyer regardless of need, so no trade can ever clear — filed as a corrected divergence from §12 item 6's literal "markup" phrasing, caught by `club_ai::tests::real_observed_candidates_can_actually_produce_a_bid`. Decisions only — the clearing loop lives in `market` |
| `condition` | Match condition (`MATCH_MODEL.md` §13, T9): `ConditionKnobs` and the pure `condition(recent, as_of, natural_fitness, age_years, k) -> f64` — a decaying per-appearance load debt cleared at a Natural-Fitness/age-scaled recovery rate, floored, `1.0` for an empty recent-appearance slice (the identity — `MATCH_MODEL.md` §2.1's `GameState`-derived analogue of Consistency's `sigma_max: 0.0`). RNG-free: recovery is a deterministic function of the calendar. `GameState::condition(pid)` (in `state`) is the only caller; `match_engine` never imports this module directly — condition arrives at `play_match` as a pre-computed map, since the engine has no `GameState` |
| `state` | `GameState` — pure fold (`apply`/`replay`), `TableRow`, `league_table()`. The Phase-4 fold arms (six from §4, two more from §10's `TransferDecisionSubmitted`/`TransferWindowClosed`) are pure integer/assignment operations only (no RNG, no math beyond addition, no engine calls) and keep club rosters sorted after mutation, so replay-path equality holds. `pending_transfer_decisions: Vec<TransferDecision>` holds the current pre-commitment (§10) — set on submission, cleared on window close, good for exactly one window. `apply_transfer_completed`/`apply_finance_deltas` are `pub(crate)` free functions so `market`/`commands` can apply the identical mutation to a working `World` without a second encoding. The Phase-2e `MatchPlayed` arm (`MATCH_MODEL.md` §12) additionally folds `injuries` → `Player.injured_until`, `cards` → `season_cards` (cleared on `SeasonStarted`; bans are derived, never stored), `ratings` → `recent_ratings` (capped at `RATING_FORM_WINDOW`), and every XI appearance → `recent_appearances`, the §13 rolling window pruned to `CONDITION_WINDOW_DAYS` wherever the fold moves the date. `condition(pid)` (T9) is a pure read of that window plus `World` — no field, nothing to desync. `available(pid)` (T10 §14, T11 §15) is the derived view over `injured_until` and `is_suspended` — a red/second-yellow at `current_matchday - 1`, or a 5th/10th/... accumulated yellow there, checked fresh against `season_cards` every call (no stored ban, no "served" flag); `suspended_players()` pools the set for callers with no per-player loop of their own. `ai_pick_lineup_available`/`validate_lineup`/`effective_player_lineup` all read `available`/`is_suspended` |
| `commands` | `Command` enum, `step()` — validates a proposal and produces the events for it; `player_match_preview()` — a pure query, re-deriving the same lineup selection and RNG stream `advance_matchday` is about to use, for live-viewing the human's own fixture before it's recorded. `Command::SubmitTransferDecision` (§10) runs `validate_transfer_decisions` (submit-time shape only: targets exist, aren't already owned, prices aren't negative, sell targets are the club's own) before recording `Event::TransferDecisionSubmitted` — affordability is resolve-time, inside `market::filter_affordable`, not here. `dev_ticks_between` returns its compounded working `World` alongside the events, so `transfer_window_events` (fired from `advance_matchday` on a §7 boundary crossing) resolves against this advance's developed attributes and finance deltas, not the pre-tick world, passes `Some(state.player_club)`/`state.pending_transfer_decisions` through to `resolve_window`, and emits `Event::TransferWindowClosed` for every crossed boundary regardless of outcome so a pre-committed plan expires on schedule; `season_start_date` derives the season's kickoff from `state.date`/`current_matchday` rather than storing it. `lineup_conditions` (T9) builds `play_match`'s `conditions` map from `GameState::condition` for both call sites. `validate_lineup` (T10 §14, T11 §15) rejects a lineup naming an unavailable (injured or suspended) player (`CommandError::PlayerUnavailable`), and (T12 §16) validates `bench` (`≤ BENCH_SIZE`, in-squad, no duplicates with itself or the starters — `CommandError::BenchTooLarge`) and every `sub_plan` rule's named players against the union of starters and bench (`CommandError::UnknownSubPlanPlayer`); `effective_player_lineup` skips a remembered lineup that has gone stale, falling back to `ai_pick_lineup_available` with `state.suspended_players()`; `advance_matchday`/`player_match_preview` compute the suspended set once and thread it through every `ai_pick_lineup_vs` call for AI-controlled sides |
| `session` | `Session` — owns the log + folded state, routes commands, notifies observers; `save_log`/`load_log` (JSON-lines) |
| `observer` | `EventObserver` trait, `SeasonTelemetry` — passive event-stream consumers (trace/telemetry spine) |
| `news` | The R2 notification Trace: `NewsItem`/`NewsKind`/`EventRef`/`Audience`, `NewsRenderer` + `TemplateRenderer`, and `NewsObserver` (`EventObserver` for event-derived news; `check_conditions(&GameState)` for state-condition news; `observe_rejected_bids` for `WindowOutcome`'s Trace). Maintains small incremental indices (fixture→clubs, squad membership, each player's/club's most recent contract/finance/squad-affecting `EventRef`) purely from events already seen, so `check_conditions` — which only ever sees `&GameState`, never the log — can still attach real provenance to a state-condition item. `warned_*` sets make every state-condition check edge-triggered (fires once per newly-true condition, re-arms on recovery) so a season-long inbox stays bounded rather than repeating the same fact every call. Not wired into `commands.rs`/`session.rs`/`fforge-game` yet — a self-contained module + test suite, by explicit scope fence |
| `match_engine` | Phase-2a engine, now tactics-, consistency-, condition-, injury-, foul/card-, substitution-, and rating-aware (`TACTICS_MODEL.md` §6-§7, `MATCH_MODEL.md` §17/§13/§14/§15/§16/§18, T6-T13): `play_match(world, home, away, rng, consistency_rng, injury_rng, foul_rng, k: &Knobs, conditions: &BTreeMap<PlayerId, f64>, today: GameDate)` — unchanged signature since T12; `home_lineup.bench`/`.sub_plan` are read internally, so an empty `Lineup` (the identity) needs no special-casing at any call site — (`MatchOutcome { home_goals, away_goals, stream }` plus the §12 boundary fields — `injuries` (T10), `cards` (T11), and `ratings` (T13) now populated, `minutes` a flat 90 per starter except a sent-off or substituted player's, which stops at his departure minute, and an entering substitute's, which starts at his entry (T12); `consistency_rng`/`injury_rng`/`foul_rng` are each their own stream, `k` lets callers pin `consistency_sigma_max`/`injury_rate`/`foul_rate` independent of the production defaults — the T5/T6 identity tests' seam; `conditions` is a pre-computed per-player map — `GameState::condition` output, since the engine itself has no `GameState` — absent-from-map defaulting to `1.0`, the identity; `today` only feeds the ambient injury channel's age term), `lineup_strength`, `ai_pick_lineup` (XI selection only, always neutral tactics, no availability filter), `ai_pick_lineup_available` (T10/T11: the same greedy fill over only the squad available as of `today` and not in the caller's `suspended: &BTreeSet<PlayerId>` — falls back to the unfiltered squad below `XI` available, a defensive floor), `ai_pick_tactics`/`AiTacticKnobs` (the §7 policy — implemented, tested, but gated), `ai_pick_lineup_vs` (the real call-site: availability/suspension-filtered XI + tactics, tactics applied only when `AI_TACTICS_ENABLED` — currently `false`, pending `TACTICS_MODEL.md` §9 item 6's open design question — is `true`; every real AI match runs `Tactics::neutral()` today). Submodules: `zone` (five-zone state + role→zone presence table), `knobs` (the fitted `Knobs` table, now including Consistency's T8 fields, Injuries' T10 fields, and Fouls' T11 fields — `foul_rate`, the p_foul logit's aggression/composure/press/fatigue terms, and the severity bands including the decoupled `foul_second_yellow_base`), `contest` (attribute→contest maps, the logistic resolver, fatigue — `fatigue_mult` takes a per-side `press_mult` and a `condition` scaling its minute-0 starting point, T9), `tactics` (§3's per-side `SideEffects` resolution — pure, RNG-free, computed once per match; `b_pass_delta_by_zone` and `Pressing::High`'s zone-profiled bias are the T7-addendum fixes, §5), `resolve` (the possession loop; `select_action`/`step`/`take_shot` all read `SideEffects` alongside `Knobs` and still only ever take a borrowed `&[XiPlayer]` — unchanged by T12, since `simulate` now owns `home`/`away` as mutable `Vec<XiPlayer>` and only mutates a slot *between* segments, at a decision point; `build_xi_player` (T12: the per-player body factored out of `build_xi`) draws one XI/bench player's Consistency multiplier, attaches his `condition` from the caller's map, and pre-rolls the ambient injury channel — one Bernoulli trial with a uniformly-drawn onset minute, T8/T9/T10 — shared by `build_xi` (starters) and the new `build_bench` (T12), so every dressed player is drawn in the same fixed order regardless of whether he ever enters; `maybe_contact_injury` rolls the contact channel inline at a failed take-on or a headed shot's aerial duel; `fire_due_ambient_injuries` fires a pre-rolled ambient injury once the clock reaches its onset; `impairment_mult` scales an injured player's effective attributes for the rest of the match, identity `1.0`; `maybe_foul` (T11) rolls the foul check after every `TakeOn` resolution and a failed non-`Def` `Pass`, overriding to a non-mirroring turnover and resolving a `Card` severity on a hit; `on_pitch`/`current_gk` (T11) let `sample_by_presence`/`team_means` renormalize over a shrunken XI once a red card fires, `team_means` recomputing per-tick only once a side has actually lost a player; `evaluate_decision_point`/`condition_holds` (T12) walk a side's `sub_plan` at half-time, the fixed `SUB_CHECKPOINTS` `[60, 70, 80]`, or forced immediately when a tick's injury/card count grows, matching each rule's `SubCondition`s against already-resolved match state and executing `Substitute` (swaps an XI slot, consumes a bench entry, capped at `MAX_SUBSTITUTIONS`, records the departed player's minutes before his slot is overwritten) or a tactics-change action — all RNG-free), `ratings` (T13: `compute_ratings`/`RatedPlayer` — a pure fold over the resolved `stream` plus final minutes/score into `MatchOutcome.ratings` per §18's delta table, base 6.0, clamped `[3.0, 10.0]`, called once at the end of `simulate` after the stream is fully built), `stream` (`MatchEvent` schema + commentary rendering, now including `Foul { card: Option<Card> }` (T11) and `Substitution { player_out: PlayerId }` (T12)), `golden` (`#[cfg(test)]`: the batch-3 T5 pinned Phase-2a baseline every 2e identity invariant asserts against — `identity_2e_knobs` now pins `consistency_sigma_max: 0.0`, `injury_rate: 0.0`, and `foul_rate: 0.0`), `calibrate` (`StreamTelemetry`'s per-zone pass-completion and turnover-won-by-zone cuts, plus T11's `fouls`/`yellows`/`reds` counters and their per-match/per-team rate accessors; `run_head_to_head` — the §7 triangle harness; stays at the condition identity, §13's status note; the T7-addendum zone-profiling test also pins `injury_rate: 0.0`/`foul_rate: 0.0` to isolate pressing's own effect) |
| `development` | Phase-3 growth engine (`DEVELOPMENT_MODEL.md` §2–§5): the `DevKnobs` table (sibling of `match_engine::Knobs`), the per-category age envelope, PA-scaled targets, `resolve_dev_profile`/`resolve_coaching` (worldgen edge), and `tick_changes` — the growth math producing a `DevelopmentTick`'s resolved deltas. The per-attribute rate law is factored into `attr_rate`, shared verbatim with `valuation`'s projection so there is one law (no second integrator to drift). `phys_lmax(knobs, professionalism, natural_fitness)` (T9, `MATCH_MODEL.md` §13/R8) is the same discipline applied to the physical-aging term: a Professionalism/Natural-Fitness blend weighted by `DevKnobs::aging_prof_weight` (identity `1.0`, production `0.5`), shared by `tick_changes` and both of `valuation`'s projections. All RNG/math lives here; `apply` only integer-adds via `apply_attr_step` |
| `valuation` | Phase-4 centralized value function (`TRANSFER_MODEL.md` §2): `value` / `value_all` (the §2.7 per-window `BTreeMap<PlayerId, Money>` cache), `project_ca` (runs `development::attr_rate` forward, jitter off, minutes/coaching neutral), `project_ca_batch` (many players, one shared knob-derived `DevTables` — `club_ai::observe`'s per-squad projection), the `ValueKnobs` §9 table (plausibility-picked, sibling of `DevKnobs`; gained `form_scale`/`form_bound`, T13), and `MarketContext` (bounded league-wide role scarcity, plus a per-player `form` multiplier since T13 — `from_world` gained a `recent_ratings: &BTreeMap<PlayerId, Vec<u8>>` parameter, `GameState.recent_ratings` verbatim or empty/neutral for a caller with none; `form_mult(pid)` reads `1.0` for anyone absent from it). `value_with` integrates each player's whole 0..=horizon_years trajectory in one pass (`project_ca_series`) rather than once per year — same numbers, no redundant re-integration of the shared prefix; multiplies `contract_mult * scarcity_mult * form_mult` into the base curve. A pure Layer-2 function — prices, never decides; no market/club-AI here (Phase 4 §5–§6) |
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