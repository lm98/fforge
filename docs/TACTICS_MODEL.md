# Tactics Model — Phase 2e Design Note

The design record for the tactics subsystem (`DESIGN.md` §4.1: "tactics = modifiers to the
transition matrix; aim for soft rock-paper-scissors matchups"; `MATCH_MODEL.md` §1's first
deferred item). It pins the decisions in the same spirit as `MATCH_MODEL.md`,
`DEVELOPMENT_MODEL.md`, and `TRANSFER_MODEL.md`: a living artifact to reference and extend,
written **before** any Rust lands — design-note-first is the project pattern, and this note is
the gate.

Tactics gets its own note (rather than a `MATCH_MODEL.md` section, where the rest of Phase 2e
lives — see `MATCH_MODEL.md` §11) because it is a genuinely new subsystem with its own
decision inventory: an instruction surface, a resolution into effective knobs, an interaction
model, an AI policy, and a whole new agent decision space for Phase 5. Injuries, cards,
fatigue-carryover, substitutions, and ratings are *additions to the existing match model* and
are drafted as sections in the note that already pins that model.

---

## 1. Purpose & status

- **Status: §2–§4, §6 landed (batch-3 handoff T6) — the engine + neutral-everywhere rollout
  step.** `Tactics`/`Mentality`/`Tempo`/`Width`/`Pressing` live in `fforge-domain::tactics`;
  `Lineup.tactics` rides the existing `SubmitLineup`/`LineupSubmitted` seam unchanged, serde-
  defaulted; `fforge-core::match_engine::tactics::{SideEffects, resolve_tactics}` implements
  §3's per-side resolution and is wired into `resolve.rs`'s `select_action`/`step`/`take_shot`/
  `simulate` (three deformation types only, no new contest types, no new zones, no presence-
  table edits). Both §4 tests land green: `resolve_tactics_neutral_is_the_exact_identity` and
  `neutral_tactics_reproduce_phase_2a_bit_for_bit` (replaying the T5 golden table). All four
  pooled calibration guards re-ran unchanged. `ai_pick_lineup` still emits `Tactics::neutral()`
  everywhere — §7's AI policy (T7) is not yet wired in, so no non-neutral tactics reach a real
  match outside tests. §5's interaction model and §8's calibration predictions remain to be
  verified by T7's triangle harness. No scratchpad was used to settle the structure — the
  `TRANSFER_MODEL.md` §1.1 reasoning applies verbatim: the *structure* is settled by this note
  (every tactic effect is a deformation of an already-calibrated probability), and the numbers
  are knob-table entries the existing Rust harness (`match_engine::calibrate`, `bin/calibrate`)
  will fit directly, exactly as it re-fit `b_beat`.
- **In scope (this pass):** the instruction surface (§2), per-side resolution into effective
  knobs (§3), the neutral-tactics invariant (§4), the structural interaction model (§5), the
  event-log seam (§6), the v1 AI tactics policy and its Phase-5 seam (§7), and the calibration
  predictions B3.9 checks (§8).
- **Out of scope:** in-match tactic *changes* (they ride the substitution decision-point
  mechanism, `MATCH_MODEL.md` §16, and the pre-commitment model of §7 here); per-player
  instructions (man-marking, free roles); formation design beyond the existing four
  (`fforge-domain::FORMATIONS`); opposition-specific counter-picking by the AI (§7, deferred
  to Phase 5 where it is an agent-quality question).

## 2. The instruction surface — the count is **four**, derived, not chosen

R4 proposes five instructions. The count should not be a taste call; it falls out of three
admission criteria an instruction must pass:

1. **It binds to a distinct structural seam the engine already reserved.** `DESIGN.md` §4.1
   commits tactics to being *transition-matrix modifiers* — no new zones, no new contest
   types, no presence-table rewrites (`MATCH_MODEL.md` §10 item 1 explicitly reserved
   per-formation presence editing as future shape-finding work, and per-*tactic* editing is
   the same reservation).
2. **It moves a distinct §8 observable.** Two instructions that both only move goals/game are
   indistinguishable to the calibration harness — one of them is uncalibratable and therefore
   unfalsifiable, which is how matchup-table fudge factors sneak in.
3. **It poses a genuine tradeoff at the neutral calibration point.** If one setting dominates,
   the instruction is not a decision, it is a difficulty slider.

Now enumerate the seams the engine actually has. Reading `resolve.rs`/`contest.rs`/`knobs.rs`,
there are exactly four places a per-side modifier can attach without structural change:

| Seam | Where it already exists | Instruction |
|---|---|---|
| (a) The lateral split `p_wide` | `MATCH_MODEL.md` §3: "where a future *width* tactic plugs in unchanged"; `formation_p_wide` already makes it per-side | **Width** |
| (b) Action-selection weights + advance probabilities (`w_*`, `p_def_advance`, `p_mid_advance`, `p_attc_*`) | `select_action`'s doc comment: "where a future direct/patient tactic re-weights, no structural change" | **Tempo** |
| (c) The additive bias slot in the shared logistic | `contest_p` already carries `home_bias` there — a per-side, per-zone tactical bias is the same mechanism | **Mentality** (symmetric attack/defence posture) and **Pressing** (zone-profiled, build-up-targeted) |
| (d) The fatigue rate | `fatigue_mult`'s `fatigue_base`/`fatigue_wr` | consumed by **Pressing** as its cost term |

Four seams, four instructions:

| Instruction | Levels | One-line meaning |
|---|---|---|
| `Mentality` | `Defensive` / `Balanced` / `Attacking` | risk posture: commit men forward for chance volume, at the price of counter exposure |
| `Tempo` | `Patient` / `Balanced` / `Direct` | progression style: many safe actions vs few risky ones |
| `Width` | `Narrow` / `Balanced` / `Wide` | route mix: how much of the final-third entry goes through `AttW` |
| `Pressing` | `Deep` / `Balanced` / `High` | where you contest the opponent's possession: their build-up, or your own block |

One tension to name before it's raised: Mentality and Pressing both use the bias *slot* (c) —
doesn't that fail the "one seam, one instruction" logic? No, because the slot is a mechanism,
not a seam: Mentality applies it **zone-uniformly and symmetrically** (attack up, defence
down — a posture), Pressing applies it **zone-targeted with a fatigue cost** (a location), and
they move disjoint §8 observables (goal variance vs build-up completion / high turnovers), so
criterion 2 separates them cleanly. The test for redundancy is the observables, and it is
exactly the test the fifth candidate fails:

**Why not five.** The natural fifth candidate, **defensive line height**, fails criterion 1:
the five-zone state space has no vertical geometry independent of *where you press* and *what
a turnover costs* — both already owned. "High line" = contest their build-up (Pressing `High`)
plus increased exposure behind (the §5 beaten-press term); "low block" = concede their
build-up and strengthen the final-third contests (Pressing `Deep`). Every effect a line-height
knob could have is a restatement of Pressing's zone profile — the same seam under a second
name, which is the matchup-lookup-table failure in miniature: two knobs, one mechanism, and
calibration can no longer attribute an aggregate shift to either. Line height is *reserved* as
the fifth instruction for the day `Box` becomes a dwell zone or a sixth zone appears
(`MATCH_MODEL.md` §10 item 5), where it would finally own geometry of its own.

**Why not three.** Folding Width into Tempo fails criterion 2 in reverse: Width owns a §8
observable no other instruction touches (the wide-origin goal share) and a seam explicitly
reserved for it. And collapsing everything into one FM-style mentality *gestalt* would satisfy
all three criteria trivially — one knob, one observable, one tradeoff — but destroys the
Phase-5 decision space: an LLM manager choosing from a 3-point slider produces no measurable
decision quality, while 3⁴ = 81 legible profiles is a real policy space that a utility
baseline can enumerate and an ablation can score (`DESIGN.md` §5).

**Why ternary levels, not sliders.** (i) The neutral level is an honest identity element (§4
needs one); (ii) 81 total profiles keeps B3.9's per-instruction predictions enumerable and
keeps the Phase-5 structured-output validation trivial; (iii) continuous sliders invite
degenerate optimization against calibration residue (agents grinding +0.03 width) without
adding any decision content. Cheap to widen later; painful to narrow (the schema §3
lean-and-add rule, applied to an instruction surface).

```rust
pub enum Mentality { Defensive, Balanced, Attacking }
pub enum Tempo     { Patient,   Balanced, Direct    }
pub enum Width     { Narrow,    Balanced, Wide      }
pub enum Pressing  { Deep,      Balanced, High      }

/// The per-side tactical instruction set (this note §2). `Default` is
/// `neutral()` — load-bearing for serde back-compat (§6) and the §4 invariant.
pub struct Tactics {
    pub mentality: Mentality,
    pub tempo: Tempo,
    pub width: Width,
    pub pressing: Pressing,
}
impl Tactics { pub fn neutral() -> Self { /* all Balanced */ } }
```

`Tactics` lives in `fforge-domain` (it is part of the `Lineup` decision value, §6) — a
sanctioned Phase-2e domain extension in the same sense as Phase 4's finance types
(`TRANSFER_MODEL.md` §3; the full 2e list is in `MATCH_MODEL.md` §12).

## 3. Resolution into effective knobs — per side, draw-free, computed once

The committed shape: a **pure, RNG-free function from `(Tactics, Tactics)` to a per-side
effect table, evaluated once per match** where `team_means` already builds its per-side view.
`TeamMeans` is the precedent — it already carries a per-side `p_wide` derived from the fielded
formation (`formation_p_wide`); tactics extends that per-side view rather than inventing a new
one.

```rust
/// Per-side effective view, resolved once per match from (own tactics,
/// opponent tactics, formation). Pure and RNG-free — consuming zero draws is
/// what makes the §4 invariant hold by construction.
struct SideEffects {
    p_wide_mult: f64,             // Width  → stacks on formation_p_wide
    advance_mult: f64,            // Tempo/Mentality → p_def_advance, p_mid_advance
    penetrate_mult: f64,          // Mentality → p_attc_penetrate, p_attc_dribble_box
    action_w_mult: [f64; N],      // Tempo  → the w_* selection weights
    atk_bias: f64,                // Mentality → added into contest_p when this side attacks
    def_bias_by_zone: [f64; 5],   // Pressing/Mentality → added (negated) when this side defends, keyed by the possessing side's zone
    fatigue_mult: f64,            // Pressing → scales fatigue_base
    b_pass_delta: f64,            // Tempo  → pass risk
}
```

Every effect is one of exactly three deformation types — **(i)** a multiplier on an existing
transition/selection probability, **(ii)** an additive term in the existing logistic bias slot
(the `home_bias` mechanism), **(iii)** a multiplier on the fatigue rate. No new contest types,
no new zones, no presence-table edits (§2 criterion 1). Zones are possessing-team-relative, so
a defending side's `def_bias_by_zone` is keyed by the *possessing* side's zone — Pressing
`High` is a bias in the opponent's `Def`/`Mid`, i.e. their build-up.

Starting effect table — **plausibility-picked, the `ValueKnobs` §9 discipline: every number
here is a fit target for B3.9** (§8), not a commitment. Neutral rows are identically 1.0 / 0.0
and are therefore omitted.

| Instruction, level | Effect (per side) |
|---|---|
| Mentality `Attacking` | `advance_mult` ×1.20, `penetrate_mult` ×1.20, `atk_bias` +0.08, `def_bias_by_zone` −0.08 in every zone (men committed forward defend everything worse) |
| Mentality `Defensive` | mirror: ×0.83, ×0.83, −0.08, +0.08 |
| Tempo `Direct` | `advance_mult` ×1.30, `w_longshot` ×1.5, `w_takeon` ×1.1, `b_pass_delta` −0.15 (forward balls fail more) |
| Tempo `Patient` | `advance_mult` ×0.80, `w_longshot` ×0.6, `b_pass_delta` +0.10 |
| Width `Wide` | `p_wide_mult` ×1.35, `w_cross` ×1.2 |
| Width `Narrow` | `p_wide_mult` ×0.70, `w_cross` ×0.85 |
| Pressing `High` | `def_bias_by_zone` +0.15 in opponent `Def`/`Mid`, 0 elsewhere; own `fatigue_mult` ×1.30; **beaten-press term:** opponent's successful `Mid` advance gets ×1.15 on `p_mid_advance` (the space behind a committed press) |
| Pressing `Deep` | `def_bias_by_zone` −0.10 in opponent `Def`/`Mid` (sitting off), +0.10 in opponent `AttC`/`AttW`/`Box` (the compact block); opponent `p_attc_penetrate` ×0.85 (no space behind) |

Where Mentality and Tempo both touch `advance_mult`, the multipliers stack (they are
independent levers on the same probability, like `formation_p_wide` × `p_wide_mult`); all
composed probabilities clamp to `[0, 1]` at the point of use, the `formation_p_wide` precedent.

**What tactics deliberately does not touch:** the role→zone presence tables (`MATCH_MODEL.md`
§6). Who *is* in a zone stays a property of the fielded roles; tactics changes what they
attempt and how contests tilt. Editing presence per tactic is new shape-finding work
`MATCH_MODEL.md` §10 item 1 reserved, and it stays reserved (§9 here carries the open item).

## 4. The neutral-tactics invariant — stated as an invariant, with the test

> **Invariant.** For every `(world, home_lineup, away_lineup, rng_stream)`:
> `play_match` with `Tactics::neutral()` on both sides produces a `MatchOutcome` **identical
> bit-for-bit** to today's Phase-2a engine — same score, same stream, same length, **and the
> same RNG draw sequence**, so every downstream draw in the fixture's stream is unperturbed.

Two design rules make it hold *by construction* rather than by hope:

1. **Tactics resolution consumes no randomness** (§3). All effects deform probabilities that
   feed *existing* draws; no code path adds, removes, or reorders a draw. The draw sequence
   can therefore only change if a probability's *value* changes.
2. **Neutral resolves to exact identity values.** `SideEffects` for `Balanced` everything is
   multipliers of literally `1.0` and biases of literally `0.0`, and IEEE-754 makes `p * 1.0`
   and `x + 0.0` exact — or, stricter still, the implementation may bypass the deformation and
   read the raw knob field when the multiplier is the identity constant. Either way every
   probability compared against every draw is bit-identical to today's, hence every branch,
   hence the sequence.

Why the invariant is worth this care: it is **calibration continuity**. Every fitted value in
`Knobs::default()`, the §8 table, `favourite_discrimination_regression_guard`, and
`aggregates_are_in_a_believable_ballpark` remains valid at the neutral point — 2e's tactics
land with zero re-fit debt, and AI clubs can ship with neutral tactics on day one with zero
behavior change. It is also the **save-compatibility story**: an old `LineupSubmitted` with no
tactics field deserializes to `neutral()` (§6), and by this invariant that replay is
bit-identical — backward compatibility *is* the neutral invariant in serialized form.

**The tests attached** (both land in the same PR as the engine change, red-before-green):

- `resolve_tactics_neutral_is_the_exact_identity` — unit: every field of
  `SideEffects` for `(neutral, neutral)` equals its identity constant exactly (`== 1.0` /
  `== 0.0`, not approx).
- `neutral_tactics_reproduce_phase_2a_bit_for_bit` — integration golden test: **before** the
  tactics change lands, capture and commit a digest of the current engine's output — the exact
  `(home_goals, away_goals)` and stream length for seeds `0..32` over the
  `worldgen::generate(7)` world with `ai_pick_lineup` XIs (the `same_seed_same_outcome`
  fixtures, pinned). The test replays those seeds through the tactics-aware engine at
  `neutral()`/`neutral()` and asserts equality against the pinned constants. Any accidental
  extra draw or perturbed probability fails it loudly.

The invariant is **tactics-specific**. The rest of Phase 2e (fouls, injuries, consistency,
subs) necessarily adds draws and moves outcomes; those features are governed by the §8
band-and-re-fit regime, not bit-for-bit equality — `MATCH_MODEL.md` §11 pins that split.

## 5. The interaction model — structural rock-paper-scissors, not a lookup table

The commitment: **no matchup table exists anywhere in the engine.** Nothing consults "tactic A
vs tactic B → modifier". Intransitivity must *emerge* from two mechanisms `MATCH_MODEL.md` §3
already has — **turnover mirroring** (where a lost ball restarts: lose it deep, they win it
high; lose it high, they win it deep) and the **per-action risk profile** (§3's contest
branches + §4's base rates). A lookup table would be calibratable but dead: it encodes the
answer instead of the situation, generalizes to nothing (a fifth tactic would need 4 new rows,
not 0), and hands Phase-5 agents a solved game.

The triangle, each edge a mechanical consequence, written here as **predictions B3.9 must
verify** (§8), not tuned-in outcomes:

- **High press beats Patient build-up.** Patient lowers advance probabilities and retains — so
  it plays *more* contested actions per possession precisely in its own `Def`/`Mid`, the only
  zones where the press bias applies. Every extra failed pass in `Def` mirrors to the pressing
  side's `AttC` restart — lost deep is dangerous, per §3's own table. Mechanism: exposure
  concentration, not a bonus.
- **Direct beats the High press.** Direct raises advance probabilities — fewer dwelling steps
  in the pressed zones, so the press bias simply *applies less often*; the beaten-press term
  (§3) means each successful escape advances further into the space behind; and the presser's
  `fatigue_mult` cost compounds late (`fatigue_mult` already scales every contest after ~60').
  Mechanism: denying the press its contests, then taxing its legs.
- **Patient beats Direct** (between two non-pressing sides). Direct's per-action success is
  lower (`b_pass_delta`, long-shot mix) with nobody pressing to justify the risk; its extra
  `Mid` turnovers mirror harmlessly to `Mid` but cede possession volume, while the patient
  side's higher completion accumulates territory and shot share. Mechanism: risk without a
  target is just variance donated to the opponent.

**Mentality is deliberately not on the triangle.** Attacking-vs-Defensive is the *risk axis*:
Attacking raises both sides' goal expectation (more possessions reach the final third, and
every ball lost there mirrors to a deep, safe opponent restart — but the −0.08 defensive bias
means the counters that do come through convert better). It amplifies the triangle's payoffs
rather than adding a fourth corner: `Defensive + Direct` is the classic counter posture that
profits most from an opponent's `Attacking` exposure, and that combination *emerges* from the
two axes rather than being a named tactic. Width, likewise, is the calibration-legible
instruction (it moves the wide-origin share, §8) and interacts mainly with the fielded
formation through `formation_p_wide`, not with the opponent — by design, so at least one
instruction has an almost-orthogonal, easily-verified signature.

## 6. Determinism & the event-log seam — `Tactics` rides the `Lineup`

**Decision: `Tactics` is a field of `Lineup`**, not a new command/event pair.

```rust
pub struct Lineup {
    pub formation: u8,
    pub players: [PlayerId; XI],
    #[serde(default)]            // ← old logs deserialize as neutral()
    pub tactics: Tactics,
}
```

- A team sheet without a shape is half a decision; they are chosen together, validated
  together, and recorded together. A separate `TacticsSubmitted` event would create
  pairing/ordering state ("which tactics apply to which matchday?") — exactly the transient
  `pending_lineup` problem, duplicated.
- `Command::SubmitLineup(Lineup)` and `Event::LineupSubmitted` are **unchanged in shape** —
  the payload widens, the seam does not move. Validation gains nothing: a `Tactics` value is
  valid by construction (closed enums).
- **Replay-safety** follows the existing pattern exactly: the human's resolved `Lineup`
  (tactics included) is recorded in `LineupSubmitted`; AI sides' tactics are re-derived
  deterministically at advance time from world state, the same way `ai_pick_lineup` already
  re-derives their XIs (pure function of `(world, club)` — same inputs at
  `player_match_preview` time and `advance_matchday` time, so the previewed trace still cannot
  disagree with the recorded score).
- **Serde back-compat:** `#[serde(default)]` + `Default = neutral()` makes every pre-2e
  `LineupSubmitted` in an existing log deserialize to neutral tactics, and the §4 invariant
  makes that replay bit-identical. No migration, no version field.

`MatchPlayed` does **not** record tactics — the score (and 2e's new resolved consequences,
`MATCH_MODEL.md` §12) remains the fold output; tactics is an *input*, already recorded where
inputs are recorded.

## 7. The AI tactics policy — and the Phase-5 seam it becomes

**v1: `ai_pick_tactics(world, club, opponent, is_home) -> Tactics`** — deterministic,
RNG-free, the tactics sibling of `ai_pick_lineup` and described the same way: *the Phase-1-
style stub of the layer-3 decision AI; same seam, richer policy later.* Policy content, kept
legible on purpose (thresholds in an `AiTacticKnobs` sibling table):

- **Mentality from the strength gap:** `lineup_strength` difference beyond ±one threshold →
  `Attacking` for the clear favourite, `Defensive` for the clear underdog; else `Balanced`.
  Away underdogs pair it with `Direct` — the counter posture of §5, emerging from the policy
  rather than hard-coded as a "counter tactic".
- **Width from the squad:** `wide_presence_share` of the chosen XI above/below thresholds →
  `Wide`/`Narrow` (reusing the exact function `formation_p_wide` already uses — no second
  encoding of "how wide is this team").
- **Tempo from the passers:** team `PASS_ATK` mean above threshold → `Patient`; below →
  `Direct`.
- **Pressing from the legs:** team Work-Rate + Stamina mean above threshold → `High`; low →
  `Deep`. (The policy pays the fatigue cost only when the squad can carry it.)

Deliberately opponent-blind except for the strength gap: real counter-picking (reading the
opponent's likely tactics and choosing the §5 counter) is a *decision-quality* behaviour, and
decision quality is precisely what Phase 5 measures — building it into the v1 baseline would
both complicate the baseline and flatten the very ablation (`DESIGN.md` §5: LLM-vs-utility)
the platform exists to run.

**The Phase-5 seam.** Tactics is the first *match-adjacent* decision an agent makes, and it
flows through the same propose-then-validate gate as every agent decision: an LLM manager
emits a constrained `Tactics` (81 legal values — trivially validatable structured output),
which rides the `Lineup` decision value into `LineupSubmitted`, exactly as `RecordedPolicy`
replays a pre-committed transfer plan (`TRANSFER_MODEL.md` §10). `ai_pick_tactics` is the
utility baseline that ablation swaps against.

**The substitution seam this becomes.** `play_match` is a pure function — it cannot pause
mid-match for I/O, human or LLM. So in-match decisions (tactic changes, substitutions,
`MATCH_MODEL.md` §16) take the **pre-committed reactive plan** form: a small, declarative
condition→action rule set ("if trailing after 60', Mentality → Attacking"; "sub the most
fatigued midfielder at 70'"), submitted with the team sheet, resolved deterministically inside
the engine at fixed decision points. That is `RecordedPolicy`'s pattern applied to the match
clock — never adapting mid-flight is the point; the plan *is* the decision, recorded up front.
Phase-5 agents author the same plan object; the v1 AI uses a default plan. This note fixes the
seam's *shape*; the decision points and rule vocabulary are pinned with substitutions in
`MATCH_MODEL.md` §16.

## 8. Calibration targets — predictions for B3.9, per instruction

The discipline: **every instruction states in advance which §8 aggregates it moves and by
roughly how much**, so B3.9 checks predictions instead of explaining surprises. Measured with
the existing harness (`StreamTelemetry` pooled over many seeds, one side forced to the tested
level, all else neutral), plus two new telemetry cuts the harness gains for the purpose:
per-zone pass completion, and turnover-won-by-zone.

| Forced setting (one side) | Aggregate | Prediction |
|---|---|---|
| — both neutral | **every §8 row** | **unchanged, bit-for-bit (§4)** |
| Width `Wide` | wide-origin goal share | +5–8 pts (from ~27% toward the band's top) |
| Width `Wide` | headed share of goals | +2–4 pts |
| Width `Narrow` | wide-origin goal share | −6–10 pts |
| Tempo `Direct` | that side's shots/game | +10–20% |
| Tempo `Direct` | that side's conversion | −1–2 pts (worse arrival mix: more long shots) |
| Tempo `Patient` | that side's possession share | +3–6 pts |
| Mentality `Attacking` | match goals (both sides') | +0.2–0.4 |
| Mentality `Attacking` | opponent goals from deep-mirrored restarts (counter-origin, via stream zone context) | visibly up |
| Mentality `Defensive` | match goals | −0.2–0.4 |
| Pressing `High` | opponent pass completion in their `Def`/`Mid` | −3–6 pts |
| Pressing `High` | turnovers won in the opponent's `Def` (→ own `AttC` restarts) | +30–60% |
| Pressing `High` | own contest success after 75' | measurably down (the fatigue cost is real) |
| any single instruction | pooled league H/D/A, favourite-discrimination slope | **within the existing §8 band / guard** |

And the §5 triangle, as pooled head-to-head predictions (many seeds, equal-strength squads,
only tactics varied):

| Matchup | Prediction |
|---|---|
| `High` press vs `Patient` | press side +2–5 pts expected-points share |
| `Direct` vs `High` press | direct side +2–5 pts |
| `Patient` vs `Direct` (no press) | patient side +2–5 pts |
| the three edges jointly | **cyclic** — no setting dominates the triangle |
| `Defensive+Direct` vs `Attacking` | counter side profits; both-`Attacking` gpm > both-`Defensive` gpm |

**Rollout discipline:** the engine + neutral-everywhere lands first (zero drift, §4);
`ai_pick_tactics` is enabled second, at which point pooled league aggregates *will* shift
(non-neutral tactics enter every AI match) — expected to be small because instruction effects
are roughly zero-mean across a league, but the full harness re-runs and, if needed, takes one
`b_beat`-style re-fit pass, recorded here exactly as `MATCH_MODEL.md` §8 recorded that one.
`favourite_discrimination_regression_guard` and the ballpark guard re-run with AI tactics on;
a tactics feature that breaks the favourite-discrimination slope is mis-designed, not
mis-fitted.

## 9. Open sub-questions

Deliberately unresolved, to settle during implementation or B3.9 calibration:

1. **Effect magnitudes.** Every §3 number is plausibility-picked; B3.9 fits them against the
   §8 predictions. Expect the biases (±0.08–0.15) to move most.
2. **Presence-table coupling.** Should `Mentality` eventually shift attacking/defensive
   presence (men genuinely upfield, not just better odds)? Reserved with `MATCH_MODEL.md` §10
   item 1 — it is the same "derive presence from context" question, and it stays closed until
   real calibration demands it.
3. **Per-zone press profile.** Pressing is one level applied to a fixed zone profile; whether
   `High` should differentiate pressing the `Def` build-up vs the `Mid` progression is a
   texture question — same seam, finer key.
4. **The reactive-plan vocabulary.** §7 fixes the pre-commitment shape; the exact condition
   set ("trailing", "after minute M", "man down") is pinned with substitutions
   (`MATCH_MODEL.md` §16) and should stay small enough for a utility baseline to search
   exhaustively.
5. **Line height as a sixth-zone tenant.** If `Box` ever becomes a dwell zone or the zone
   count grows (`MATCH_MODEL.md` §10 item 5), line height gets geometry of its own and
   re-enters as the fifth instruction (§2's reservation).
