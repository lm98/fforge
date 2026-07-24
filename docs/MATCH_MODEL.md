# Match Model — Phase 2a Design Note

The design record for the event-based possession match engine (`DESIGN.md` §4.1, Phase 2). It pins
the decisions reached in prototyping and the reasoning behind them, in the same spirit as `DESIGN.md`
and `ATTRIBUTE_SCHEMA.md`: a living artifact to reference and extend. The companion
`match_model_prototype.ipynb` is the throwaway shape-finder these decisions were fitted in; **this
note is the thing that survives it**, and the structure below is what drops into Rust `fforge-core`.

---

## 1. Purpose & status

- **Status:** Phase 2a — *ported, calibrated, and guarded.* Model shape settled in a Python
  scratchpad (per `DESIGN.md` §8, discarded after use, never a port target); `fforge-core::match_engine`
  is the Rust transcription (`play_match`), and `fforge-core::match_engine::calibrate` +
  `bin/calibrate` re-ran the calibration against real `worldgen` + `ai_pick_lineup` (not the
  notebook's synthetic squads), re-fit `b_beat` against the real attribute distribution, and
  pinned the result behind the `favourite_discrimination_regression_guard` regression test (§10).
- **In scope (this pass):** open play across five pitch zones, **including the wide route**
  (crossing, headers, cutbacks). Fatigue, a home-advantage edge, and a per-step clock.
- **Deferred to Phase 2e** (behind the same call site, no structural change): tactics as
  transition-matrix modifiers, cards & fouls, injuries, set pieces, substitutions, and the
  character/hidden attributes (Consistency, Determination, …). *The 2e design is now drafted:
  tactics in its own note (`TACTICS_MODEL.md` — a genuinely new subsystem), everything else as
  §11–§18 below (additions to this model, so they live in the note that pins it).* The wide
  route is included now
  precisely because it activates **Crossing, Heading, Jumping, and Command of Area** — four
  attributes that are dead weight in a central-only engine — and brings the entire aerial game and
  the goalkeeper's cross-claiming into the first version.

## 2. State space

A possession process over **`(possession, zone, clock)`**, in the tradition committed to in
`DESIGN.md` §4.1 (event-based, not Elo/Poisson, not full spatial).

- **Possession:** which side has the ball (home / away).
- **Zone — five, from the possessing team's frame:**

  | Zone | Meaning | Primary business |
  |---|---|---|
  | `Def`  | own defensive third | build-up |
  | `Mid`  | middle third | progression |
  | `AttC` | attacking third, central | through balls, long shots |
  | `AttW` | attacking third, wide | crosses, cutbacks |
  | `Box`  | the penalty area | shot resolution (a *point*, not a dwell) |

  The attacking third splits laterally into `AttC` / `AttW` — the whole reason the wide route exists
  as first-class geometry. `Box` is not dwelt in: an edge that reaches it resolves a shot
  immediately (**arrival = chance**), with rebounds handled as inline follow-ups.
- **Clock:** each possession step advances `δ` sim-minutes (a knob); the match runs two 45-minute
  halves, each kicked off by the appropriate side. Fatigue accumulates against the clock.

## 3. Transition graph

Dwelling zones are `Def`, `Mid`, `AttC`, `AttW`; reaching `Box` resolves a shot. Every edge is the
success/failure branch of a resolved contest (§4).

| Zone | Actions | Success → | Failure → |
|---|---|---|---|
| **Def**  | Pass (build-up) | `Mid` \| retain `Def` | Turnover → opp `AttC` (lost deep = dangerous) |
| **Mid**  | Pass, TakeOn | advance to `AttC` **or** `AttW` (lateral split) \| retain `Mid` | Turnover → opp `Mid` |
| **AttC** | Pass (through), TakeOn, Shot (long) | `Box`[finish] \| recycle to `Mid`/`AttC` \| (long shot resolves in place) | Turnover → opp `Def` |
| **AttW** | Cross, TakeOn, Pass | `Box`[header] via cross · `Box`[finish] via cutback \| cut inside to `AttC` \| recycle | Turnover → opp `Def` |
| **Box**  | Shot (type set by arrival) | Goal \| Save (parry→rebound / catch→opp `Def`) \| Off/Block → opp `Def` | — |

**Turnover mirroring** encodes counter-attacks: possession flips and the winner restarts in the
mirrored zone — lose it in your `AttC`/`AttW` and the opponent starts deep in *their* `Def`; lose it
in your own `Def` and they win it high in *their* `AttC`. The **lateral split** out of `Mid`
(`p_wide`) is where a future *width* tactic plugs in unchanged.

## 4. Resolution model — actor-centric sampled *(c)*

The committed resolution model (chosen over whole-team aggregate and aggregate-with-cosmetic-names):
every contest samples the two players who matter and resolves them head-to-head, so **stars matter
causally** and the "world-class striker starved of service" case emerges structurally rather than by
fudge factor.

1. **Sample the on-ball actor** from the possessing XI and the **primary defender** from the
   defending XI, each weighted by the **role→zone presence table** (§6).
2. **Score each side** as a weighted mean of the contest's attributes (`ATTRIBUTE_SCHEMA.md` §6),
   fatigue-scaled, blended with a light **team support term**.
3. **Resolve** with one logistic-of-difference, shared by every open-play contest:

   ```
   p(success) = σ( k · (atk − def) / s  +  b_action  +  home_bias·[home attacking] )
   ```

   `k` sets attribute-difference sensitivity, `s` normalizes the 0–100 range, `b_action` is the
   per-action base rate, and `home_bias` is an additive edge applied to the home side's attacking
   contests.

- **Attribute maps** are transcribed from `ATTRIBUTE_SCHEMA.md` §6 — e.g. a Pass pits the passer's
  Passing/Vision/Decisions/Ball Control/Composure against the defender's Def.
  Positioning/Marking/Decisions/Speed/Aggression/Work Rate. This is the schema's no-orphan coverage
  map finally *consuming* every performance attribute.
- **Support term** (`support` ∈ 0–1): each side's score is `(1−λ)·actor + λ·team_mean`, where
  `team_mean` is the XI's mean weighted attribute for that contest (precomputed once per match).
  Small by default — the actor dominates, the team quality nudges. This is the cheap form of the
  "interaction effects" `DESIGN.md` §4.1 calls for.
- **Fatigue:** an effective-attribute multiplier `1 − drop`, where `drop` grows with match minute and
  is larger for low-Stamina, high-Work-Rate players — they fade late, modulating everything as
  `ATTRIBUTE_SCHEMA.md` §6 #11 requires.

## 5. The wide route: crosses & shot types

- **Cross is the one two-contest action** (a delivery and an aerial are genuinely distinct events,
  both narratable):
  1. **Delivery** — crosser's Crossing/Vision vs the defender's Def. Positioning/Marking cutting it
     out. Fail → cleared (turnover).
  2. **Contested header → outcome** — a box-arriving attacker (sampled by `Box` presence: ST/AM/W)
     heads it, with the **aerial duel folded into the header shot's defensive side**: the marking
     defender's Heading/Jumping/Marking/Strength *and* the goalkeeper's **Command of Area**. This is
     where the keeper earns his cross-claiming job.

  Together these two contests implement `ATTRIBUTE_SCHEMA.md` §6 #5 (*Cross → box*), which the schema
  already writes as a two-stage contest ("Crossing, Vision · then Heading, Jumping, …") — the aerial
  duel (§6 #7) is the defensive half of stage two rather than a separate resolved step.
- **Shot type is set by how the ball arrived**, selecting both the attacker's attributes and a base
  chance-quality `q`:

  | Arrival | Type | Attacker attributes | Base quality |
  |---|---|---|---|
  | Through ball (`AttC` pass) | finish | Finishing/Composure/Ball Control/Off-the-ball | high |
  | Dribble into box (`AttC`/`AttW` take-on) | finish | (as above) | modest |
  | **Cutback** (`AttW` take-on to byline) | finish | (as above) | **highest** — cutbacks are lethal |
  | **Cross** (`AttW`) | header | Heading/Jumping/Strength/Composure/Off-the-ball | modest |
  | Long shot (`AttC`) | finish | (as above) | low, resolves in `AttC` |

- **Shot resolution** is two chained sigmoids so the stat line is rich: **on-target** (shot vs
  blockers) then **beat-the-keeper** (shot vs GK Reflexes/Handling/Positioning), the arrival quality
  `q` added into both. A save parries to a scrappy rebound with probability `p_rebound` (an inline
  follow-up shot), else the keeper collects.

## 6. Role → zone presence table (the new design-once artifact)

A **new** table, distinct from the role→attribute weighting of `ATTRIBUTE_SCHEMA.md` §5 (which rates
*attribute importance*; this rates *spatial presence*). It answers "who is on the ball / defending
here?" and drives all actor/defender sampling. Starting estimates below — the notebook tunes them,
and later they become **formation/tactics-derived** rather than fixed.

**Attacking presence** — how often a role is the on-ball actor in a zone:

| Role | Def | Mid | AttC | AttW | Box |
|---|:-:|:-:|:-:|:-:|:-:|
| GK | 5 | 0 | 0 | 0 | 0 |
| CB | 4 | 1 | 0 | 0 | 0 |
| FB | 3 | 3 | 1 | 3 | 0 |
| DM | 3 | 4 | 1 | 0 | 0 |
| CM | 1 | 4 | 3 | 1 | 1 |
| AM | 0 | 3 | 4 | 2 | 2 |
| W  | 0 | 2 | 2 | 5 | 2 |
| ST | 0 | 1 | 3 | 1 | 5 |

**Defensive presence** — the primary challenger when the opponent attacks a zone:

| Role | Def | Mid | AttC | AttW | Box |
|---|:-:|:-:|:-:|:-:|:-:|
| GK | 0 | 0 | 0 | 0 | 3 |
| CB | 1 | 1 | 4 | 2 | 5 |
| FB | 1 | 2 | 2 | 5 | 3 |
| DM | 2 | 4 | 3 | 1 | 1 |
| CM | 2 | 4 | 2 | 1 | 0 |
| AM | 2 | 2 | 1 | 1 | 0 |
| W  | 3 | 2 | 1 | 2 | 0 |
| ST | 4 | 1 | 0 | 0 | 0 |

The sampling that falls out is the intuitive one: wingers on the ball wide, the full-back as their
primary marker with a covering centre-back, strikers and centre-backs contesting the box, forwards
pressing a deep build-up.

## 7. Determinism & the Rust seam

The model is a **pure function of `(lineups, world, seed)`** — the property `DESIGN.md` commits the
whole architecture to. The Rust shape:

```rust
fn play_match(world: &World, home: &Lineup, away: &Lineup, rng: &mut Rng) -> MatchOutcome
// MatchOutcome { score: (u8, u8), stream: Vec<MatchEvent> }
```

- **The fold boundary does not move.** Only `MatchPlayed`'s score (`home_goals, away_goals`) is the
  match engine's fold output (as today) — the crude engine is swapped for this one behind the same
  call site in `commands::advance_matchday`, and replay never re-simulates (`event.rs`'s
  record-outcomes-not-inputs rule). The RNG is the existing per-fixture derived stream
  (`derive_stream(seed, FIXTURE_STREAM_NS | fixture.id)`).
- **`MatchPlayed` also records the two XIs (`home_xi`/`away_xi`) — for a *different* consumer, not
  the match engine.** Phase 3 development needs each match's participants as its playing-time signal,
  so the resolved lineups ride in the recorded event (the same record-the-outcome rule the score
  follows — see `DEVELOPMENT_MODEL.md` §3, "playing-time data source"). The match engine neither
  reads them back nor changes because of them; they are appearances first-class, folded by
  `GameState` into the per-tick window development consumes. The stream boundary below is unaffected.
- **The minute-by-minute stream is a Trace, not a fold input** (the decision reached this session):
  it rides alongside via an `EventObserver`, persisted to SQLite **only for matches that matter**
  (the human's matches now; journalist-agent matches later) — bulk AI matches store the score only.
  The rule: *never re-derive an authoritative stream* — persist at play-time or omit. Calibration
  re-runs the engine freely (drift is fine there); authoritative replay reads the persisted record
  (drift is impossible).

## 8. Calibration

The harness (`DESIGN.md` §4.1) checks emergent aggregates against reality. Knobs are grouped in the
notebook's `Knobs` dataclass; the fitted starting point and its readings:

| Aggregate | Fitted reading | Target |
|---|---|---|
| Goals / game | ~2.7 | ~2.6 |
| Home / draw / away | 41 / 28 / 31 % | ~45 / 26 / 29 % |
| Shots / game | ~28 | ~25 |
| Shots on target | ~35 % | ~33 % |
| Conversion | ~10 % | ~10 % |
| Wide-origin goals (cross + cutback) | ~25 % | 25–35 % |
| …of which headed | ~19 % | 15–20 % |
| Home possession | ~55 % | > 50 % |

**Primary levers:** `delta` (tempo → shot volume); the `p_*` transition splits (how often a
possession reaches the box); `b_beat` / the `q_*` arrival qualities (conversion); `home_bias` (the
home edge); `p_wide` + the cross/cutback rates (the central-vs-wide goal mix).

**Calibration lesson worth banking (surfaced in prototyping):** a *single* synthetic league is a
**noisy estimator** — goals/game swings by ±0.4 purely on which league is drawn, while H/D/A stays
stable. **Calibrate on the mean pooled over many league draws**, and watch the per-league spread the
report prints; never tune against one league. (This is a synthetic-data artifact of small leagues
with random squads, not a model defect — but the Rust harness must average the same way.)

**Rust harness result (the deliverable this section deferred):** `fforge-core/src/match_engine/calibrate.rs`
(`StreamTelemetry`) + `fforge-core/src/bin/calibrate.rs` (`cargo run --bin calibrate -- --seeds N`) now run
this table against real `worldgen` + `ai_pick_lineup`, not the notebook's own synthetic squad
generator. Diagnosis (pooled, 12+ seeds): the resolution loop is a faithful port (`resolve.rs`'s
`notebook_parity` test reproduces ~2.5-2.9 gpm on notebook-equivalent inputs run through the same
loop) and shots/game, on-target rate, and wide-origin share all landed on target against real
inputs — but conversion sat at ~7% against real `worldgen`'s attribute distribution, versus the
notebook's own ~10%. Re-tuning `b_beat` (-1.7 → -1.05, the beat-the-keeper stage only — it doesn't
touch on-target rate or shot volume, confirmed by sweep) closes it: goals/game ~2.5-2.6, H/D/A
~43/26/31%, conversion ~10%, wide-origin share ~27%, all pooled over 16 seeds. `knobs.rs`'s default
`Knobs` now reflects this real-`worldgen`-calibrated point, not the notebook's original fitted
values verbatim (`b_beat` is the one field that differs; see `knobs.rs`'s doc comment).

## 9. Event-stream schema mapping

The action alphabet **is** the stream's event-kind alphabet — the humble text match view of Phase 2
(the forcing function for stream richness, `DESIGN.md` §9) prints exactly these beats, and the four
downstream consumers (commentary, stats, journalist agent, future viewer) read them:

- `Pass`, `TakeOn`, `Cross` (first-class, carrying delivery outcome), `AerialDuel`, `Clearance`,
  `Turnover`.
- `Shot { kind: Finish | Header | LongShot, source: Through | Dribble | Cutback | Cross | Long,
  outcome: Goal | Saved | Off | Blocked }` — `kind` is what's narratable for commentary (headed vs
  long-range); `source` is the finer arrival route `kind` collapses (through-ball, dribble, and
  cutback all share `Finish`), and is what makes the wide-origin-goal-share metric (cross + cutback,
  §8) actually computable, not just headed-goal share. A rebound follow-up keeps the `source` of the
  shot that created it.
- `Goal`, `Save` (with parry/collect), and zone-entry context so a beat can say *where* on the pitch
  it happened.

Designing this schema for narratability now (not just outcomes) is the cheap-as-a-decision /
expensive-as-a-retrofit call from `DESIGN.md` §9 — the same shape as the narrative-feedback rules.

## 10. Open sub-questions

Deliberately unresolved, to settle during the Rust port or Phase 2 calibration:

1. **Presence table → formation coupling.** *Partially settled.* The raw per-role presence numbers
   (§6) stay global and unedited — reinventing them per formation would be new shape-finding work
   this doc reserves for real calibration, not a mechanical Rust addition. Instead
   `resolve::formation_p_wide` derives each side's effective `p_wide` from its own XI's actual
   `AttC`/`AttW` presence share (using the existing table), scaled relative to the reference shape
   `p_wide` was fitted against — a lineup shaped like the reference gets the fitted constant back
   exactly; a winger-less 3-5-2 routes less into `AttW`. Measured effect (`calibrate.rs`,
   12 seeds/4560 matches): pooled gpm moved by <0.01 — real but small. The rest of the presence
   table (who resolves a contest once a zone is reached) is still global; deriving *that* per
   formation, if ever warranted, remains open.
2. **Support-term weight (`support`).** Kept small; whether it should scale with zone (more team
   dependence in build-up than in the box) is a calibration-taste question.
3. **Long-shot home for the action.** Currently an `AttC` action resolving in place; whether `AttW`
   should also permit a (worse) shot is a texture question, not structural.
4. **Between-league variance.** Acceptable as a synthetic-data artifact, but the Rust calibration
   harness must pool over league draws — flagged so it isn't silently defaulted to one league.
5. **`Box` as point vs dwell.** The prototype resolves on arrival; if second-phase box play
   (knock-downs, scrambles) earns its keep, `Box` could become a shallow dwell zone. Deferred.
6. **Bookmaker-baseline check implemented.** `DESIGN.md` §4.1 lists *favourite win-rate vs
   bookmaker-implied probabilities* as a calibration axis. There are no real odds in a synthetic
   world, so the harness compares against a **reference win-probability curve** instead: draws mean
   `E(win) ≠ E(points)`, so the comparison is against **expected points share**
   `(wins + 0.5·draws)/matches`, not P(home win). `StreamTelemetry::record` (`calibrate.rs`) bins
   each match by `home_strength - away_strength` (`lineup_strength`, ~2-CA-point bins) and tracks
   per-bin match/win/draw/loss counts; `expected_points_curve` reports the empirical curve.
   `calibrate::elo_expected(gap, s)` is the reference — the Elo expected-score curve, with
   `ELO_SCALE_S = 40` a documented, plausibility-picked constant (not fitted); `score_against_
   reference` reports per-bin deviation plus max/mean-weighted deviation. This is a
   **discrimination** check (does the favourite-vs-underdog slope look sane) — it validates the
   *slope*, not the *intercept*; the home-advantage *level* is already covered by the H/D/A
   aggregate. `bin/calibrate.rs` prints the table; `favourite_discrimination_regression_guard`
   (`calibrate.rs`, pooled over 8 seeds) is the regression guard, asserting monotonic
   non-decreasing expected points and a bounded max deviation from the reference — a sibling to the
   goals-per-match sanity band, not a precise-fit assertion.

---

**Phase 2e extensions (drafted).** Everything from here down is the Phase-2e design draft — **status: drafted, pre-implementation**,
unlike §1–§10's ported-calibrated-guarded. Sections are appended after §10 (rather than
renumbering it) because §10 is referenced by name from code comments and the other notes.
Tactics is *not* here: it is a new subsystem with its own decision inventory and lives in
`TACTICS_MODEL.md`; the sections below are additions to the model this note already pins.

## 11. 2e scope, sequencing, and the two invariance regimes

**Set pieces stay deferred** (call it 2f): no 2e review item (R4–R10) needs them, and §15's foul restart
is deliberately abstracted (possession retained in zone) so free kicks/corners can later become
real contests behind the same seam without reworking fouls.

**Two invariance regimes govern the rollout, and confusing them would be expensive:**

1. **Bit-for-bit** — tactics only. `Tactics::neutral()` reproduces the 2a engine exactly, RNG
   draw sequence included (`TACTICS_MODEL.md` §4). This works because tactics resolution is
   draw-free and neutral is an exact identity.
2. **Band-and-re-fit** — everything else. Fouls, injuries, consistency, condition, and
   substitutions *necessarily add draws* and move outcomes; they cannot and should not promise
   bit-equality with 2a. Each lands knob-gated, states its predicted §8 movement in its section
   below, re-runs the harness, and takes a `b_beat`-style re-fit if its prediction misses. The
   regression guards (`aggregates_are_in_a_believable_ballpark`,
   `favourite_discrimination_regression_guard`) are re-pinned once per landing, never silently.

**Sequencing** (each step leaves the suite green and the §8 table annotated):

1. §12's boundary extension (pure plumbing — new fields empty, zero behavior change);
2. tactics at neutral, then `ai_pick_tactics` (per `TACTICS_MODEL.md` §8's rollout);
3. fouls & cards (§15), then injuries (§14) — both small, independent draw additions;
4. condition & recovery (§13) with the Natural Fitness split, then substitutions (§16), which
   consume condition;
5. character activation (§17), ratings (§18) last — ratings are a pure derivation over a stream
   the earlier steps enrich.

**RNG discipline for every added draw:** all new randomness comes from the *same* per-fixture
stream (`derive_stream(seed, FIXTURE_STREAM_NS | fixture.id)`), drawn inline where the
triggering event resolves — never from a second stream, never conditionally skipped in a way
that depends on impure state. New-draw counts may depend on match events (a foul draw happens
because a take-on resolved), which is fine: determinism is per-(inputs, seed), not
per-draw-count.

## 12. The extended `MatchOutcome` / `MatchPlayed` boundary (R6)

*Status: landed (sequencing step 1) for all three fields; `injuries` (§14, T10) and `cards`
(§15, T11) are now populated by their real models, `ratings` still emits empty pending §18.
`cond_drain` joins the struct when §13 lands, behind the same serde-default seam. `minutes`
landed early, ahead of §16 (batch handoff T4/§2.8): every starter recorded a flat 90, every
non-starter absent from the vec, until T11 made a sent-off player's minutes stop at his
dismissal — degenerate and inert (all four pooled calibration guards read unchanged) until
T10/T11/T12 make partial minutes possible; the playing-time input to development (below)
needed the field and its minutes-share redefinition in place well before substitutions do, so
landing it separately cost nothing and de-risked the larger §16 change.*

2e produces per-player consequences that outlive the match. The §7 rule (record outcomes; the
fold consumes without re-running engines) dictates the boundary: **every consequence that
mutates world state rides in `MatchPlayed` as a resolved value**; the minute-by-minute stream
stays a Trace.

```rust
MatchOutcome {
    home_goals, away_goals, stream,          // as today
    injuries:  Vec<InjuryOutcome>,           // player, days_out — resolved at match time (§14)
    cards:     Vec<CardOutcome>,             // player, Yellow | SecondYellow | Red, minute (§15)
    minutes:   Vec<(PlayerId, u8)>,          // true minutes, substitutions included (§16)
    cond_drain: Vec<(PlayerId, u8)>,         // per-player condition spent this match (§13)
    ratings:   Vec<(PlayerId, u8)>,          // 10×rating fixed-point, e.g. 68 = 6.8 (§18)
}
```

`Event::MatchPlayed` gains the same five vectors alongside `home_xi`/`away_xi`. All fold arms
remain pure integer/assignment operations; all RNG stays at match time inside `play_match`.

**The sanctioned 2e `fforge-domain` extension** (the `TRANSFER_MODEL.md` §3 pattern — named
here once so it isn't an open-ended license): `Tactics` + its four instruction enums and
`Lineup.tactics` (`TACTICS_MODEL.md` §2, §6), `Lineup.bench` (§16),
`Character.natural_fitness` (§13), and `Player.condition` / `Player.injured_until` (§13, §14).
Nothing else; suspensions in particular are `GameState` bookkeeping (derived, below), not
domain fields.

- **Why ratings are recorded rather than re-derived:** a rating is a pure function of the
  stream (§18), but the stream is *not persisted* for bulk AI matches (§7) — re-deriving would
  mean re-simulating, the exact thing replay never does. Future consumers (news now; the form
  multiplier `TRANSFER_MODEL.md` §2.5 deliberately deferred; Phase-5 morale) need it
  replay-safe, so it is an outcome.
- **Why suspensions are *derived*, never recorded — the derived-suspension rule.** A
  suspension is a rule applied to recorded cards: deterministic, RNG-free, engine-free. That
  puts it in the same class as `league_table` (points derived from recorded scores) and CA
  (derived from attributes — the domain's own hard constraint 1): cards are the truth,
  the ban is a view. Recording bans *and* cards would create two sources of truth that can
  disagree — the sync bug the CA rule exists to make impossible. Accepted consequence, same as
  the league table's: changing the suspension rule re-derives history on replay; that is the
  known cost of the derived class, and suspensions (like standings, unlike attributes) are
  ephemeral bookkeeping where it is the right cost.
- **Availability becomes a fold view:** `GameState::available(pid, date)` — false while
  `injured_until` lies ahead (§14) or a derived ban is unserved (§15). `validate_lineup`
  rejects unavailable players (new `CommandError` variants); `ai_pick_lineup` filters them;
  `effective_player_lineup` falls back to auto-pick when a remembered lineup goes stale. The
  playing-time input to development upgrades from XIs to `minutes` — the deepening
  `DEVELOPMENT_MODEL.md` §3 explicitly left room for, behind the same appearances window.
- **Log growth:** ~50 small entries per match against the ~22 `PlayerId`s already recorded —
  same order of magnitude, still linear in matches. No concern.

**§8 impact: none by itself** — plumbing only, landed with all vectors empty. New §8 rows are
introduced by the sections that fill them (cards/game §15, injuries/season §14, subs/match §16,
mean rating §18).

## 13. Condition & between-match recovery (R5) — and the Natural Fitness split (R8)

**Status: landed (batch-3 T9).** Implementation lives in the new `fforge_core::condition`
module: `ConditionKnobs` (plausibility-picked, sibling of `DevKnobs`/`ValueKnobs`) and a pure
`condition(recent, as_of, natural_fitness, age_years, k) -> f64` — each recorded appearance in
the caller-supplied `recent` slice leaves a decaying load debt (`drain_per_match`, cleared at
`recovery_per_day`, which scales up with Natural Fitness and down with age past
`age_anchor`); condition is `(1.0 − Σ debts).clamp(condition_floor, 1.0)`, and an empty
`recent` always reads exactly `1.0`. `GameState::condition(pid)` is the one call site that
turns the already-existing `recent_appearances` rolling window (§12's boundary extension,
landed ahead of this task) plus `Character.natural_fitness` and `Player::age` into a value —
no new fold state, no new event, per §2.3's "derived, never stored" rule.

**The identity mechanism differs from Consistency's.** Condition is `GameState`-derived, and
`match_engine` has no `GameState` — so unlike `Knobs::consistency_sigma_max` (an in-engine
knob), condition arrives from *outside* `play_match` as a plain `conditions: &BTreeMap<PlayerId,
f64>` parameter, threaded through `build_xi` into a new `XiPlayer.condition` field (not baked
into `attrs` like Consistency — it scales only `fatigue_mult`'s starting point, not every
attribute). A player absent from the map — including the whole map being empty — defaults to
`1.0`, which *is* the identity setting (§2.1): callers with no real `GameState` (every test,
`bin/calibrate`, the pooled harnesses) simply never populate the map. `fatigue_mult` gained a
`condition: f64` parameter; its new starting point at minute 0 is `condition` exactly (was
always `1.0`), fading further from there exactly as before — "a player at 85% begins the match
already part-fatigued," §13's own framing, realized as a single multiplication.

**Scope decision: `bin/calibrate` and the four pooled calibration guards stay at the identity
setting.** None of them tracks a `GameState` across a simulated season (they drive
worldgen → AI-lineup → `play_match` directly, one fixture at a time, discarding state between
matches — `bin/calibrate`'s own module doc comment), so there is no accumulated
`recent_appearances` window for them to read; wiring one in would be new season-tracking
machinery outside T9's scope, not a data-flow change. This means `bin/calibrate`'s pooled
readings are unaffected by this task by construction (verified: `cargo run --release --bin
calibrate -- --seeds 8` reads unchanged goals/match, and all four pooled guards
— `favourite_discrimination_regression_guard`, `career_arcs_are_in_a_believable_ballpark`,
`market_is_in_a_believable_ballpark`, `aggregates_are_in_a_believable_ballpark` — pass with no
threshold change). The real per-player effect is exercised end-to-end through
`commands::advance_matchday`/`player_match_preview` (both build a real `conditions` map via
`GameState::condition`) and asserted directly (`condition`'s own unit tests; `fatigue_mult`'s
extended bounds test; `build_xi_attaches_condition_from_the_caller_supplied_map`;
`a_tired_home_side_scores_visibly_fewer_expected_goals_than_a_fresh_one`, pooled over 300
seeds). A season-long pooled reading of condition's aggregate bite is left to a future harness
extension, not fabricated here.

**Aging blend (§2.4/R8) also landed, in `development`, not `match_engine`.** See
`DEVELOPMENT_MODEL.md`'s updated §3/§6 for the `Lmax_Phys` blend formula, the `aging_prof_weight`
knob (`w`, identity `1.0`, production `0.5`), and the `career_arc` check confirming no reading
moved outside its already-banked band at the production default.

**Condition** is a persistent per-player `0..=100` state, distinct from §4's *in-match*
fatigue: fatigue is the within-90' drop; condition is what you start the day with.

- **In-match:** the §4 fatigue multiplier gains a condition anchor — a player at condition `c`
  starts the match effectively `c/100`-scaled and fades from there. One formula change inside
  `fatigue_mult`, no new seam.
- **Drain (recorded):** condition spent = base per-minute cost × minutes played, scaled up by
  a High-press game plan (`TACTICS_MODEL.md` §3's fatigue coupling) and (slightly) down by
  Stamina. Resolved at match time → `cond_drain` in `MatchPlayed` (§12): it depends on match
  events, so it is an outcome.
- **Recovery (derived):** between matchdays, condition regenerates toward 100 as a pure
  RNG-free function of days elapsed, age, and **Natural Fitness** — so it is *derived in the
  fold* on date advance, the same class as suspensions (§12): the recovery law can never
  desync from recorded state because it isn't recorded state.

**The Natural Fitness resolution (R8) — decision: split it out, as a hidden Character
attribute.** `ATTRIBUTE_SCHEMA.md` §3 folded it away pending "a genuine second consumer";
`DEVELOPMENT_MODEL.md` §4 kept it merged for Phase 3 and set the revisit tripwire at exactly
this point. The tripwire fires:

- Recovery is now modeled, and **no existing attribute can carry it without double-dipping**.
  Stamina is the candidate — but Stamina already owns in-match fade, and one attribute owning
  both means "durable in a match" and "recovers fast between matches" become the same player,
  which deletes the squad-rotation decision this section exists to create: the player you'd
  rotate (excellent for 90', slow to recharge) becomes unrepresentable. Professionalism is the
  other candidate and is worse — it is a *training/aging* trait, and using it here would make
  every professional also physically resilient, one hidden number secretly running two systems.
- So: `Character.natural_fitness: Rating` — hidden (it drives no in-match contest, so it must
  never enter CA — the schema's own class rule), resolved at worldgen/youth-intake, generated
  with a modest positive correlation to Professionalism so existing worlds' character makeup
  stays plausible.
- **Deliberately narrow:** its only v1 consumer is the recovery law. `DEVELOPMENT_MODEL.md`
  §3's aging-resistance term **stays with Professionalism** — migrating it would force a
  Phase-3 re-fit for zero behavioral need; flagged as a possible later cleanup, not done now.

**Honesty note on when condition bites.** The calendar is strictly weekly (7-day matchday
steps), and a week recovers most players fully — so in v1, condition's bite is at the edges:
returning from injury below 100 (§14), aging players with low Natural Fitness starting in the
low 90s, and High-press squads accumulating a deficit. Its strategic payoff arrives with
fixture congestion (cups, continental midweeks — future), and the law is deliberately simple
until then. This is scoped-in *now* anyway because injuries and substitutions both consume it.

**§8 impact:** predicted pooled aggregates ≈ unchanged in a weekly calendar (gpm drift < ±0.05)
— confirmed exactly (not just "within noise") for `bin/calibrate` and all four pooled guards,
per the status note above, because that harness never accumulates the season-long
`recent_appearances` window condition reads; the real per-match bite is confirmed instead by
`a_tired_home_side_scores_visibly_fewer_expected_goals_than_a_fresh_one`'s pooled 300-seed
comparison. New telemetry rows (mean pre-match condition, the post-75' contest-success dip by
pressing level) are deferred to a season-tracking harness extension, not fabricated against a
harness that cannot see them.

## 14. The injury model

**Status: landed (batch-3 T10).** Implementation lives in `match_engine::resolve`: the contact
channel rolls inline at each failed take-on and each headed shot's aerial duel
(`maybe_contact_injury`); the ambient channel is pre-rolled once per player at kickoff in
`build_xi` (one Bernoulli trial approximating a per-minute hazard integrated over 90', with a
uniformly-drawn onset minute) and fired by `fire_due_ambient_injuries` as the possession loop's
clock reaches it — a deliberate simplification recorded as such at the call site, since the
target metrics below care about expected *count*, not the intra-match timing of a non-eventful
injury. Both channels draw from their own `INJURY_NS` stream (`play_match` gained an
`injury_rng: &mut Rng` parameter, identity `injury_rate: 0.0`); severity is drawn only when the
check itself fires — the same conditional-on-outcome shape `take_shot`'s on-target/beat-keeper
rolls already use, not a violation of the unconditional-fixed-order rule (reserved for whole-
population draws like Consistency's, not per-event narrative branches). `GameState::available`
(`MATCH_MODEL.md` §12) is the derived view `ai_pick_lineup_available`, `validate_lineup`
(a new `CommandError::PlayerUnavailable`), and `effective_player_lineup`'s staleness check all
read — squad depth finally bites, exactly as §2.5 named it.

**Mid-match handling today vs. eventually.** T10's scope fence is explicit: an injured player
*continues at reduced effectiveness for the remainder* (`impairment_mult`, identity `1.0`,
production `0.6`) — no substitution mechanism yet. The forced substitution described below is
T12's; until it lands, an injured player simply plays on impaired rather than being replaced.

Contest #9 of `ATTRIBUTE_SCHEMA.md` §6, finally consuming **Injury-proneness** (its no-orphan
promise). Two hazard channels, both drawn from the fixture stream at match time:

- **Contact:** evaluated on the contests that model contact — a failed take-on (the tackle)
  and the aerial duel inside a headed shot. `p_injury = base_contact × prone_mult ×
  intensity`, where `prone_mult` comes from hidden Injury-proneness (with a small
  Professionalism discount, per the schema's "aging/injury resistance"), and `intensity` rises
  with the tackler's Aggression.
- **Ambient** (muscle/overload): a small per-minute hazard scaled by `(1 − condition)` and
  age — the channel that makes playing a drained veteran a real risk and gives §13 teeth.

**Severity** is a categorical draw — `Knock` (0–3 days), `Minor` (1–3 weeks), `Moderate`
(4–8 weeks), `Severe` (3–6 months) — with probabilities skewed hard toward the small end.
Resolved **at match time** into `(player, days_out)`, recorded in `MatchPlayed` (§12) — the
severity model can evolve without rewriting anyone's medical history (the `DevelopmentTick`
argument verbatim). The fold sets `Player.injured_until`; availability (§12) does the rest;
a returning player re-enters below full condition (§13).

**In-match consequence:** an injured player triggers a forced substitution (§16); with no sub
available the side plays impaired (the slot's attributes scaled hard) or, past the limit,
short — presence sampling already tolerates a shrunken XI (§16).

**Targets:** ~1.5–2.5 match-missing injuries per club per season; a visible in-match injury
every ~4–6 matches; severe cases rare but real (a handful per league season).

**T10 finding — the first magnitude pick read high, one re-scale landed in band.**
`state::a_pooled_seasons_injury_count_lands_in_the_documented_band` drives real full seasons
through `commands::advance_matchday`, pooled over 6 seeds, and counts only "match-missing"
injuries (`days_out >= 7` — a `Knock`, 0–3 days, always heals inside the weekly calendar and
never actually costs an appearance, so it does not count toward this target). The starting
pick (`injury_base_contact: 0.006`, `injury_ambient_base: 0.0025`, both plausibility-picked
from `bin/calibrate`'s own per-match shot/take-on counts) read **3.88** match-missing
injuries/club/season — above the 1.5–2.5 band. Scaling both bases down by ~0.515 (to `0.0031`
/ `0.0013`) landed at **2.14**, inside it. A single-knob-family magnitude re-scale, the same
class of correction `career_arc`'s `K_DEC` re-fit was — not yet a full B3.9 fit against every
sub-target (the visible-injury-every-4–6-matches and severe-rarity reads are unverified; a
future task's job).

**§8 impact:** predicted gpm ≈ unchanged (< ±0.05 — injuries redistribute minutes, they don't
change contest math) — not directly re-verified here (`bin/calibrate` has no season-length
calendar to accumulate injuries across, so its per-fixture pooled reading only exercises
single-match injury draws, the same scope `favourite_discrimination_regression_guard` already
covers and still passes with the mechanism live). New telemetry rows (injuries/club/season,
mean matches missed) exist today only via the pooled test above, not yet in `bin/calibrate`'s
own report. Second-order: squad-depth quality starts mattering in season aggregates, which the
market harness (`TRANSFER_MODEL.md` §11) should see as slightly increased demand for depth —
flagged there, not yet measured.

## 15. Fouls, cards, and derived suspensions — resolving the discipline question

**Status: landed (batch-3 T11).** Implementation lives in `match_engine::resolve`: the foul
check rolls after every `TakeOn` resolution (either outcome) and after a failed `Pass` outside
the actor's own `Def` zone (`maybe_foul`), on its own `FOUL_NS` stream (`play_match` gained a
`foul_rng: &mut Rng` parameter, identity `foul_rate: 0.0`) — an unconditional check draw, then
a conditional severity draw only if it fires, the same shape `roll_injury` (§14) already uses.
A fired foul is a **non-mirroring turnover**: the beat returns `(poss, zone)` unchanged instead
of the take-on/pass's own outcome, so the fouled side keeps the ball right where it happened. A
red (straight or second yellow) sets `XiPlayer.sent_off_from_minute`; `sample_by_presence` and
`team_means` (renamed to take a `minute` and filter `on_pitch`) drop that player from every
subsequent tick's sampling, giving the "renormalize over ten" behaviour with no change to the
presence tables themselves — `team_means` now recomputes per tick once a side has actually
gone down a player (a cheap fast path reuses the once-per-match reading otherwise, so the
identity setting's per-tick cost is unchanged). `current_gk` resolves the red-carded-keeper
edge case: formation slot 0 if still on the pitch, else the lowest-indexed on-pitch outfielder,
attributes and all. `GameState::is_suspended`/`suspended_players` (§12) derive bans straight
from `season_cards`, joining `available()` alongside the injury check; `ai_pick_lineup_vs`/
`ai_pick_lineup_available` gained a `suspended: &BTreeSet<PlayerId>` parameter so AI-controlled
sides respect suspensions too, not just the human's own `validate_lineup`/
`effective_player_lineup`. A sent-off player's recorded `MatchPlayed` minutes now stop at his
dismissal minute rather than running to a flat 90 — T11's own slice of §2.8's "T10/T11/T12 make
partial minutes possible," ahead of T12's substitutions.

**The foul contest** attaches to the contests that model a challenge: after a take-on
resolves (either way), and after a *failed* pass in the defender's pressed zones, a foul draw
fires: `p_foul = σ(base_foul + a·(Aggression − 50)/50 − c·(Composure/Decisions blend − 50)/50
+ press_term + fatigue_term)` — the schema §6 #8 signature (↑ Aggression, ↓ Composure,
Decisions), plus the two modulators 2e adds (a High press fouls more, via the defending side's
own `SideEffects::fatigue_mult`; tired legs foul more, via `1 - fatigue_mult`). "The defender's
pressed zones" resolved as every zone but the actor's own `Def` third: a lost ball there reads
as a clean interception during build-up, not a physical challenge worth a foul check.

**Restart (v1):** the fouled side retains possession in the zone — a free kick abstracted to
"possession kept", per §11's set-piece deferral. No shot from the foul yet.

**Cards, given a foul:** a severity draw sets `Yellow` (p_yellow ≈ 0.15–0.20 of fouls) or
straight `Red` (rare, ≈ 0.01), with the yellow odds pushed up by the same Aggression margin
and by repeat fouling (a per-player in-match foul count — the referee's patience is state the
engine already has for free). A second yellow is a red by bookkeeping, not a new draw.
**Implementation refinement found during calibration:** a second yellow is *not* drawn from the
same formula as the first, plus its repeat/aggression bumps — an early pass did exactly that,
and one aggressive repeat fouler's compounding foul count reliably snowballed into an
implausible per-match red rate (≈0.39/team/match, four times the target). `foul_second_yellow_base`
is its own flat, deliberately low probability once a player already carries a yellow: a
cautioned player draws more careful, and the second dismissal-worthy act is rarer per foul than
the first, not more common — the repeat/aggression bumps stay first-yellow-only.

**The discipline resolution (`ATTRIBUTE_SCHEMA.md` §9 item 3) — verdict: Aggression alone
sufficed, no hidden discipline factor needed.** The lean-and-add case: the schema merged
Bravery into Aggression and flagged the possible split "if card rates won't calibrate from
Aggression alone." T11's calibration (below) hit both the yellow band (~2-3/team/match) and
the red band (well under 0.1/team/match) by tuning only the foul-contest's own knobs
(`foul_base`, `foul_yellow_base`, `foul_red_base`, `foul_second_yellow_base`) — the duel
contests' own weights (`TAKEON_DEF`, `PASS_DEF`, `AERIAL_DEF`, all still schema §6-verbatim)
were never touched. No conflation ever showed up between "Aggression as a duel-performance
input" and "Aggression as a card-risk input," so the split tripwire never fires: one attribute,
no new field, `ATTRIBUTE_SCHEMA.md` §9 item 3 resolves closed.

**Derived suspensions:** a red (straight or second yellow) → miss the next league match;
5 accumulated yellows → 1-match ban, counter resets, season boundary clears it. All of it
**derived in the fold from recorded cards** — §12's derived-suspension rule; cards are the
truth, the ban is a view, `available()` enforces it. `GameState::is_suspended` compares each
qualifying booking's matchday against `current_matchday - 1` exactly, so a ban both applies and
self-clears without ever storing a "served" flag — once the calendar moves past the one match a
booking covers, the same read starts returning `false` again.

**Playing short:** a sent-off player leaves the XI; presence sampling renormalizes over ten
(the §6 tables need no change — totals just shrink), team means recompute, and the one edge
case is pinned by test: a red-carded *keeper* forces either a sub (§16) or an outfielder in
goal (slot re-roled `Gk`, his attributes making the punishment automatic). The `+0.4-0.6`
expected-goals-conceded-over-the-remainder target below is unverified in v1 — it needs T12's
substitutions to be a fair reading (right now a ten-man side never gets a tactical reshuffle in
response, which would bias the reading pessimistic).

**§8 impact:** gpm ≈ unchanged at the league level (fouls retain possession; reds are rare and
symmetric) — not independently re-verified here (`favourite_discrimination_regression_guard`
passes with the mechanism live at production defaults, the same scope-of-check T10's own §8
impact note relied on). New §8 rows, pooled over 16 seeds via `bin/calibrate`: fouls/game
**20.2** (target ~20-25), yellows/team/match **2.20** (target ~2-3), reds/team/match **0.038**
(target well under 0.1) — landed on the first knob pick that hit all three simultaneously,
`foul_base: -2.5` (from an initial `-3.3`, which read only 9.5 fouls/game and 1.06 (later 1.07)
yellows/team/match, both well short) alongside the second-yellow decoupling above. H/D/A and
the favourite-discrimination guard hold at these knobs (`favourite_discrimination_regression_guard`,
production `Knobs::default()`); red-card matches will fatten the scoreline tails slightly
(watched, not banded, in v1). Suspension matches served/club/season: not yet read off a real
pooled season — a `bin/calibrate` reporting gap, not a modelling one.

## 16. Substitutions (R7)

**Status: landed (batch-3 T12).** Implementation lives in `match_engine::resolve`: `simulate`
now owns `home`/`away` (and their bench counterparts) as mutable `Vec<XiPlayer>` rather than
borrowing them, since a substitution replaces a slot's `XiPlayer` outright — `step`/
`take_shot`/`sample_by_presence`/`team_means` still only ever see a borrowed `&[XiPlayer]` for
one segment between decision points, so none of their signatures changed. Decision points
(half-time, the fixed `SUB_CHECKPOINTS` `[60, 70, 80]`, or forced immediately when a tick's
injury/card count grows for a given side) call `evaluate_decision_point`, which walks
`Lineup.sub_plan`'s rules in order and fires an action once every one of its `SubCondition`
clauses holds. `build_xi`'s per-player body was factored into `build_xi_player`, reused by a
new `build_bench` — every dressed player, starter or bench, gets his Consistency multiplier and
ambient-injury pre-roll at kickoff (§16's own "RNG discipline" requirement), so a substitute
enters with already-resolved match-time attributes and evaluation itself draws nothing.
`XiPlayer` gained `entered_at_minute` (`0.0` for a starter, the identity) so a substitute's
`fatigue_mult` clock starts at his own entry rather than the match clock. A departed player's
final minutes are captured into a side accumulator before his slot is overwritten (his own
`XiPlayer` no longer exists in `home`/`away` to read them back from); `GameState::available` and
the injury/suspension pipeline are otherwise untouched.

**T12 finding — the substitution cap is 3, not 5.** This section was drafted at T2 against the
real law's simplification; the batch-3 handoff's own T12 task spec pins **"three substitutions
maximum"** explicitly, in both its deliverable and its test list. Implemented as
`fforge_domain::MAX_SUBSTITUTIONS = 3` (bench size stays **7**, unaffected). Treated as the
authoritative, more specific instruction superseding this section's original "5" — filed here
as the correction, per the project's own "divergences get fixed as findings" convention, rather
than silently building to the newer number without a record of the change.

**Scope actually landed vs. this section's original draft:**
- **The rule vocabulary is the general condition→action language `TACTICS_MODEL.md` §7/this
  doc's own §2.7 described** (`SubCondition::{MinuteAtLeast, Score, PlayerConditionBelow,
  PlayerInjured, ManDown}`, `SubAction::{Substitute, SetMentality, SetTempo, SetWidth,
  SetPressing}`, `SubRule { conditions: Vec<SubCondition>, action }`, AND-combined) — not the
  three named built-in rule *kinds* (forced/fatigue/chase-hold) this draft sketched. Those three
  are expressible in the general language (e.g. a forced-cover rule is one `PlayerInjured(pid)`
  condition per starter, naming a fixed bench replacement — never resolved by an in-match
  "best fit" search, keeping evaluation genuinely draw-free) but **no policy that emits them
  automatically exists yet.**
- **No AI default plan or bench-selection policy landed.** `ai_pick_lineup`/`ai_pick_lineup_vs`
  field every AI-controlled side with an empty bench and an empty `sub_plan` — the substitution
  identity, so AI-vs-AI league play is entirely unaffected by this task (confirmed: every pooled
  calibration guard reads unchanged). This mirrors `ai_pick_tactics`'s own seam
  (`TACTICS_MODEL.md` §7) and is left for a follow-on task, not silently dropped — T12's own
  deliverable and test list never actually required it (only that a *submitted* plan, human or
  hand-built, is honoured). The human side has no bench/plan UI yet either (`fforge-game`
  submits an empty bench/plan, same as its tactics picker), for the same reason
  `TACTICS_MODEL.md`'s own tactics UI is deferred.
- **No new persisted `MatchOutcome`/`MatchPlayed` field.** Unlike injuries/cards, a
  substitution has no derived `GameState` consequence beyond minutes — there is no
  "substitutions history" field anything downstream reads. `MatchEventKind::Substitution`
  narrates it in the Trace-only stream (discarded after presentation, exactly like every other
  beat); the one consequence that *does* outlive the match, minutes, already had a field
  (`MatchPlayed.minutes`, landed early at T4) built for exactly this.

**Law, simplified:** a bench of **7** (squad floor 18 = XI + 7, so every legal squad can fill
it), **three substitutions** (T12 finding above — not the real law's 5 this section originally
drafted), usable at fixed decision points — half-time, 60', 70', 80' — plus immediately on
injury or a red card. Window-count bookkeeping (the real law's "3 windows") is not modeled in
v1: the four fixed points *are* the windows.

- **The `Lineup` widens** (with `tactics`, `TACTICS_MODEL.md` §6): `bench: Vec<PlayerId>`
  (≤ 7, validated like starters — `commands::validate_lineup`) and `sub_plan: Vec<SubRule>`.
  Old logs deserialize to an empty bench and plan — and both empty means no decision point can
  ever act, which keeps pre-2e replays coherent.
- **Decisions are pre-committed reactive plans**, never mid-match I/O — `play_match` is pure
  (`TACTICS_MODEL.md` §7 pins the pattern). The human's plan rides the `Lineup`; the AI plays
  with an empty one in v1 (the "scope actually landed" note above) — a future default-plan
  policy is `ai_pick_tactics`'s own sibling seam, not yet built.
- **RNG discipline:** plan evaluation is deterministic and draw-free (conditions read the
  score, the clock, and already-resolved injury/card/condition state; a rule whose named player
  is no longer eligible — already subbed or sent off, or no longer on the bench — is a silent
  no-op, not a search for a substitute). A substitution changes *who* is sampled afterwards —
  outcomes diverge, the draw *mechanism* doesn't. No bench + no plan ⇒ zero new draws ⇒ the §11
  regime-1 property degrades gracefully, confirmed by `ai_pick_lineup`'s empty-bench/plan
  default reproducing the T5 golden baseline bit-for-bit with no special-casing needed.
- **Consequences:** `minutes` in `MatchPlayed` (§12) becomes real per-player minutes — the
  playing-time upgrade `DEVELOPMENT_MODEL.md` §3 reserved; condition drain (§13) scales with
  minutes actually played; a sub's fatigue clock starts at his entry minute (fresh legs are
  mechanically real via `fatigue_mult`'s minute argument being offset).

**§8 impact:** **not measured against production league play** — every AI-controlled match
still runs the substitution identity (empty bench/plan), so `bin/calibrate`'s pooled aggregates
are unaffected by construction (confirmed: every pooled calibration guard reads unchanged with
the mechanism live). The original subs/match ~4-5, late-match goal-share, and gpm predictions
apply once a default AI plan exists to actually generate substitutions in league play — deferred
alongside that policy. What *is* verified: a submitted plan resolves deterministically, honours
the 3-substitution cap, reacts to forced triggers (injury, red card) ahead of the next fixed
checkpoint, and produces the non-degenerate partial minutes `DEVELOPMENT_MODEL.md` §3 needed
(`match_engine`'s own test suite, plus a `commands::step`-level integration test proving the
minutes reach `GameState.appearances_since_tick`).

## 17. Character activation — Consistency & Concentration, and the schema §9 item 2 verdict

**Status: Consistency landed (batch-3 T8); Concentration not yet implemented.** This section
was written at T2 (design-note-first, before any 2e Rust) describing both halves of the
schema §9 item 2 resolution as one design; only Consistency was ever assigned an
implementation task (T8) in the batch-3 task list — Concentration's lapse mechanic has no
task and is not wired into `fforge-core` (confirmed: no `Attribute::Concentration` reference
anywhere in the crate). The "split holds" verdict below is therefore only half-tested; treat
Concentration's half as still a pure design description pending its own task.

Implementation lives in `match_engine::resolve::build_xi`:
`Knobs::consistency_sigma_max`/`consistency_mult_min`/`consistency_mult_max`
(`0.25`/`0.7`/`1.3`, plausibility-picked, identity `0.0` per §2.1) and its own RNG stream
(`match_engine::CONSISTENCY_NS`, "CONS"), drawn once per player at XI-build time (22 draws,
11 per side, unconditional, fixed slot order) and applied to *every* attribute uniformly for
the match. `play_match`/`resolve::play_match` gained `consistency_rng: &mut Rng` and `k:
&Knobs` parameters (the latter so the T5/T6 identity tests can pin
`consistency_sigma_max: 0.0` independent of the production default, the `notebook_parity`
precedent). Tests: the identity holds bit-for-bit (`build_xi_reproduces_raw_attributes_bit_
for_bit_at_the_consistency_identity`, plus the T5/T6 golden/bit-identity tests re-passing with
`consistency_sigma_max: 0.0` pinned); a low-Consistency player shows visibly (>2×) wider
match-to-match spread than a high-Consistency player at equal starting CA
(`low_consistency_shows_a_visibly_wider_match_to_match_spread_than_high_consistency`).

**T8 finding — the mean shift was larger than "roughly unchanged."** At `sigma_max = 0.25`,
pooled `bin/calibrate` goals/match moved from 2.64 (T6/T7 baseline, `AI_TACTICS_ENABLED =
false`) to 2.89 at a matched 16-seed pool (+9.5%), not the "roughly unchanged (the multiplier
is mean-1.0)" this section originally predicted. The per-match multiplier's *expectation* is
exactly 1.0 by construction, but that does not imply the *engine's output* mean is preserved:
contest probabilities are sigmoid functions of attribute differences, and a symmetric
perturbation to a nonlinear function's input does not generally preserve its output's mean
(the general shape of the effect Jensen's inequality describes). Recorded here rather than
chased — `consistency_sigma_max` is a plausibility pick explicitly meant to be a B3.9 fit
target (this section's own §8 impact note below), and T8's scope fence was "do not re-tune
knobs in this task." T14's re-banking pass inherits this as a concrete, measured number to
weigh when it proposes a knob table.

**Guard interaction (T8 finding).** `favourite_discrimination_regression_guard`'s pool was
widened 8 → 24 seeds: Consistency's added per-match variance made the guard's sparsest
extreme-strength-gap bins (a handful of matches each at 8 seeds) trip the monotonicity
tolerance — e.g. the `(gap −27, 6 matches) → (gap −25, 18 matches)` pair read `(0.083,
0.000)`, an 0.083 dip past the 0.05 tolerance, at 8 seeds; the same bin pair at 24 seeds reads
`(0.023, 0.008)`, an 0.015 dip comfortably inside it. This is test-pool sizing (statistical
power at the harness's sparsest bins), not a knob re-tune — `consistency_sigma_max` and the
tolerance itself are unchanged.

The two attributes activate on *different axes*, which is the whole test of whether the split
holds:

- **Consistency (hidden)** — *between*-match variance: one draw per player per match (22
  draws at team-build time, fixed count, before the possession loop) scales his effective
  attributes for the day: `eff = attrs × (1 + σ(consistency) · z)`, `z ~ N(0,1)`, where
  `σ(consistency)` falls with the hidden rating (a 90-consistency player is nearly always
  himself; a 40 swings ±). This is `ATTRIBUTE_SCHEMA.md` §6 #13's "per-match floor".
- **Concentration (performance)** — *within*-match lapses: inside defensive contests, a lapse
  check whose probability rises with match minute (fatigue-coupled, per the schema: "avoiding
  lapses; error rate, esp. when fatigued") and falls with Concentration; a lapse applies a
  flat penalty to the defender's contest score for that resolution. Cheap, local, and it makes
  low-Concentration defenses concede *late* — a narratable, calibratable signature.

**Resolution of `ATTRIBUTE_SCHEMA.md` §9 item 2 — the split holds.** The two knobs now drive
two *distinct observables*: Consistency moves match-to-match rating volatility and the
underdog upset rate (favourite-discrimination slope softens as consistency spread widens);
Concentration moves the late-goal share and nothing else. Neither observable is reachable by
the other knob, so both earn their keep, and the collapse option is closed. (Determination's
big-match modifier and Leadership stay Phase 5 — "match importance" and morale don't exist
yet, and activating them here would be decoration.)

**§8 impact:** pooled means unchanged (both effects are zero-mean by construction); the
favourite-discrimination *slope* softens slightly — the guard's bounded-deviation band is the
watchpoint, and `σ(consistency)`'s scale is fitted against it (upsets should get a touch more
common, monotonicity must survive). New telemetry: per-player rating standard deviation vs
hidden Consistency (should correlate negatively), late-goal share vs defense Concentration.

## 18. Match ratings (R9)

**Definition: a per-player 1.0–10.0 rating, a pure RNG-free function of the player's recorded
stream events plus the team result**, computed once at match end and recorded (`ratings`,
§12 — recorded because the stream is not persisted for AI matches, and news/form/morale need
it replay-safe). Stored as tenths in a `u8` (68 = 6.8), clamped to `[3.0, 10.0]`.

Base 6.0, plus event deltas read off the stream (starting weights — plausibility-picked, the
knob-table discipline):

| Stream evidence | Delta |
|---|---|
| Goal | +1.0 |
| Assist (the same-side successful `Pass`/`Cross`/`TakeOn` immediately preceding a scored shot in the stream) | +0.7 |
| Shot on target (non-goal) / off-blocked | +0.10 / −0.05 |
| Take-on won / lost | +0.10 / −0.05 |
| Pass completed / lost | +0.02 / −0.04 |
| Tackle won (named `opponent` of a failed take-on) | +0.15 |
| Save (GK) | +0.20 |
| Card: yellow / red | −0.3 / −1.0 |
| Causing an opposition goal's turnover (last failed action before a mirrored-restart goal) | −0.3 |
| Team: win / clean sheet (GK, CB, FB only) | +0.2 / +0.5 |
| Minutes scaling | deltas accrue only while on the pitch; sub cameos regress toward 6.0 |

The assist and blame rules are derivable purely from stream ordering (the stream is
chronological and side-tagged), so the rating needs no new engine hooks — it is a fold over
`MatchOutcome.stream` performed while the stream is still in hand.

**Ratings drive nothing mechanical in 2e.** They feed the news Trace (R2's `NewsObserver` gets
man-of-the-match material) and become the input the deferred systems were designed around: the
form multiplier valuation deliberately left out (`TRANSFER_MODEL.md` §2.5 — its "small,
bounded, decaying" sketch is a decayed rating average), and Phase-5 morale — which, when it
closes the loop, does so under `DESIGN.md` §7's bounded/event-triggered feedback rules. Not
before.

**§8 impact: none** (a derivation moves no aggregate). New §8 rows: league mean rating ~6.4–6.6,
winning-side mean ≈ +0.3 over losing-side, goalscorer mean > 7, and a sane man-of-the-match
spread (forwards overrepresented but defenders present — if the weights can't produce a
defender MOTM, the tackle/clean-sheet weights are wrong, not the world).
