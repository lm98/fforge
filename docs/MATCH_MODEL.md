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
  character/hidden attributes (Consistency, Determination, …). The wide route is included now
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
