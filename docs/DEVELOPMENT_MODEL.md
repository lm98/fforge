# Development Model — Phase 3 Design Note

The design record for the player-development engine (`DESIGN.md` §4.2, Phase 3). It pins the
decisions reached in shape-finding and the reasoning behind them, in the same spirit as `DESIGN.md`,
`ATTRIBUTE_SCHEMA.md`, and `MATCH_MODEL.md`: a living artifact to reference and extend. A throwaway
Python scratchpad (`dev_shape`, the sibling of `match_model_prototype`) was the shape-finder these
curve parameters were fitted in; **this note is the thing that survives it**, and the structure below
is what drops into Rust `fforge-core`.

This note resolves the six development decisions the earlier docs deliberately left open:
PA-gating (`ATTRIBUTE_SCHEMA.md` §4), the age-curve shapes per `DevCategory`
(`ATTRIBUTE_SCHEMA.md` §7), the event-log representation, the in-scope inputs (`DESIGN.md` §4.2),
Natural Fitness (`ATTRIBUTE_SCHEMA.md` §3), and the validation targets.

---

## 1. Purpose & status

- **Status:** Phase 3 — *model shape settled and calibrated in a Python scratchpad; not yet ported.*
  Per `DESIGN.md` §8 the deterministic core is built **once, in Rust**; the notebook is a
  discard-after-use design tool, never a port target. No `fforge-core` implementation lands with this
  note — it is the settled design the port follows.
- **In scope (this pass):** monthly attribute growth and decline across a full career, gated by
  hidden PA, driven by age, playing time, a club coaching coefficient, the Determination /
  Professionalism character attributes, and noise. Enough noise for wonderkids-who-flop and late
  bloomers. The append-only event-log seam and the career-arc calibration harness.
- **Deferred:** player-directed per-attribute **training focus** (a decision-layer / management-UI
  concern, Phase 4/6); **injuries** and **between-match recovery/fatigue-carryover** (Phase 2e);
  **Consistency, Injury-proneness, Leadership** as development inputs (their homes are match variance,
  injury events, and morale respectively — §4). Development is the active front; the knob table below
  is a fitted starting point, not a finished calibration (the Rust harness §6 re-fits it against real
  `worldgen`, exactly as `MATCH_MODEL.md` §8 re-fitted the match knobs).

## 2. The model — a PA-scaled age-envelope the attributes track

Development is a **trajectory from current attributes toward a hidden ceiling**, the commitment
`DESIGN.md` §4.2 makes. The shape that carries it: each attribute chases a **target** that is its
hidden potential scaled by a per-category **age envelope**. Growth is proportional approach to that
target; aging is the envelope turning down. This one structure delivers every property the schema
asks for — diminishing returns near PA, category-specific peak ages, graceful decline — without
bolting on separate mechanisms.

For each attribute `i` of `DevCategory c`, per monthly step (`dt = 1/12` yr), at player age `y`:

```
target_i = (PA / NORM) · env_c(y − φ)                      # PA-scaled, age-shaped ceiling
rate_i   = { K · E · plast(y−φ) · coaching · minutes · (target_i − a_i)   if target_i > a_i   (growth)
           { K_DEC · (target_i − a_i)                        if a_i > target_i and y ≥ peak_c  (aging)
           { 0                                               otherwise  (precocious youth: hold)
a_i     += step( rate_i · dt + jitter )                     # integer ±1 quantization, §5
```

- **`env_c(y)`** — the category age envelope, a maturation logistic minus an aging logistic (§2.1).
- **`PA / NORM`** — the level scaler. `NORM = max_y Σ_role-weights · env_c(y)` is chosen so a
  *fully-realized* player's peak **best-role CA** equals PA exactly (§2.2). This is the PA gate.
- **`φ`** — a per-player bloomer phase-shift (years), resolved once. `φ > 0` = late bloomer.
- **`E`** — a per-player growth efficiency, resolved once from Determination / Professionalism (§4).
- **`plast(y)`** — a plasticity multiplier falling from 1 to 0 across the early-20s: the *window* in
  which potential can still be realized. Miss it and you flop (§2.3).
- **`K`, `K_DEC`** — base growth / decline tracking rates.

### 2.1 The age envelope `env_c` — resolving the `DevCategory` curves (`ATTRIBUTE_SCHEMA.md` §7)

`ATTRIBUTE_SCHEMA.md` §7 fixes the *qualitative* commitment (physicals peak ~24–27 then decline;
technical/mental grow into the 30s) and defers the numbers to here. The envelope is a rising
maturation logistic minus a later aging logistic, both on the player's age:

```
env_c(y) = clamp( grow_c(y) − loss_c(y), 0, 1 )
grow_c(y) = 1 / (1 + exp(−(y − g_c)/s_c))          # 0→1 maturation
loss_c(y) = Lmax_c / (1 + exp(−(y − d_c)/w_c))      # 0→Lmax aging
```

Fitted parameters (scratchpad `dev_shape`, the starting point the Rust harness re-fits):

| `DevCategory` | `g` | `s` | `Lmax` | `d` | `w` | Envelope peak | Character of the arc |
|---|:-:|:-:|:-:|:-:|:-:|:-:|---|
| **Physical** (`Phys`) | 15.0 | 3.0 | 0.55 | 28.5 | 2.6 | ~23 | matures fast, **real decline** from ~28 |
| **Technical** (`Tech`) | 17.5 | 4.5 | 0.22 | 31.0 | 3.4 | ~29 | slow build, **mild** late decline |
| **Mental** (`Ment`) | 18.5 | 5.0 | 0.16 | 32.5 | 3.8 | ~32 | slowest build, **gentle** decline |

- **Physical** loses a large fraction of peak with age (`Lmax = 0.55`) starting early (`d = 28.5`) —
  Speed/Agility steepest. This is why a physically-reliant career (winger, full-back) is short.
- **Technical / Mental** lose little (`Lmax` 0.22 / 0.16) and late — passing and reading the game hold
  into the mid-30s, so a technical/mental role (deep playmaker, centre-back) ages well. This
  reproduces the schema's "Passing/Finishing hold well into the 30s" without per-attribute special-
  casing: the `DevCategory` tag *is* the curve family, exactly as `ATTRIBUTE_SCHEMA.md` §7 intends.
- **Goalkeepers age gracefully** (schema §7) with no new machinery: GK-weighted CA leans on Handling,
  Command, Positioning, Composure, Concentration — `Tech`/`Ment` attributes on the flat curves —
  and barely on `Phys`, so a GK's best-role CA decays slowly by construction.

The **downward pull only acts past the category's envelope peak** (`peak_c`). Before it, a precocious
youth already above the young-envelope simply **holds** rather than being yanked down to an
age-inappropriate target — the alternative produced 16-year-olds collapsing toward a low
young-envelope in shape-finding, a pure artifact. Worldgen initializes a youth's attributes *on* this
curve (`(PA/NORM)·env_c(15)` + noise), so being advanced-for-age is itself the visible PA signal a
scout reads, and development continues a consistent trajectory rather than fighting the initial state.

### 2.2 PA-gating — decision: gate on **best-role peak CA**, not an attribute budget

`ATTRIBUTE_SCHEMA.md` §4 leaves two options for how PA caps growth. We take its lean:

- **Chosen — best-role-peak-CA gate.** PA is *defined* as peak attainable best-role CA (schema §4), so
  the gate keeps PA and CA **directly comparable on one scale**: a PA of 82 means "this player's
  best-role CA tops out at 82," full stop. The `NORM` normalizer bakes this in — it uses the same
  **role→attribute weights** (`ATTRIBUTE_SCHEMA.md` §5) that define CA, so when every attribute sits
  at its target the role-weighted mean equals PA at the envelope's blended peak. The cap is expressed
  in the currency the whole game already speaks (CA), reuses the one design-once weighting table, and
  needs no second hidden budget.
- **Rejected — position-agnostic attribute budget** (`Σ attr ≤ B(PA)`). It severs PA from any role:
  a player could spend the budget on attributes their role weights at zero, hitting "full PA" while
  their best-role CA stays low — PA and CA stop being comparable, and the schema's clean "PA = peak
  best-role CA" identity breaks. It also double-counts against §5: the role weights already encode
  which attributes matter, so a separate flat budget fights them.
- **Tradeoff acknowledged.** The best-role gate means growth is implicitly steered toward the
  attributes the player's **best role** values (those with weight in `NORM` move CA, so they earn the
  most headroom). That is the intended behaviour — players develop into their position — but it does
  mean a position *change* re-scores headroom. Acceptable: role reassignment is a management action,
  not a per-tick event, and re-deriving headroom on it is cheap.

**Diminishing-returns shape as best-role CA → PA.** Growth rate is **proportional to the gap**
`(target_i − a_i)`, so each attribute approaches its ceiling on an **exponential/geometric** curve —
fast while far below, asymptotically slow as it closes. At the player level, best-role CA approaches
PA on the same shape: the last few points before PA take as long as the whole climb before them. This
is the "diminishing returns near PA" of `DESIGN.md` §4.2 falling straight out of proportional control
— no separate taper term needed. (An explicit multiplicative headroom gate `H = clamp((PA − CA)/H₀,
0, 1)` is the equivalent knob if a sharper collective cap is ever wanted; the proportional form
already delivers the shape and is preferred for having one fewer constant.) Because the target can
never exceed `(PA/NORM)·env`, **CA is structurally incapable of overshooting PA** — the cap is a
property of the target, not a clamp bolted on after.

**Implementation note (25 attributes vs the 3-composite scratchpad).** The `dev_shape` scratchpad
validated the shape on three category *composites*; the literal `target_i = (PA/NORM)·env_c` applied
per-attribute would scale every attribute of a category to the *same* level and so **flatten role
shape** across the 25 real attributes (a centre-back's Finishing would grow to his Tackling, breaking
the position-relative-CA property the whole schema rests on). `fforge-core::development` therefore
keeps the role-weighted `NORM` exactly as specified but multiplies it by a **role-shaped per-attribute
ceiling** — `ceiling_i = pa_base + (w_i−3)·spread`, with `pa_base` solved so best-role CA at the
ceiling equals PA, mirroring `worldgen`'s own weight shaping. Attributes the best role weights at 0
earn no headroom (they never develop). This is the faithful realization of this section's stated
intent ("growth steered toward the attributes the role values"); the §2 pseudocode above is the
single-composite simplification and should be read through this note.

### 2.3 Noise — wonderkids who flop, and late bloomers

Three independent noise sources, each resolved **once per player** except the last, give the career-
arc variety `DESIGN.md` §4.2 and `ATTRIBUTE_SCHEMA.md` §4 require:

1. **Growth efficiency `E`** (per player) `~ N(0.72 + 0.011·(Det−50) + 0.008·(Prof−50), 0.34)`,
   clamped `[0.20, 1.9]`. Low `E` = slow tracking; combined with the closing plasticity window, a
   low-`E` high-PA youth **runs out of runway before reaching PA — the flop**. High `E` overshoots the
   schedule and realizes potential early. Det/Prof shift the mean (§4), so character is a real
   predictor without being destiny.
2. **Plasticity window `plast(y) = 1/(1 + exp((y − 24.5)/2.5))`.** The multiplier on growth that
   closes through the early-20s. It is *why* missing the window is permanent: past ~26 the growth term
   is throttled regardless of headroom, so an unrealized gap stays unrealized. Aging (`K_DEC`) is
   **not** plasticity-gated — decline always applies.
3. **Bloomer phase `φ ~ N(0, 1.8 yr)`** (per player). Shifts the whole envelope in age. `φ > 0`
   delays maturation and the window — the **late bloomer** who keeps climbing into his mid-20s;
   `φ < 0` is the early peaker who is finished young.
4. **Monthly jitter** `~ N(0, 0.35)` added to the rate before quantization — the small month-to-month
   texture, absorbed into the ±1 integer step (§5). It is cosmetic; the career shape is set by 1–3.

The flop/bloomer behaviour therefore lives almost entirely in the **once-resolved per-player
parameters** (`E`, `φ`), not in per-month randomness — which is exactly what lets the event log stay
compact (§5): the trajectory is nearly determined at birth, so little needs recording each month.

## 3. In-scope inputs — keeping "invest in youth vs buy ready-made" a real decision

`DESIGN.md` §4.2 lists five candidate inputs. What Phase 3 models now vs defers, and why:

| Input | Phase 3? | How it enters |
|---|:-:|---|
| **Age** | ✅ core | the envelope `env_c(y)` and the plasticity window `plast(y)` — the spine of the model |
| **Playing time** | ✅ | a `minutes` growth multiplier from the share of available minutes the player got that month (0 → stunted; regular starter → full). Starts as an appeared/benched/absent coarse multiplier; deepens to true minutes once the match loop tracks them |
| **Coaching quality** | ✅ (thin) | a single **per-club coaching coefficient** multiplying growth — the academy-quality lever. Worldgen sets it (default ~1.0); club/facility depth is later |
| **Training focus** | ❌ defer | player-directed per-attribute allocation is a decision-layer / management-UI feature (Phase 4/6). Phase 3 grows attributes toward their role-weighted targets *without* a per-attribute focus knob |
| **Noise** | ✅ | `E`, `φ`, monthly jitter, integer quantization (§2.3, §5) |

**Playing time is the load-bearing strategic input** and the reason it is in scope now: it makes
buying a wonderkid who then rots on the bench a *losing* move, and selling minutes to a prospect a
real cost — the "invest in youth vs buy ready-made" tension `DESIGN.md` §4.2 demands, and a natural
ally of the §4.3 market stabilizer "players wanting minutes." Ready-made players are near their
plateau (little headroom, closed window); youth are pure headroom gated on minutes and `E`. That is
the whole decision, and it is present with just age + playing time + PA.

**Playing-time data source (implementation sub-decision, resolved).** Two options: record each
match's participants in the event schema, or re-derive past lineups at tick time. We take the first —
`Event::MatchPlayed` carries the two XIs (`home_xi`/`away_xi`), the *resolved outcome* the fold reads.
Re-deriving is not replay-safe: a past matchday's effective lineup depends on transient
`pending_lineup` state that is not reconstructable at tick time, and it would duplicate the selection
logic. Recording the XIs (while `Event` was being extended for `DevelopmentTick` anyway) makes
appearances first-class and drift-proof. `GameState` folds them into a per-tick window
(`appearances_since_tick` / `club_matches_since_tick`, reset each tick); the coarse
appeared/benched/absent multiplier reads that window. This is exactly the record-outcomes rule
(`event.rs`), the same one `MatchPlayed`'s score already follows.

**Character attributes feeding development now:** **Determination** and **Professionalism**, via `E`
(§2.3) — precisely the two `ATTRIBUTE_SCHEMA.md` §2 flags as "development-rate drivers the game needs
with or without agents." Professionalism *also* modestly **reduces the physical decline slope**
(`Lmax_Phys` scaled by `1 − 0.3·(Prof−50)/50`): the pro who ages well (§5). **Deferred:**
**Consistency** (match-to-match variance → Phase 2e), **Injury-proneness** (injury events → Phase 2e),
**Leadership** (morale/captaincy → Phase 5) — none is a development-rate driver, so none enters here.

## 4. Natural Fitness — decision: **not** split out in Phase 3

`ATTRIBUTE_SCHEMA.md` §3 flags Natural Fitness "split out in Phase 3 if recovery modeling needs it."
It does not. Resolution: **keep it merged; do not add the attribute yet.**

- Natural Fitness has two jobs: **between-match recovery** and **physical-aging resistance**. Recovery
  is a *fatigue/injury* concern (match-cadence, Phase 2e) — Phase 3's monthly slow loop never touches
  it. So in Phase 3 the attribute would have exactly **one** consumer: aging resistance.
- That single job is **already covered by Professionalism** (§3: it slows physical decline) plus
  Injury-proneness. Adding a hidden Natural Fitness field now would be a new attribute earning its
  keep in one place another attribute already occupies — a violation of the schema's own lean-and-add
  / rule-of-three discipline (`ATTRIBUTE_SCHEMA.md` §3, `DESIGN.md` §2).
- **Revisit at Phase 2e**, when between-match recovery and fatigue-carryover are actually modeled and
  Natural Fitness would have a genuine *second* consumer distinct from Professionalism. Splitting is
  cheap-to-reverse-upward (schema §3); merging now costs nothing and keeps the hidden-attribute set
  minimal. Flagged here so the split isn't silently defaulted either way.

## 5. Determinism & the event-log seam — the architectural crux

Attributes are fixed at worldgen today; development mutates them monthly across a decade and thousands
of players. The seam must keep the append-only log **bounded** and replay **bit-identical**. This is
the same *record-don't-re-derive* tension the match stream faced (`MATCH_MODEL.md` §7), and it is
resolved by the two principles `event.rs` already codifies.

**The event.** A monthly `DevelopmentTick`, emitted by the calendar advance alongside
`MatchdayAdvanced` (cadence: monthly, per `DESIGN.md` §4.2):

```rust
Event::DevelopmentTick {
    date: GameDate,
    changes: Vec<AttrStep>,          // only the attributes that actually moved this month
}
struct AttrStep { player: PlayerId, attr: Attribute, delta: i8 }   // usually ±1
```

`GameState::apply(DevelopmentTick)` folds it by **adding the recorded steps** to each player's
`Attributes` — no RNG, no growth math inside `apply` (fforge-core invariant 2 preserved), players
visited in `BTreeMap`/id order (domain hard-constraint 2). CA is *not* stored; `current_ability()`
re-derives it from the mutated attributes (domain hard-constraint 1 preserved). All growth
computation lives in `commands::step`, which *produces* the tick from
`derive_stream(seed, DEV_STREAM_NS | month_index)` over the current `GameState`.

**Record the resolved changeset, not the seed — the crux, with the tradeoff.** Two designs exist:

- **(A) `DevelopmentTick { date, seed }`, fold re-derives deltas.** Minimal log (12 tiny events/yr).
  But re-deriving on every load means any later change to the growth math **silently rewrites every
  recorded career** — the exact failure `event.rs` rejects for worldgen ("improving worldgen would
  corrupt every old save") and the match engine ("upgrading the engine can never rewrite history").
- **(B) `DevelopmentTick { date, changes }` records the resolved integer steps; the fold only applies
  them.** Drift-proof by construction — the growth model can evolve freely and no recorded career
  moves. Cost: the log grows with actual ability change.

**We take (B)**, for exact consistency with the two `event.rs` principles (record resolved values;
record outcomes the fold consumes without re-running engines). The seed still exists — it is the
production-time RNG source *inside* `commands::step` — it is simply not the stored payload, the same
split as the match RNG feeding `MatchPlayed`'s recorded score.

**Why (B) stays bounded** — the "record resolved values once, not per-attribute-per-month deltas"
refinement:

1. **Monthly cadence**, not per-match-minute — 12 ticks/yr.
2. **Integer-quantized, sparse steps.** Attributes are `u8`; monthly growth is fractional (~0.1–0.5
   pts). Rather than carry a hidden fractional reservoir per attribute — itself derived state needing
   persistence or re-derivation, the very drift trap we are avoiding — each attribute's continuous
   monthly rate `r` becomes a **seed-driven Bernoulli ±1 step with probability `|r|·dt`**. A tick then
   records **only the attributes that actually crossed an integer that month**: most step 0 and are
   absent, a developing teenager posts a handful, a plateaued veteran almost none. The log grows
   **linearly in true ability change** — the irreducible information content of a decade of careers,
   uncompressible below this without re-derivation. Arithmetic (scratchpad): a full career is ~25
   CA-points × ~20 moving attributes ≈ a few hundred ±1 steps over ~250 months; at thousands of
   players, a few MB/decade in SQLite. Bounded and cheap.
3. **Per-player trajectory parameters recorded once, not monthly.** `E`, `φ`, and the club coaching
   coefficient are resolved at worldgen (or youth-intake generation) and ride in the **`World`
   snapshot `GameStarted` already records** (`event.rs` principle 1). The monthly tick carries no
   per-player parameters — only the resolved steps. Because the trajectory is nearly determined by
   these once-resolved values (§2.3), each month's changeset is small.

**How replay reconstructs identical histories.** *Faithful replay* folds `changes` — pure integer
addition — reproducing every attribute at every date **exactly, independent of the growth-math
version** (drift impossible), the identical guarantee `MatchPlayed` gives scores. *Genesis
re-simulation* (calibration/debug) may instead re-run the growth math from the world seed; it is
bit-identical **same-build** (`rng.rs`'s stated bar) and cross-build drift is acceptable there — the
same calibration-vs-authoritative split the match stream draws (`MATCH_MODEL.md` §7): calibration
re-derives freely, authoritative replay reads the record.

## 6. Validation targets

The career-arc harness — the development sibling of `match_engine::calibrate` — simulates a decade+
and checks emergent career statistics, the `DESIGN.md` §4.2 discipline ("validate by simulating a
decade and checking career arcs"). Fitted starting point (`dev_shape`, 4000-player synthetic cohort)
and its readings:

| Metric | Fitted reading | Target |
|---|---|---|
| Peak age — Physical composite | ~25–26 | 24–27 |
| Peak age — Technical (plateau onset) | ~29 | late 20s, holds into 30s |
| Peak age — Mental (plateau onset) | ~30–32 | early 30s, holds |
| Overall best-role CA peak age | ~27–28 | mid–late 20s |
| PA attainment (peak CA / PA) — mean | ~0.88 | 0.85–0.92 |
| PA attainment — p10 / fraction < 0.80 | ~0.78 / ~13% | a real underperforming tail |
| Veteran decline, Physical (30→35) | ~ −2.7 CA/yr | clearly negative |
| Veteran decline, Mental (30→35) | ~ +0.3 CA/yr | ≈ flat / slightly positive |
| Wonderkid (PA ≥ 80) hit rate (≥ 0.90 PA) | ~56% | most, not all |
| Wonderkid flop rate (< 0.75 PA) | ~4% | a small but real flop rate |

**Peak-age metric note.** For a category that barely declines (Technical/Mental), a raw `argmax` of
the composite over age drifts late on the flat plateau and is a poor estimator. The harness measures
**plateau onset** — the age the composite first reaches 98% of its career maximum — which is the
decision-relevant "when does this player arrive?" and is stable. Physical, which genuinely declines,
is checked by both onset and the post-peak slope.

**Calibration lesson banked from the match model** (`MATCH_MODEL.md` §8): a single synthetic cohort
is a noisy estimator, and scratchpad `worldgen` is not the Rust `worldgen`'s attribute distribution.
**Pool over many world seeds** and re-fit the knob table against real `worldgen` — the `dev_shape`
numbers are the notebook's fitted point, expected to shift on the real distribution exactly as
`b_beat` did for the match engine. The knobs (`ENV` params, `K`, `K_DEC`, plasticity `(24.5, 2.5)`,
`E` mean/spread, `φ` spread) group into a `DevKnobs` table, the sibling of `match_engine::Knobs`.

**First real-`worldgen` re-fit already banked (`K_DEC`).** The scratchpad fitted `K_DEC = 1.0` on
env-consistent-from-youth careers. The Rust engine instead starts from `worldgen`'s mid-career squads,
which seed veterans *above* the aging envelope; at `K_DEC = 1.0` the proportional pull crashed their
physicals ~20 pts in a few seasons. `DevKnobs::default` ships `K_DEC = 0.30`, which gives a believable
early-30s decline from a mid-career start (a ~−4 CA/yr Speed slope over 3 seasons in `fforge-core`'s
`development_ages_veterans_and_respects_pa` test) — the `b_beat`-style single-field re-tune this model
expected. The from-youth env-consistent slope stays gentler still.

**Market-pathology hooks (Phase 4, noted now).** The same harness feeds the transfer-market pathology
checks `DESIGN.md` §4.3 wants — talent-inflation and wonderkid-hoarding are development×market
interactions, and a development engine that produces a sane PA-attainment distribution is the
precondition for a sane market. Flagged so the harness is built with that second consumer in mind.

## 7. Open sub-questions

Deliberately unresolved, to settle during the Rust port or Phase 3/4 calibration:

1. **Integer-quantization vs a persisted fractional reservoir.** §5 chooses seed-driven Bernoulli ±1
   steps to avoid hidden fractional state. If the resulting month-to-month granularity ever looks too
   jumpy in the UI, an explicit per-attribute fractional accumulator *recorded in the tick* (not
   re-derived) is the fallback — larger log, smoother curves. Deferred; quantization is the lean.
2. **Youth intake / regens.** This note models the development of *existing* players. Where new
   youth cohorts come from each season (a worldgen-at-runtime generator vs a fixed pool draining) is a
   Phase-4 squad-continuity question. The per-player parameters (`E`, `φ`, PA) they carry are defined
   here; *when and how many* are generated is not.
3. **Coaching coefficient depth.** Modeled now as one per-club scalar. Whether it should split by
   `DevCategory` (a fitness coach vs a technical coach) or by age band is a later texture question,
   not structural — it multiplies the same growth term.
4. **Playing-time granularity.** Starts coarse (appeared / benched / absent). Whether true minutes,
   competition weighting (cup vs league), or a loan-move multiplier earn their keep is a calibration-
   taste question the market phase will pressure.
5. **Position-change re-scoring cost.** The best-role gate (§2.2) re-derives headroom when a player's
   role changes. Whether a retraining penalty (temporary `E` drop on a position switch) is worth
   modeling, or whether instant re-scoring is fine, is deferred to when role reassignment is a live
   management action.
6. **Does the `E`↔character coupling strength calibrate?** `E`'s mean shifts with Determination /
   Professionalism (§2.3). Whether that coupling is strong enough to make character a *visible*
   scouting signal without making it deterministic is a Phase-3 calibration call — the development
   analogue of the match model's support-term-weight question (`MATCH_MODEL.md` §10 item 2).
