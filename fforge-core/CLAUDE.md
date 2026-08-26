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

Phase 2e began with `MATCH_MODEL.md` §11's sequencing step 1 — the §12 boundary
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
distinct from `appearances_since_tick`, which stays monthly and tick-reset. At that step the
engine emitted all three vectors empty, so no RNG draw and no calibration reading moved; the
§14/§15/§18 models that fill them landed afterwards as T10/T11/T13, and **all three are
populated today** — nothing on this boundary is a placeholder any more.

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
gated the policy behind `match_engine::AI_TACTICS_ENABLED` (`false` at the time), so every
AI-controlled match ran `Tactics::neutral()` in practice, and T8-T13 could proceed
against T5's stable baseline.

**T7-R has since resolved §9 item 6** (`TACTICS_MODEL.md` §5's "T7-R finding"). T7's stop
condition rested on a false premise: both of its fixes moved *logit-class* terms (additive
`contest_p` biases), which are ~4× weaker in absolute probability than the *advance-class*
multipliers on raw transition probabilities — and `advance_mult`, the term that actually
decides Tempo dominance, was never touched. Neutralizing it flipped which tactic won.
Three advance-class magnitudes were re-fitted to net-neutral (`Tempo::Direct` `advance_mult`
1.30→1.13, `Tempo::Patient` 0.80→0.88, `Pressing::High` `opp_mid_advance_mult` 1.15→1.02;
`Pressing::High`'s `fatigue_mult ×1.30` deliberately untouched — it is what makes Pressing
squad-conditional). Result: §5's cyclic triangle is **retired** (edges now flat within
±0.4pt, where `Direct` previously won 32/32 profile×world cells), and **squad-conditional
non-dominance is adopted** — the press's expected-points share rises +0.0194 from a
technical to a physical squad (8 worlds × 3000 seeds, ~11σ), a sign *forced* by
`contest::fatigue_mult`'s `(1 − stamina)` term rather than fitted. New harness:
`calibrate::run_squad_conditional_probe` / `bin/calibrate --squad-conditional` (pools across
worlds, not just match seeds — a single-world read flips on noise); new guard
`calibrate::tests::tactics_are_squad_conditionally_non_dominant`.

**T7-R2 then resolved §9 item 7 and flipped the gate.** Item 6's fit exposed `Mentality` as the
last dominant instruction (`Attacking` beat `Balanced` 0.540/0.460, and the §5 counter posture
`Defensive+Direct` *lost* to `Attacking` 0.431/0.569) — the same lever-class mismatch in its
purest form: two advance-class gains (`advance_mult`/`penetrate_mult` ×1.20) against one
logit-class cost (`def_bias` −0.08), i.e. a *risk* axis with no advance-class risk term. §5 had
assigned that role to turnover mirroring, but mirroring sends a ball lost high to a **deep**
opponent restart, which protects the committed side rather than punishing it. Fixed by adding
the missing concession rather than shrinking the gains (the gains are what drive §8's goal
rows): `Attacking` now concedes opponent `p_mid_advance`/`p_attc_penetrate` ×1.25, `Defensive`
denies them ×0.79 — reusing `opp_mid_advance_mult`/`opp_penetrate_mult`, so no new deformation
type. Result (8 worlds × 3000 seeds): every forced matchup on the axis within ±0.3pt of even
after correcting the harness's ~+0.4pt leg-asymmetry offset, and §8's "match goals ±0.2-0.4"
predictions in band for the first time (+0.296 / −0.238, previously +0.16 / −0.13). Guarded by
`calibrate::tests::mentality_is_a_risk_axis_not_a_better_setting`.

**`AI_TACTICS_ENABLED` is now `true`** — every AI-controlled side picks real tactics. Re-banked
league aggregates (24 seeds, `TACTICS_MODEL.md` §8): goals/match 2.84 → **2.59**, H/D/A
43.0/25.7/31.3 → 42.7/26.8/30.6, headed-goal share 20.7% → 23.4% and wide-origin 27.2% → 30.3%
(Width instructions now live), fouls/cards unchanged. **No `b_beat`-style re-fit was needed**;
161/161 green with the flag on, all four pooled guards and both golden-baseline tests included
(the latter automatic — §4's invariant makes `Tactics::neutral()` bit-identical either way).

**T14 closed Phase 2e: both remaining harnesses re-banked, no re-fit needed.** With AI tactics live,
`career_arc` (8 seeds × 16 seasons) and `market::calibrate` (24 seeds × 15 seasons) were both re-run at
their banked pooling and every metric landed inside its own per-seed spread — development integrates a
monthly rate law over years and the market prices off CA, so neither is sensitive to a per-match
tactical effect, even though the causal path to both is real (tactics → fatigue → cards → suspensions →
who plays; tactics → results → ratings/revenue → `MarketContext.form`). Readings are in
`DEVELOPMENT_MODEL.md` §6 and `TRANSFER_MODEL.md` §9.

T14's doc-reconciliation pass also corrected one **live doc-vs-code divergence** worth knowing:
`MATCH_MODEL.md` §11's RNG rule said all 2e randomness must come from the *single* per-fixture stream,
"never from a second stream". The implementation uses four (`FIXTURE_STREAM_NS`, `CONSISTENCY_NS`,
`INJURY_NS`, `FOUL_NS`) and is correct to — one shared stream would make every feature's identity
setting non-local (turning off `injury_rate` would shift every later draw), which would have made the
bit-for-bit identity tests anchoring the whole rollout impossible to write past the first landing. §11
now records the separate-stream rule and why. Three model-vs-target gaps were also filed rather than
fixed, since a re-bank pass records readings and does not get to invent mechanisms: Mental plateau
onset reads ~26.4 against an "early 30s" target (with the veteran mental slope flat at +0.02 against
~+0.3 — the same fact twice), wonderkid flop rate reads 0.00 against ~4%, and transfer volume reads
1.805/club/window against §11's ~2-5 band, unmoved by two successive re-banks.
**Two of those three are since closed** by the wonderkid seeding fix (see below): the flop rate reads
**0.036** on its `start_age ≤ 18` headline cohort and the mental pair reads **32.12 / +0.36**, both on
target. Transfer volume stays open at **1.880** (`BACKLOG.md` §4.1), now unmoved by three re-banks.

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
`MatchOutcome.injuries` is populated for the first time (`cards`/`ratings` followed at
T11/§15 and T13/§18). An injured player continues at reduced effectiveness for the rest of the match
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
time; `ratings` followed at T13/§18). A red sets `XiPlayer.sent_off_from_minute`;
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
**No AI default-plan/bench-selection policy landed at T12** — `ai_pick_lineup`/`ai_pick_lineup_vs`
fielded every AI-controlled side with an empty bench and plan (the substitution identity), mirroring
`ai_pick_tactics`'s own seam; every pooled calibration guard read unchanged with the mechanism live,
since AI-vs-AI league play never touched it. **That seam is now filled — see the S1a/S1b entry
below.** Finding: the
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

**S1a/S1b — the AI bench-selection and default-substitution-plan policy** (`MATCH_MODEL.md` §16's
own subsection, drafted at S1a and implemented at S1b). Closes T12's deferred seam and `BACKLOG.md`
§5.1: `pick_lineup_from` now calls `ai_pick_bench_and_plan`, so every AI-controlled side fields a
real `bench` (GK-reserved slot first, then a role-spread pass, then CA fill to `BENCH_SIZE`) and a
default `sub_plan` (forced-cover pairs on `PlayerInjured`, ≤2 fatigue rules on
`MinuteAtLeast(70) + PlayerConditionBelow(80)`, and two tactics-only chase/hold rules). Pure and
RNG-free. **Vocabulary finding recorded in §16:** score-conditioned *player* selection is not
expressible in the `SubCondition`/`SubAction` language, so chase/hold changes mentality only, never
who is on the pitch. Both golden-baseline tests now clear `bench`/`sub_plan` explicitly to pin the
substitution identity, exactly as `neutral_tactics_reproduce_phase_2a_bit_for_bit` already did for
tactics. **Measurement refuted three of §16's five predictions** — subs/match reads **0.13** against
a predicted +3 to +5, and non-XI mean minutes **45.1** against a predicted 10–20 — with the root
causes recorded there (forced-cover fires only on a rare in-match injury; the fatigue rules are
*unreachable* in `bin/calibrate`, which runs at the condition identity; and §14 draws ambient-injury
onset uniformly over the match, so substitutes enter at a mean minute of ~45, not ~75). Goals/match
and cards held. **`bin/calibrate` is the wrong harness to re-read the fatigue channel on** — it has
no `GameState`, so `conditions` is empty; a real-season harness is needed (`BACKLOG.md` §4.3).

**W2–W5 — the wonderkid seeding fix** (`DEVELOPMENT_MODEL.md` §8; `BACKLOG.md` §2, now closed —
**Phase 5 is unblocked**). The project's **first note-wins doc-vs-code reconciliation** (§8.2
records why, against two prior code-wins precedents). `worldgen::gen_player` derived
`potential = best_ca + headroom` — CA first — while §2.1 had always specified the opposite.
W3 inverted it: PA is drawn first (anchored on club quality), then attributes are seeded on
`(PA/NORM)·env_c(age − φ)` beneath it, reusing `development`'s own `EnvTables`/`norms_by_role`/
`role_ceiling_consts` through a new shared `worldgen::SeedTables` rather than re-encoding the
envelope; `youth_discount` and `headroom` are deleted, and `resolve_dev_profile` gained a `phi`
parameter (split out as `resolve_bloomer_phase`) so seeding and development read the *same* draw.
`pool::youth_cohort` inherits it. W4 re-fit `DevKnobs` in the same change (§8.5's fixed procedure —
`k_dec` alone first, then `plast_*`/`e_sigma`/`e_min` **jointly**): `k_dec` 0.30 → 1.0,
`plast_*` (23.5, 2.2) → (31.25, 4.6), `e_sigma`/`e_min` (0.42, 0.15) → (0.095, 0.61) — **loosened**,
the reverse of the re-fit that had compensated for the bug. W5 re-banked `bin/market`.
Headline result: `fit_pa_from_ca_age_youth`'s `residual_sd` rose **2.61 → 5.566**, so PA is no
longer trivially recoverable from `(CA, age)` — the property Phase 5's scouting fog-of-war
structurally requires. Three consequences worth carrying forward:
- **A world re-roll is not a re-fit.** The golden baselines and seven `fforge-game` snapshots were
  re-pinned because the *world* changed, not the engine.
- **`k_dec` doubled as a correctness check on W3** (§8.6): with seeding env-consistent, raising it
  0.30 → 1.0 barely moved the veteran slopes, confirming veterans now start *on* the envelope.
  A residual finding: the 30→35 physical slope reads ~−2.0 regardless of `k_dec` (it goes
  *shallower* at 2.5, past `env_phys`'s own `d = 27` inflection), so §8.5's −2.2..−3.2 accept-band
  is likely unreachable without moving `env_phys`. Filed, not chased.
- **Two collateral effects, both filed rather than fixed.** `valuation::tests::
  physical_role_depreciates_faster_than_a_technical_one` weakened `>` → `>=` (the much wider
  plasticity window keeps growth active into the mid-30s, offsetting the decline differential for
  both roles equally at every age/horizon probed); and the market's finance side moved sharply
  (insolvency 5.34/20 → 0.01/20, hoarding 0.68/20 → 5.85/20) because wages are set once at signing
  off `best_ca`, which the invert lowers across the whole youth band — attributed to seeding, not to
  S1b's `MarketContext.form` channel, which has no path to wages.

`fforge-core` is the active development front.

## Module map

| Module | Owns |
|---|---|
| `event` | `Event` enum — the append-only log's payload types, including the Phase-4 transfer/contract/finance/pool events (`TRANSFER_MODEL.md` §4), §10's `TransferDecisionSubmitted`/`TransferWindowClosed`, and `MatchPlayed`'s Phase-2e boundary fields (`MATCH_MODEL.md` §12: `injuries`/`cards`/`ratings`, serde-defaulted for pre-2e logs) |
| `market` | Phase-4 clearing loop and window mechanics (`TRANSFER_MODEL.md` §5, §7): `resolve_window` (gained a `recent_ratings: &BTreeMap<PlayerId, Vec<u8>>` parameter, T13 — threaded straight into `MarketContext::from_world`; `commands::transfer_window_events` passes `state.recent_ratings`, every other caller/test passes empty) — freeze the valuation cache once, then simultaneous rounds of `club_ai`-decided bids/listings (`human_club: Option<ClubId>` substitutes `club_ai::RecordedPolicy` for that one club, §10's pre-commitment seam; every other club runs `UtilityPolicy`), contention resolved by the selling club's ranking (fee, buyer reputation, `ClubId`) then player consent (`MarketKnobs`'s wage/reputation-threshold roll), refused pairs never re-proposed (classic deferred acceptance — the actual convergence mechanism; `MAX_ROUNDS = 12` is the adversarial-input cap, not the normal exit). `filter_affordable` applies the same resolve-time affordability/squad-bounds/GK-floor/availability gate to every club's decisions regardless of which policy produced them — a no-op for `UtilityPolicy` (already compliant by construction), the actual gate for a `RecordedPolicy` plan that bypasses that producer-side filtering; it also re-validates a `Bid`'s claimed seller against the round's live observation, closing a staleness gap a static replay can hit that `UtilityPolicy`'s always-fresh decisions never could. Returns `WindowOutcome { transfers, rejected_bids, valuations, unfilled_needs, rounds_used }` — only `transfers` folds into `Event::TransferCompleted`; the rest is a Trace, exactly `MatchOutcome.stream`'s shape (`MATCH_MODEL.md` §7). `summer_window_close`/`winter_window_close` derive window boundaries from the season (never day-of-year constants); `commands::transfer_window_events` fires resolution when `advance_matchday` crosses one, using `TRANSFER_STREAM_NS \| window_index` as its RNG stream. Its `calibrate` submodule (re-exported at `market::{MarketTelemetry, MarketReport, run_market_calibration, print_report}`) is the §11 pathology harness: since `WindowOutcome`'s rich Trace never survives the fold, `MarketTelemetry` reads competitive-balance/financial-health metrics off the folded `World` at each season boundary (`record_season_end`, via `state::league_table` and `valuation::value_all`) while consuming `TransferCompleted`/`YouthIntake`/etc. as an `EventObserver` for fee/volume data — pooled over many seeds × ~15 seasons, exactly the multi-seed-pooling discipline `career_arc` and `match_engine::calibrate` established. `submit_player_clubs_ai_equivalent_plan` keeps the harness's own `player_club` behaving like every other AI club post-§10 (see "Current phase" above); a youth-valuation cut (`start_age ≤ YOUTH_MAX_AGE = 18`: mean/median value and headcount per season-end snapshot) was added at W5 as its own reported row. Harness plumbing only; never feeds back into `ValueKnobs`/`FinanceKnobs` by itself |
| `club_ai` | Phase-4 Layer-3 club decision AI (`TRANSFER_MODEL.md` §6, §6.1): the `ClubPolicy` trait (`ClubObservation` in, `Vec<TransferDecision>` out — the Gym-shaped seam `ai_pick_lineup`'s doc comment anticipated), `UtilityPolicy` (`need(club, role)` = depth + quality-vs-own-reputation-target + succession risk from `valuation::project_ca_batch`; buy shortlists ranked by `need · (value − asking_price)`, with a role-coverage **override** ahead of that ranking (`TRANSFER_MODEL.md` §11's hard stabilizer): a candidate in a role currently below its §6 hard minimum (`hard_minimum_violations` — today just `Gk` below `min_goalkeepers`) ranks first regardless of `need · surplus` elsewhere, exempt from the positive-surplus filter too, so no ordinarily-attractive opportunity can outbid it; the cash/wage/squad-ceiling stabilizers still gate it, so a club with no headroom must sell first; sell lists from §6's first two triggers plus a third, squad-size pressure term — `UtilityKnobs::squad_pressure_start`/`_exponent`/`_max_listings` — that makes at-template (not yet genuinely surplus) roles listable through a bounded, continuously-growing quota as the squad approaches `squad_max`, addressing the §9 "squads pin at the ceiling" residual; GK is excluded from this term since its template sits only one above `min_goalkeepers`), and `observe()` (builds a `ClubObservation` off `World` + the `value_all` cache — the only place in this module that reads `World`). `RecordedPolicy` (§10) is the second `ClubPolicy`: replays a pre-committed `Vec<TransferDecision>` verbatim on every call, ignoring `ClubObservation` entirely — never adapting is the point, and an empty plan yields no decisions rather than falling back to `UtilityPolicy`. Squad bounds `[18, 30]`, `≥2` GK, cash and wage headroom are hard stabilizers, not utility terms — the pressure term never touches them. **`UtilityKnobs::asking_markup` must stay `<= 1.0`**: with every club pricing off the same omniscient `value()` (§2.6 — no private valuations in v1), an ask *above* value makes `need · (value − asking_price)` negative for every buyer regardless of need, so no trade can ever clear — filed as a corrected divergence from §12 item 6's literal "markup" phrasing, caught by `club_ai::tests::real_observed_candidates_can_actually_produce_a_bid`. Decisions only — the clearing loop lives in `market` |
| `condition` | Match condition (`MATCH_MODEL.md` §13, T9): `ConditionKnobs` and the pure `condition(recent, as_of, natural_fitness, age_years, k) -> f64` — a decaying per-appearance load debt cleared at a Natural-Fitness/age-scaled recovery rate, floored, `1.0` for an empty recent-appearance slice (the identity — `MATCH_MODEL.md` §2.1's `GameState`-derived analogue of Consistency's `sigma_max: 0.0`). RNG-free: recovery is a deterministic function of the calendar. `GameState::condition(pid)` (in `state`) is the only caller; `match_engine` never imports this module directly — condition arrives at `play_match` as a pre-computed map, since the engine has no `GameState` |
| `state` | `GameState` — pure fold (`apply`/`replay`), `TableRow`, `league_table()`. The Phase-4 fold arms (six from §4, two more from §10's `TransferDecisionSubmitted`/`TransferWindowClosed`) are pure integer/assignment operations only (no RNG, no math beyond addition, no engine calls) and keep club rosters sorted after mutation, so replay-path equality holds. `pending_transfer_decisions: Vec<TransferDecision>` holds the current pre-commitment (§10) — set on submission, cleared on window close, good for exactly one window. `apply_transfer_completed`/`apply_finance_deltas` are `pub(crate)` free functions so `market`/`commands` can apply the identical mutation to a working `World` without a second encoding. The Phase-2e `MatchPlayed` arm (`MATCH_MODEL.md` §12) additionally folds `injuries` → `Player.injured_until`, `cards` → `season_cards` (cleared on `SeasonStarted`; bans are derived, never stored), `ratings` → `recent_ratings` (capped at `RATING_FORM_WINDOW`), and every XI appearance → `recent_appearances`, the §13 rolling window pruned to `CONDITION_WINDOW_DAYS` wherever the fold moves the date. `condition(pid)` (T9) is a pure read of that window plus `World` — no field, nothing to desync. `available(pid)` (T10 §14, T11 §15) is the derived view over `injured_until` and `is_suspended` — a red/second-yellow at `current_matchday - 1`, or a 5th/10th/... accumulated yellow there, checked fresh against `season_cards` every call (no stored ban, no "served" flag); `suspended_players()` pools the set for callers with no per-player loop of their own. `ai_pick_lineup_available`/`validate_lineup`/`effective_player_lineup` all read `available`/`is_suspended` |
| `commands` | `Command` enum, `step()` — validates a proposal and produces the events for it; `player_match_preview()` — a pure query, re-deriving the same lineup selection and RNG stream `advance_matchday` is about to use, for live-viewing the human's own fixture before it's recorded. `Command::SubmitTransferDecision` (§10) runs `validate_transfer_decisions` (submit-time shape only: targets exist, aren't already owned, prices aren't negative, sell targets are the club's own) before recording `Event::TransferDecisionSubmitted` — affordability is resolve-time, inside `market::filter_affordable`, not here. `dev_ticks_between` returns its compounded working `World` alongside the events, so `transfer_window_events` (fired from `advance_matchday` on a §7 boundary crossing) resolves against this advance's developed attributes and finance deltas, not the pre-tick world, passes `Some(state.player_club)`/`state.pending_transfer_decisions` through to `resolve_window`, and emits `Event::TransferWindowClosed` for every crossed boundary regardless of outcome so a pre-committed plan expires on schedule; `season_start_date` derives the season's kickoff from `state.date`/`current_matchday` rather than storing it. `lineup_conditions` (T9) builds `play_match`'s `conditions` map from `GameState::condition` for both call sites. `validate_lineup` (T10 §14, T11 §15) rejects a lineup naming an unavailable (injured or suspended) player (`CommandError::PlayerUnavailable`), and (T12 §16) validates `bench` (`≤ BENCH_SIZE`, in-squad, no duplicates with itself or the starters — `CommandError::BenchTooLarge`) and every `sub_plan` rule's named players against the union of starters and bench (`CommandError::UnknownSubPlanPlayer`); `effective_player_lineup` skips a remembered lineup that has gone stale, falling back to `ai_pick_lineup_available` with `state.suspended_players()`; `advance_matchday`/`player_match_preview` compute the suspended set once and thread it through every `ai_pick_lineup_vs` call for AI-controlled sides |
| `session` | `Session` — owns the log + folded state, routes commands, notifies observers; `save_log`/`load_log` (JSON-lines) |
| `observer` | `EventObserver` trait, `SeasonTelemetry` — passive event-stream consumers (trace/telemetry spine) |
| `news` | The R2 notification Trace: `NewsItem`/`NewsKind`/`EventRef`/`Audience`, `NewsRenderer` + `TemplateRenderer`, and `NewsObserver` (`EventObserver` for event-derived news; `check_conditions(&GameState)` for state-condition news; `observe_rejected_bids` for `WindowOutcome`'s Trace). Maintains small incremental indices (fixture→clubs, squad membership, each player's/club's most recent contract/finance/squad-affecting `EventRef`) purely from events already seen, so `check_conditions` — which only ever sees `&GameState`, never the log — can still attach real provenance to a state-condition item. `warned_*` sets make every state-condition check edge-triggered (fires once per newly-true condition, re-arms on recovery) so a season-long inbox stays bounded rather than repeating the same fact every call. Not wired into `commands.rs`/`session.rs`/`fforge-game` yet — a self-contained module + test suite, by explicit scope fence |
| `match_engine` | Phase-2a engine, now tactics-, consistency-, condition-, injury-, foul/card-, substitution-, and rating-aware (`TACTICS_MODEL.md` §6-§7, `MATCH_MODEL.md` §17/§13/§14/§15/§16/§18, T6-T13): `play_match(world, home, away, rng, consistency_rng, injury_rng, foul_rng, k: &Knobs, conditions: &BTreeMap<PlayerId, f64>, today: GameDate)` — unchanged signature since T12; `home_lineup.bench`/`.sub_plan` are read internally, so an empty `Lineup` (the identity) needs no special-casing at any call site — (`MatchOutcome { home_goals, away_goals, stream }` plus the §12 boundary fields — `injuries` (T10), `cards` (T11), and `ratings` (T13) now populated, `minutes` a flat 90 per starter except a sent-off or substituted player's, which stops at his departure minute, and an entering substitute's, which starts at his entry (T12); `consistency_rng`/`injury_rng`/`foul_rng` are each their own stream, `k` lets callers pin `consistency_sigma_max`/`injury_rate`/`foul_rate` independent of the production defaults — the T5/T6 identity tests' seam; `conditions` is a pre-computed per-player map — `GameState::condition` output, since the engine itself has no `GameState` — absent-from-map defaulting to `1.0`, the identity; `today` only feeds the ambient injury channel's age term), `lineup_strength`, `ai_pick_lineup` (XI selection, always neutral tactics, no availability filter — but since S1b it *does* fill a real `bench`/`sub_plan` via `ai_pick_bench_and_plan`, so a caller wanting the substitution identity must clear both explicitly, as both golden tests now do), `ai_pick_bench_and_plan` (`MATCH_MODEL.md` §16's v1 policy: a GK-reserved bench slot, then a role-spread pass, then CA fill to `BENCH_SIZE`; forced-cover rules pairing each slot role's weakest starter with its strongest tagged backup, ≤2 fatigue rules, and two tactics-only chase/hold rules — pure, RNG-free, and deliberately *not* score-conditioned on player selection, which the `SubCondition` vocabulary cannot express), `ai_pick_lineup_available` (T10/T11: the same greedy fill over only the squad available as of `today` and not in the caller's `suspended: &BTreeSet<PlayerId>` — falls back to the unfiltered squad below `XI` available, a defensive floor), `ai_pick_tactics`/`AiTacticKnobs` (the §7 policy — implemented, tested, but gated), `ai_pick_lineup_vs` (the real call-site: availability/suspension-filtered XI + tactics, tactics applied only when `AI_TACTICS_ENABLED` — **now `true`** since T7-R2 cleared `TACTICS_MODEL.md` §9 items 6 and 7; every real AI match now plays `ai_pick_tactics`'s choice rather than `Tactics::neutral()`). Submodules: `zone` (five-zone state + role→zone presence table), `knobs` (the fitted `Knobs` table, now including Consistency's T8 fields, Injuries' T10 fields, and Fouls' T11 fields — `foul_rate`, the p_foul logit's aggression/composure/press/fatigue terms, and the severity bands including the decoupled `foul_second_yellow_base`), `contest` (attribute→contest maps, the logistic resolver, fatigue — `fatigue_mult` takes a per-side `press_mult` and a `condition` scaling its minute-0 starting point, T9), `tactics` (§3's per-side `SideEffects` resolution — pure, RNG-free, computed once per match; `b_pass_delta_by_zone` and `Pressing::High`'s zone-profiled bias are the T7-addendum fixes, §5; the `advance_mult`/`opp_mid_advance_mult` magnitudes are T7-R's re-fit, and Mentality's `opp_mid_advance_mult`/`opp_penetrate_mult` concession terms are T7-R2's (§9 item 7) — those two fields now have two tenants each, Pressing and Mentality, and stack — see §3's lever-class note: an advance-class multiplier moves ~4× the absolute probability a logit-class bias does, so re-fit the advance-class lever first), `resolve` (the possession loop; `select_action`/`step`/`take_shot` all read `SideEffects` alongside `Knobs` and still only ever take a borrowed `&[XiPlayer]` — unchanged by T12, since `simulate` now owns `home`/`away` as mutable `Vec<XiPlayer>` and only mutates a slot *between* segments, at a decision point; `build_xi_player` (T12: the per-player body factored out of `build_xi`) draws one XI/bench player's Consistency multiplier, attaches his `condition` from the caller's map, and pre-rolls the ambient injury channel — one Bernoulli trial with a uniformly-drawn onset minute, T8/T9/T10 — shared by `build_xi` (starters) and the new `build_bench` (T12), so every dressed player is drawn in the same fixed order regardless of whether he ever enters; `maybe_contact_injury` rolls the contact channel inline at a failed take-on or a headed shot's aerial duel; `fire_due_ambient_injuries` fires a pre-rolled ambient injury once the clock reaches its onset; `impairment_mult` scales an injured player's effective attributes for the rest of the match, identity `1.0`; `maybe_foul` (T11) rolls the foul check after every `TakeOn` resolution and a failed non-`Def` `Pass`, overriding to a non-mirroring turnover and resolving a `Card` severity on a hit; `on_pitch`/`current_gk` (T11) let `sample_by_presence`/`team_means` renormalize over a shrunken XI once a red card fires, `team_means` recomputing per-tick only once a side has actually lost a player; `evaluate_decision_point`/`condition_holds` (T12) walk a side's `sub_plan` at half-time, the fixed `SUB_CHECKPOINTS` `[60, 70, 80]`, or forced immediately when a tick's injury/card count grows, matching each rule's `SubCondition`s against already-resolved match state and executing `Substitute` (swaps an XI slot, consumes a bench entry, capped at `MAX_SUBSTITUTIONS`, records the departed player's minutes before his slot is overwritten) or a tactics-change action — all RNG-free), `ratings` (T13: `compute_ratings`/`RatedPlayer` — a pure fold over the resolved `stream` plus final minutes/score into `MatchOutcome.ratings` per §18's delta table, base 6.0, clamped `[3.0, 10.0]`, called once at the end of `simulate` after the stream is fully built), `stream` (`MatchEvent` schema + commentary rendering, now including `Foul { card: Option<Card> }` (T11) and `Substitution { player_out: PlayerId }` (T12)), `golden` (`#[cfg(test)]`: the batch-3 T5 pinned Phase-2a baseline every 2e identity invariant asserts against — `identity_2e_knobs` now pins `consistency_sigma_max: 0.0`, `injury_rate: 0.0`, and `foul_rate: 0.0`), `calibrate` (`StreamTelemetry`'s per-zone pass-completion and turnover-won-by-zone cuts, plus T11's `fouls`/`yellows`/`reds` counters and their per-match/per-team rate accessors; `run_head_to_head` — the §7 triangle harness; stays at the condition identity, §13's status note; the T7-addendum zone-profiling test also pins `injury_rate: 0.0`/`foul_rate: 0.0` to isolate pressing's own effect; T7-R adds `SquadProfile`/`SQUAD_PROFILES`/`apply_squad_profile` — CA-preserving ±`PROFILE_SHIFT` mirror shifts that change a squad's *shape* without its level — and `run_squad_conditional_probe`, which pools those profiles' forced-tactics head-to-heads **across worlds** and retains the per-world argmax, since a single-world read of this axis flips on noise) |
| `development` | Phase-3 growth engine (`DEVELOPMENT_MODEL.md` §2–§5): the `DevKnobs` table (sibling of `match_engine::Knobs`), the per-category age envelope, PA-scaled targets, `resolve_bloomer_phase`/`resolve_dev_profile`/`resolve_coaching` (worldgen edge — `resolve_bloomer_phase` is split out so `worldgen::gen_player` can draw `φ` *before* seeding attributes with it and then hand the same value to `resolve_dev_profile`, never two draws), and `tick_changes` — the growth math producing a `DevelopmentTick`'s resolved deltas. `tick_changes_with_clip_stats` is its instrumented sibling (identical output plus per-player `ClipStats`), harness-only, backing `career_arc`'s `max_step`-saturation reading (`DEVELOPMENT_MODEL.md` §8.6). **The `DevKnobs` defaults are the post-W4 re-fit** (`k_dec` 1.0, `plast_*` (31.25, 4.6), `e_sigma`/`e_min` (0.095, 0.61)) and are calibrated *against envelope-consistent seeding* — they are not valid for the pre-W3 world and should not be read as historical. The per-attribute rate law is factored into `attr_rate`, shared verbatim with `valuation`'s projection so there is one law (no second integrator to drift). `phys_lmax(knobs, professionalism, natural_fitness)` (T9, `MATCH_MODEL.md` §13/R8) is the same discipline applied to the physical-aging term: a Professionalism/Natural-Fitness blend weighted by `DevKnobs::aging_prof_weight` (identity `1.0`, production `0.5`), shared by `tick_changes` and both of `valuation`'s projections. All RNG/math lives here; `apply` only integer-adds via `apply_attr_step` |
| `valuation` | Phase-4 centralized value function (`TRANSFER_MODEL.md` §2): `value` / `value_all` (the §2.7 per-window `BTreeMap<PlayerId, Money>` cache), `project_ca` (runs `development::attr_rate` forward, jitter off, minutes/coaching neutral), `project_ca_batch` (many players, one shared knob-derived `DevTables` — `club_ai::observe`'s per-squad projection), the `ValueKnobs` §9 table (plausibility-picked, sibling of `DevKnobs`; gained `form_scale`/`form_bound`, T13), and `MarketContext` (bounded league-wide role scarcity, plus a per-player `form` multiplier since T13 — `from_world` gained a `recent_ratings: &BTreeMap<PlayerId, Vec<u8>>` parameter, `GameState.recent_ratings` verbatim or empty/neutral for a caller with none; `form_mult(pid)` reads `1.0` for anyone absent from it). `value_with` integrates each player's whole 0..=horizon_years trajectory in one pass (`project_ca_series`) rather than once per year — same numbers, no redundant re-integration of the shared prefix; multiplies `contract_mult * scarcity_mult * form_mult` into the base curve. A pure Layer-2 function — prices, never decides; no market/club-AI here (Phase 4 §5–§6) |
| `career_arc` | Phase-3 career-arc harness (`DEVELOPMENT_MODEL.md` §6): the development sibling of `match_engine::calibrate`. Drives the real worldgen + development-fold pipeline over many seeds × a decade-plus and reports the §6 metrics (peak ages, PA attainment + tail, veteran decline slopes, wonderkid hit/flop) with per-seed spread. `bin/career_arc` is the runner; `career_arcs_are_in_a_believable_ballpark` is the wide-band regression guard. Also owns the §8 measurement surface: `fit_pa_from_ca_age`/`_youth`/`_band` (the **NAIVE** attack on PA — the `residual_sd` that is §8.4's headline success criterion), `fit_pa_from_composites_age*` (the **COMPETENT** attack, fitting PA on the per-`DevCategory` composites plus age — what a scouting agent actually has, since `φ` shifts each category's envelope differently so the composite *ratios* partially decode it), the W1b projection (`run_career_arc_with_projection`/`print_seeding_projection`/`le18`), and `max_step_saturation_16_band` (§8.6's escalation reading — it reconstructs the real `Session`'s exact tick inputs from its own `MatchPlayed`/`DevelopmentTick` events and `debug_assert_eq!`s against the recorded changes, so a reconstruction bug fails loudly instead of mismeasuring). **Open finding:** the NAIVE/COMPETENT gap is ~0 at ages 16–18 — the very population wonderkid scouting is about — and only opens up at 19–21; PA is fog with little skill in it there, which Phase 5's decision-quality axis needs to reckon with. Harness plumbing, never fed back into `DevKnobs` by itself — the re-fit is a human reading the numbers |
| `finance` | Phase-4 finance tick (`TRANSFER_MODEL.md` §4): `finance_deltas` resolves monthly revenue (∝ `Club.reputation`) minus the monthly share of committed wages into per-club deltas; `FinanceKnobs` (plausibility-picked, sibling of `DevKnobs`/`ValueKnobs`). RNG-free — both inputs are already-resolved world state, unlike `tick_changes`'s jitter. `commands::dev_ticks_between` calls it on the same 30-day boundary crossing `DevelopmentTick` fires on, emitting `Event::FinanceTick` alongside it |
| `pool` | Phase-4 player-pool lifecycle (`TRANSFER_MODEL.md` §8): `summer_pool_events` — one `YouthIntake` per club with roster headroom (reusing `worldgen::gen_player` with a 16-18 age band, quality anchored on `reputation` × `coaching_milli`), then every qualifying `PlayerRetired` (age ≥ `min_retirement_age` and best-role CA below `relevance_floor`, or a full season unsigned via `GameState::unsigned_since`). `PoolKnobs` (plausibility-picked, sibling of the others — but re-tuned against a real 15-season run, not left at a naive guess: the aging law lets CA plateau rather than crash, so a too-low floor leaves veterans immortal, squads permanently full, and mean age climbing unchecked). Intake is capped to `squad_max` headroom so it can never walk a club through the market's own hard squad-bound stabilizer. `commands::transfer_window_events` calls it only on the summer (even) window index, before `market::resolve_window`, so new prospects are on the books and retirees are already excluded from valuation when the clearing loop runs |
| `rng` | Seeded xoshiro256** + `derive_stream` — the crate's only source of randomness |
| `schedule` | `double_round_robin()` — deterministic fixture generation |
| `worldgen` | `generate()` — seeded new-game world/schedule/start date, recorded once into `GameStarted`. `gen_player` implements `DEVELOPMENT_MODEL.md` §8.1's **envelope-consistent seeding** (W3): PA is drawn *first*, anchored on club quality, then every attribute is seeded on `(PA/NORM)·env_c(age − φ)` beneath that ceiling, plus the pre-existing seeding noise — **calling the growth engine's own ceiling function at generation time, never re-encoding the envelope**. `SeedTables` (`EnvTables` + `norms_by_role` + `role_ceiling_consts`, built once per `generate`/cohort call and shared across every player) is the seam that guarantees it; `pool::youth_cohort` threads the same struct. `youth_discount` and `headroom` no longer exist. The bloomer phase `φ` is drawn here via `development::resolve_bloomer_phase` and passed into `resolve_dev_profile`, so seeding and development read the **same** `φ` rather than two independent draws. Changing any of this re-rolls every world — expect re-pinned golden baselines and `fforge-game` snapshots, which is a re-roll, not a re-fit |

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