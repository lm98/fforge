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

- **Status:** Phase 3 — *ported, calibrated, and guarded.* Model shape settled in a Python
  scratchpad (per `DESIGN.md` §8, discarded after use, never a port target); `fforge-core::development`
  is the Rust transcription (the monthly `DevelopmentTick` fold), and the career-arc harness
  (`fforge-core::career_arc`, `bin/career_arc`) re-ran the §6 metrics against real `worldgen` +
  the full command pipeline, re-fit the knob table against it exactly as `MATCH_MODEL.md` §8
  re-fitted the match knobs, and pinned the result behind the
  `career_arcs_are_in_a_believable_ballpark` regression test.
- **In scope (this pass):** monthly attribute growth and decline across a full career, gated by
  hidden PA, driven by age, playing time, a club coaching coefficient, the Determination /
  Professionalism character attributes, and noise. Enough noise for wonderkids-who-flop and late
  bloomers. The append-only event-log seam and the career-arc calibration harness.
- **Deferred:** player-directed per-attribute **training focus** (a decision-layer / management-UI
  concern, Phase 4/6); **injuries** and **between-match recovery/fatigue-carryover** (Phase 2e);
  **Consistency, Injury-proneness, Leadership** as development inputs (their homes are match variance,
  injury events, and morale respectively — §4). The knob table below was a plausibility-picked
  starting point; the Rust harness (§6) has since re-fit it against real `worldgen`, exactly as
  `MATCH_MODEL.md` §8 re-fitted the match knobs.

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

**[Divergence, resolved at §8 — note wins.]** `worldgen::gen_player` has never actually implemented
this: it derives `potential` from CA, not the other way around. §8 pins the resolution normatively
and records why this is one of the rare reconciliations that goes against code.

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
| **Playing time** | ✅ | a `minutes` growth multiplier from the share of available minutes the player got that month (0 → stunted; regular starter → full). Regular/rotation/absent bands read on minutes share, with a `minutes_absent_share` floor (`MATCH_MODEL.md` §12, batch handoff T4/§2.8) — inert (every starter still a flat 90) until T10/T11/T12 make partial minutes possible |
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
appearances first-class and drift-proof. `GameState` folds `MatchPlayed.minutes` into a per-tick
window (`appearances_since_tick` — minutes-valued since T4, despite the name — /
`club_matches_since_tick`, reset each tick); the minutes-share regular/rotation/absent
multiplier reads that window. This is exactly the record-outcomes rule (`event.rs`), the same
one `MatchPlayed`'s score already follows.

**Character attributes feeding development now:** **Determination** and **Professionalism**, via `E`
(§2.3) — precisely the two `ATTRIBUTE_SCHEMA.md` §2 flags as "development-rate drivers the game needs
with or without agents." Professionalism *also* modestly **reduces the physical decline slope**
(`Lmax_Phys` scaled by `1 − 0.3·(Prof−50)/50`): the pro who ages well (§5). **Deferred:**
**Consistency** (match-to-match variance → Phase 2e), **Injury-proneness** (injury events → Phase 2e),
**Leadership** (morale/captaincy → Phase 5) — none is a development-rate driver, so none enters here.

**[resolved at Phase 2e: the aging term blends with Natural Fitness — batch-3 T9, `MATCH_MODEL.md`
§13.]** §4's revisit fired for the recovery job (Natural Fitness split out), but the physical-aging
term above named itself a *possible later cleanup, not done* — it is now done, on the same trigger:
once Natural Fitness exists as a real attribute, leaving the aging term Professionalism-only means the
"consummate professional whose body still gives out" (high Prof, low NF) and "trains casually but is
built of iron" (low Prof, high NF) archetypes are unrepresentable, even though both attributes now
exist to represent them. The formula becomes a blend, weighted by a new `DevKnobs` field:

```
Lmax_Phys scaled by  1 − prof_aging_coeff · (w·Prof + (1−w)·NatFit − 50) / 50
    where w = aging_prof_weight
```

`prof_aging_coeff` keeps its calibrated value (§6) — only the term it scales changes, from
Professionalism alone to the blend. **`w = 1.0` is the identity setting**, bit-for-bit equal to the
pre-blend formula above (`aging_prof_weight_one_reproduces_the_pre_split_formula_exactly`), so the
split is verifiable before it is tuned. **Production starts at `w = 0.5`** (§2.4's own starting
value) — `career_arc` read the peak-age/decline-slope table at `w = 0.5` against real `worldgen` and
found no reading moved outside its already-banked §6 band (the population's Prof/NF correlation is
mild by construction — worldgen draws them independently, per `MATCH_MODEL.md` §13 — so the blend's
population-level effect is a widening of the aging-slope *spread*, not a shift of its mean). `w` is
the knob to move at a future re-fit if peak-age bands drift; not touched here, per T9's own scope
fence.

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
- **[resolved at Phase 2e: split out — `MATCH_MODEL.md` §13.]** The revisit fired as designed:
  condition/recovery modeling arrived and no existing attribute could carry the recovery job without
  double-dipping, so `Character` gains a hidden `natural_fitness`. **This note's §3 term was flagged
  as a possible later cleanup, not done — batch-3 T9 has since done it:** the physical-aging-resistance
  scaling is now a Professionalism/Natural-Fitness blend, §3's own updated text below.

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

**Full real-`worldgen` re-fit banked (career-arc harness).** The harness (`fforge-core::career_arc`,
`bin/career_arc`) is now built: it drives the real worldgen → AI-lineup → match → monthly-development
pipeline over many seeds, each a decade-plus, and reads the metrics above off the developed world (per
player: category composites and best-role CA vs age), reporting **per-seed spread**, not just the pooled
mean (the `MATCH_MODEL.md` §8 noisy-estimator lesson). Running it (8 seeds × 16 seasons) moved four
knob groups off their scratchpad points — the shift §6 predicted, the development analogue of `b_beat`:

| Knob group | scratchpad → re-fit | What the real distribution showed |
|---|---|---|
| `env_phys` | `lmax` 0.55→0.60, `d` 28.5→27.0, `w` 2.6→2.3 | physical-composite peak read ~27 and overall CA peak ~31 (both late) because worldgen seeds players *below* target and they climb past the envelope peak; earlier/steeper decline lands physical peak ~26, CA peak ~29, veteran (30→35) physical slope ~−2.7 CA/yr |
| `plast_mid` / `plast_width` | (24.5, 2.5) → (23.5, 2.2) | the scratchpad window never closes hard enough (`plast(30)≈0.10`), so over a decade+ every prospect crawls to PA and no one falls short; the tighter window freezes an unrealized gap past the mid-20s |
| `e_sigma` / `e_min` | 0.34→0.42, 0.20→0.15 | a narrower `E` spread realized potential too uniformly (wonderkid hit rate ~0.75 vs the ~0.56 target); a fatter low tail spreads prospect outcomes into range |

Re-fit readings (8 seeds × 16 seasons; per-seed sd in parentheses): physical peak **~26.1** (0.2),
technical onset **~26.9** (0.2), mental onset **~26.7** (0.3), overall CA peak **~29.4** (0.2), PA
attainment mean **~0.88** (0.01) with p10 **~0.80** and a **~11%** sub-0.80 tail, veteran physical slope
**~−2.7** (0.1), veteran mental slope **~+0.04** (0.01), wonderkid hit rate **~0.65** (0.07).

**T14 re-bank: AI tactics live, development unmoved.** `TACTICS_MODEL.md` §7's policy is now enabled
(`AI_TACTICS_ENABLED = true`), so every match in this harness is played with real per-side tactics
rather than `Tactics::neutral()`. That reaches development through a genuine causal path — tactics
change fatigue, fatigue changes fouls, fouls change cards and suspensions, and suspensions change who
actually plays — so the harness was re-run at the banked 8 seeds × 16 seasons to see whether any of it
survives into career shape. It does not:

| Metric | Banked (T9, tactics neutral) | T14 re-read (tactics live) |
|---|---|---|
| Physical peak age | 26.00 | 25.94 (sd 0.09) |
| Veteran physical slope (30→35) | −2.66 | −2.66 (sd 0.05) |
| PA attainment mean | 0.86 | 0.86 (sd 0.01) |
| Wonderkid hit rate | 0.59 | 0.60 (sd 0.07) |
| Technical plateau onset | ~26.9 | 26.51 (sd 0.13) |
| Mental plateau onset | ~26.7 | 26.38 (sd 0.20) |
| Overall CA peak age | ~29.4 | 29.11 (sd 0.06) |

Every row is inside its own per-seed spread. **No re-fit required** — which is the expected result
rather than a lucky one: development integrates a monthly rate law over years, and the channel tactics
opens onto it (a few extra suspensions redistributing minutes at the margin) is far too small to move a
career-length integral. Recorded so the next person does not have to re-derive that reasoning.

**Divergences between this table's *targets* and what the harness actually reads — pre-existing, and
not introduced by the re-bank.** The T14 run makes four of them explicit, all present in the banked
readings too:

| Metric | Target | Actually reads | Note |
|---|---|---|---|
| Mental plateau onset | early 30s | **26.4** | ~5 years early, and has been since the original re-fit (~26.7) |
| Veteran mental slope | ~ +0.3 CA/yr | **+0.02** | flat rather than slightly positive; banked read +0.04 |
| Wonderkid flop rate | ~4% | **0.00** | the harness produces essentially no flops |
| PA attainment sub-0.80 tail | ~13% | **18%** | fatter than targeted |

The mental-onset and mental-slope rows are the same fact seen twice: the Mental envelope's late build
and gentle decline (§2.2's `d = 32.5`, `w = 3.8`) are not surviving into the measured composite, so
mental attributes plateau with the others in the mid-20s instead of holding into the 30s. That is a
model-vs-target discrepancy worth a look on its own terms, not a calibration drift — it should be
filed and fitted deliberately, not folded into a re-bank pass. Flagged here rather than fixed.

**T9 checked, not re-fit: `aging_prof_weight = 0.5` against this table.** Re-running the same 8-seed
×16-season harness with the blend live (§3's `w = 0.5`) reads physical peak **26.00**, veteran physical
slope **−2.66**, PA attainment mean **0.86**, wonderkid hit rate **0.59** — every metric inside the
band above, no re-fit triggered. Expected: worldgen draws Professionalism and Natural Fitness
independently (`MATCH_MODEL.md` §13), so their population correlation is ~0, and blending two
uncorrelated ~N(50,·) inputs at `w = 0.5` leaves the *population mean* of the blended term
unchanged from Professionalism alone — only individual careers move (the archetype the split exists
to represent), which a population-level peak-age/slope reading is not sensitive to. `w` stays a T14
knob if a later, more targeted archetype check calls for moving it.

**T14 disposition: `w` stays at `0.5`, and the reason it cannot be settled here is worth stating.**
The re-bank pass re-ran this harness (above) and `w` again shows no population-level signal — which
is exactly what the paragraph above predicts, so the re-read carries no new information about it.
Moving `w` needs the "more targeted archetype check" named here: a cohort matched on Professionalism
but *split* on Natural Fitness, tracked individually rather than pooled. That is a new harness cut,
not a re-run of an existing one, so it is left un-taken rather than approximated with the tool at
hand. `w = 0.5` is a live production value backed by a plausibility argument (§3) and bounded by a
guard, not an unvalidated guess — but it is not *fitted*, and this note is here so a later reader
does not mistake "re-banked twice with no movement" for "confirmed".

**Two structural findings the harness surfaced (real-`worldgen` shifts, not knob failures).** Both are
consequences of worldgen seeding squads *near their plateau* rather than on the young envelope the
scratchpad assumed (`gen_player` shapes attributes around club quality, not `env_c(15)`):

1. **Technical and mental plateau-onset collapse to a mid-20s wash.** For a category worldgen already
   seeds near-plateau, the composite barely climbs, so "first age reaching 98% of career max" fires
   early and is insensitive to the envelope's late peak — pushing `env_ment` *later* even moved its
   onset *earlier*. The clean phys < tech < ment maturation *ordering* the from-youth cohort produced
   does **not** survive; onset for the two flat categories is ~27 for both. What survives — and what
   §7 actually cares about for squad-building — is the **aging character**: physicals peak and decline
   hard (~−2.7 CA/yr) while mental holds (~flat), a wide, robust split. The regression guard
   (`career_arcs_are_in_a_believable_ballpark`) therefore pins the veteran-slope split and loose age
   bands, **not** an age ordering.
2. **Hard wonderkid flops (<0.75 PA) are ~0 — an attainment floor, not a dead mechanism.** Worldgen's
   youth discount is only ~2 CA/yr, so a prospect starts at CA ≈ 0.76–0.85 of PA, and pre-peak players
   never decline — peak-CA/PA is *floored* above 0.75 by construction. The notebook's ~4% flop assumed
   from-youth low-on-envelope seeding (CA far below PA), which real worldgen does not do. The genuine
   underperformance signal on the real distribution is the **sub-0.80 tail** (~11%, p10 ~0.80), which
   the plasticity re-fit restored; a true <0.75 flop rate would require worldgen to seed prospects
   further below PA (a worldgen change, out of scope for this knob re-fit).

   **[This is now the fix, not just a filed finding — see §8.]** The wonderkid investigation
   (`BACKLOG.md` §2) picked this exact thread back up: seeding prospects on the envelope beneath a
   drawn PA (rather than deriving PA from a seeded CA) is precisely "worldgen change" this note
   said was out of scope for a knob re-fit. §8 pins it normatively, including a revised reading
   methodology for the flop-rate row below (a `start_age ≤ 18` headline, not the full cohort this
   table's targets were originally read against).

These are banked exactly as the `K_DEC` note above: the design shape is unchanged; the numbers are the
notebook's point, and the real distribution moved them — which is the whole reason §6 asked for the
harness.

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

## 8. The wonderkid seeding fix — normative record

**Status: normative, pinning `BACKLOG.md` §2's critical-path item — the only thing blocking
`AGENT_MODEL.md`.** Full measurement history in `WONDERKID_FLOP_DIAGNOSIS.md` and its W1 amendment:
W1 (measurement-only) confirmed the attainment floor §6 already flagged and refuted the stronger
"no knob can produce a flop" claim (5.1% of the wonderkid cohort is born below 0.75 PA-attainment;
a growth-disabled probe surfaces 2.7% of it even with growth switched off); W1b added the arithmetic
seeding-fix projection now implemented as `fforge-core::career_arc::{run_career_arc_with_projection,
print_seeding_projection, fit_pa_from_ca_age_youth}` and exposed by `bin/career_arc`. This section is
the destination those investigations were gated on reaching, and the reason §2.1's and §6's structural
findings above are marked superseded rather than rewritten in place — the numbers they recorded are a
real, banked reading of the *pre-fix* engine and stay true as that.

### 8.1 The seeding rule

**Envelope-consistent seeding is normative.** `worldgen::gen_player` (and `pool::youth_cohort`, which
calls it) must draw **PA first**, anchored on club quality where CA-relative `base` is anchored today,
and then seed every attribute **beneath that drawn ceiling**, on the envelope: `a_i ≈
(PA/NORM)·env_c(age − φ)` plus the existing seeding noise, for a freshly-resolved per-player `φ` (§2.3's
bloomer phase, drawn at generation time exactly as `resolve_dev_profile` already draws `E`/`φ` for the
development trajectory — seeding and development read the *same* `φ`, not two independent draws).

This is not new machinery to design: it is `target_i`'s own formula (§2), and the maturity ratio
`env_c(y)/NORM` it needs is already built and load-bearing — `career_arc::role_maturity_ratio` (task
4 of the W1 investigation, reused by W1b's projection itself) wraps the identical `EnvTables::env_at`
/ `norms_by_role` machinery `development::tick_changes` computes `target_i` from. **The seeding fix is
"call the growth engine's own ceiling function at generation time," not "encode the envelope a second
way."** `youth_discount` and `headroom` both cease to exist — there is no separate youth-quality
discount (PA already carries youth's headroom) and no separate CA-to-PA gap draw (the gap is now
whatever the envelope says it should be at that age, not a free parameter).

**This is what makes PA non-trivially inferable from `(CA, age)` — and the inferability, not the flop
rate, is why the divergence was Phase-5-blocking.** Under the old CA-first rule, `PA = CA + U(0, 8ish)`
is a small, near-uniform residual on top of an already-known quantity: a scout — human or agent — can
read PA off `(CA, age)` to within a few points for free. Under the new rule, `(CA, age)` alone
under-determines PA, because the same observed CA is consistent with many `(PA, φ)` pairs: a player
who looks like this at this age could be an on-schedule player with a lower PA, or a late bloomer
(`φ > 0`) with a higher one, still climbing. That ambiguity is the entire point — it is what Phase 5's
scouting fog-of-war has to *have something to hide*, and what gives the agent ablation's
decision-quality axis (a manager who reads form/character/underlying trajectory better than a naive
CA-lookup) any range to score against. A near-zero flop rate is a symptom of the same bug and worth
fixing, but a *low* flop rate with genuinely unrecoverable PA would not have blocked Phase 5; a
*high* flop rate with trivially recoverable PA still would have. §8.4 makes this the primary success
criterion explicitly, ahead of the flop rate.

**A cheaper fix was considered and rejected for exactly this reason.** Widening the old `headroom`
draw's range (e.g. `U(0, 24)` instead of `U(0, 8)`) would produce a non-zero flop rate in a handful of
lines, no world-generation restructuring, no re-derivation of `youth_discount`. It is rejected because
it does not touch the actual defect: PA would still be `CA` plus **structureless** flat noise — wider,
but uncorrelated with anything real about the player, so an agent cannot get better at estimating it
and the ablation gains variance without gaining a decision-quality axis. `φ`-driven uncertainty is
categorically different: it is attached to a real per-player trait that also drives the player's actual
trajectory, so inferring it is inferring something true, not sampling noise. The wider-window patch
remains a legitimate *fallback* if §8.6's escalation clauses fire and the full inversion proves more
disruptive than the project's appetite — but it is a deliberate fallback, never a default to slide
into ahead of trying the real fix.

### 8.2 The divergence and its resolution — a pinned record

`worldgen::gen_player` derives `potential = best_ca + headroom` (CA first, `headroom` a young/veteran-
banded uniform draw) — while `DEVELOPMENT_MODEL.md` §2.1 has, since this note's first draft, specified
the opposite: seed a player on the age envelope beneath a drawn ceiling (§2.1's own text: "Worldgen
initializes a youth's attributes *on* this curve... so being advanced-for-age is itself the visible PA
signal a scout reads"). The code has never implemented what the note describes.

**This project has two prior doc-vs-code reconciliations on record, and both resolved *code*-wins —
this one resolves *note*-wins, and the difference is worth stating rather than leaving implicit.**

- `MATCH_MODEL.md` §16's T12 finding: the section drafted a 5-substitution cap; the batch-3 task spec
  implemented 3 (`fforge_domain::MAX_SUBSTITUTIONS`); the note was corrected to 3, "the authoritative,
  more specific instruction superseding this section's original '5'."
- `MATCH_MODEL.md` §11's T14 correction: the section required every 2e draw to come from one shared
  RNG stream; the implementation uses four independent ones; the note was corrected to require the
  four-stream design, because the single-stream rule — if actually followed — would have made every
  feature's identity setting non-local and the bit-for-bit invariant tests it was gating impossible to
  write.

Both of those were cases where the **code encoded a real, deliberate decision** — a newer, more
specific task instruction in one case, a discovered architectural necessity in the other — and the
note was simply stale. **This divergence is the opposite shape.** `gen_player`'s CA-first draw was
never a deliberate decision recorded anywhere: no task spec asks for it, no comment defends it, and it
silently drifted from what §2.1 said from the note's first draft. Whose version wins is decided by
which one the actual, named downstream consumer needs — §8.1's inferability property, which Phase 5's
fog-of-war and decision-quality axis structurally require and which only the note's version delivers.
The tie-break criterion generalizes: a divergence resolves toward whichever side a real, identified
consumer depends on, not toward whichever side happens to be already built.

### 8.3 The cohort and the headline split

The existing §6 table's cohort is `start_age ≤ 21` — attainment mean, the sub-0.80 tail, and
attainment p10 keep reading that full cohort; nothing about those three metrics' population changes.

**Wonderkid hit and flop rates narrow to the `start_age ≤ 18` pool**, reported **per start-age band
(16, 17, 18) as well as pooled**, not only pooled. This is deliberate, not a tightening for its own
sake: the original ~4% flop target (§6's table) was derived for 16–18-year-old prospects specifically
— the population "scouting actually cares about," in `fit_pa_from_ca_age_youth`'s own words — and
`start_age ≤ 21` silently mixes it with 19–21-year-olds who are most of the way up the maturity curve
already (`role_maturity_ratio` reads ~0.80–0.92 of `NORM` by age 20–22, against ~0.55–0.70 at 16–18) and
so have structurally lower flop probability by construction, not by having "made it." Pooling the two
populations and re-tuning to hit 4% on the mixture would be fitting to a target the mixture was never
derived against.

**This makes the headline number harder, and that is recorded as a deliberate choice, not an
oversight.** The W1b projection reads **0.191** on the `start_age ≤ 18` pool against **0.176** pooled
across every traced band (16–20) — the `≤ 18` headline is the harder, more honest number, and is the
one §8.5's re-fit targets.

**The `≤ 18` headline needs `≥ 24` seeds to be readable.** The per-band wonderkid sub-population is
small — a handful of seeds at the harness's default pooling puts `n_wk` per band in the low hundreds,
and a rare-event rate (flop is a tail event even pre-fix) on a sample that size carries real sampling
noise the way `MATCH_MODEL.md` §8 already warned every pooled aggregate does. Any report of the `≤ 18`
flop rate — in a re-fit's own working notes or a future re-bank of this table — **must print
`n_wk` alongside every rate it states**, per band and pooled, so a reader can tell a genuine reading
from a small-sample artifact without re-running the harness themselves.

### 8.4 The primary success criterion

**`fit_pa_from_ca_age_youth`'s `residual_sd` is the headline measurement of this fix — not the flop
rate.** §8.1 already states why: the divergence was blocking on inferability, and `residual_sd` is the
direct, quantitative reading of how well `(CA, age)` alone determines PA. The flop rate is a derived,
noisier, rare-event statistic of the same underlying fix; `residual_sd` is what to actually watch.

**Current reading: 2.61.** This is essentially the uniform `headroom` draw's own standard deviation —
`headroom ~ U(0, 8)`-ish, `sd = 8/√12 ≈ 2.31` — meaning today's entire residual is explained almost
exactly by the one mechanism this fix deletes. Concretely: this fix removes the present, near-total
source of PA-unrecoverability (a small, bounded, uniform draw with no hidden structure — the easiest
kind of noise to invert) and replaces it with the spread of the hidden bloomer shift `φ`, which is not
recoverable from `(CA, age)` without also knowing `φ`.

**Sensitivity estimate.** `dr0'/dy ≈ 0.068/yr` at the youth bands (the `role_maturity_ratio` table's
own slope across 16–22 — steepest near 16–18, flattening toward 21 as the curve nears its 24–27 peak),
giving `dPA ≈ PA·(dr0'/r0') ≈ 9.6` per year of `φ` at a representative youth `r0'`. `φ`'s own spread
(`σ_φ = 1.8` yr, §2.3) times a sensitivity of that order is a real, non-trivial swing in what PA a
given `(CA, age)` is consistent with — not a tight derivation of the exact post-fix `residual_sd`
(the two are order-of-magnitude corroboration, not the same calculation), but the reason to expect a
material rise rather than a marginal one.

**Falsifiable prediction: `residual_sd` should roughly double**, with the effect **strongest at age
16** (`φ` has the most leverage there — the maturity curve is steepest, so a given phase shift moves
`r0'` the most) and **weakest at age 21** (the curve is flattening toward its plateau, so the same
`φ` shift moves `r0'` the least). (Provenance note: `WONDERKID_FLOP_DIAGNOSIS.md`'s original,
pre-measurement diagnosis hand-waved "into the double digits" for this same number, before any real
`r0'`/maturity-curve data existed to sharpen it. The `dr0'/dy`-based estimate above supersedes that
guess with an actual sensitivity computed off the real envelope; the two are not in tension, one is
just earlier and cruder than the other.)

**If `residual_sd` does not rise, the fix has failed at its actual purpose regardless of what the flop
rate reads, and that is an escalation to design, not a re-fit.** A flop rate that lands in band with
`residual_sd` still pinned near 2.6 would mean the mechanism intended to inject hidden variation isn't
actually injecting it — the seeding change would be cosmetic (moving *where* the uniform draw sits in
the formula, not removing what it does to PA's recoverability) rather than the structural fix §8.1
requires.

### 8.5 The re-fit procedure

Fixed in advance because `BACKLOG.md` §2 item 3 correctly predicts the knobs below "trade against each
other and fitting them singly will oscillate," and gives no procedure to stop that. This is the
procedure.

- **Primary — fit to this, and only this:** the `start_age ≤ 18` wonderkid flop rate. Target band
  **0.02–0.08**.
- **Accept-bands — must hold, but are never the fitting target:**
  - `≤ 18` wonderkid hit rate `≥ 0.45`
  - Attainment mean `0.85–0.92`
  - Sub-0.80 attainment tail `0.10–0.20`
  - Physical peak age `24–27`
  - Veteran physical slope (30→35) `−2.2` to `−3.2` CA/yr
- **Read-and-report only — explicitly not fitted:**
  - `residual_sd` — must **not** fall back toward `2.6`. If it does, that is a **stop**, not a trade
    against the primary metric (§8.4).
  - Mental plateau onset and veteran mental slope (`BACKLOG.md` §4.2, already reading ~26.4 and ~+0.02
    against targets of "early 30s" and ~+0.3). **Raising `k_dec` pushes the mental slope *further* from
    its target**, because `k_dec` is the aging-tracking rate for every `DevCategory`'s decline branch,
    not a physical-only knob — accelerating it to fix the physical/veteran crash (below) accelerates
    mental decline too, on a category that is already too flat and plateauing too early. Do not chase
    it in this pass; it is a pre-existing, separately-filed divergence, not this re-fit's job.
- **Order, not a knob-at-a-time grid:**
  1. **`k_dec` alone first** (`0.30` toward `1.0`), judged only against the veteran physical/mental
     slopes (the accept-band and the read-and-report-only mental slope above) — §8.6's own correctness
     check on the seeding change rides on this step.
  2. **Then `plast_mid`/`plast_width`, `e_sigma`, `e_min` jointly** — a coarse grid or coordinate
     descent against a *stated* objective (the primary metric, constrained by the accept-bands), never
     one knob at a time. `DEVELOPMENT_MODEL.md` §6's own banked history is the precedent this generalizes
     from: these three already traded against each other once, during the original real-`worldgen`
     re-fit, and were fit jointly then for exactly this reason.
- **Hard constraint: `env_*` shape does not move.** Peak ages and veteran slopes currently pass, and
  the envelope (§2.1) is what produces *both* the aging character the accept-bands check *and* `r0'`
  — the quantity every W1b projection number and the `residual_sd` prediction above are computed from.
  Moving `env_*` during this re-fit would silently invalidate every number this section pins.

### 8.6 Two escalation clauses

**Max-step saturation.** The required gap-closure fraction on the `≤ 18` cohort rises from the current
`f ≈ 0.412` to `f ≈ 0.62` (+50% relative) under the seeding fix — and the W1b projection's own
documented caveat is that `max_step` quantization *suppresses* `f` at larger gaps, which is exactly the
regime a lower `r0'` (a bigger `1 − r0'` gap to close) enters. **The re-fit must instrument the
fraction of monthly attribute steps clipped at `max_step`, specifically for the 16-year-old-start-age
band** (the band with the largest post-fix gap and the most exposure). If that fraction rises
materially *and* the primary metric (the `≤ 18` flop rate) stalls short of its target band even after
§8.5's procedure, **`max_step` is the binding constraint. It is not on the re-fit knob list above** —
raising it is a structural change to the quantization scheme (§7 item 1), not a value in `DevKnobs`'
existing re-fit surface — **and the task stops and reports** rather than reaching for it.

**`k_dec` as a correctness check on the seeding change itself.** `k_dec = 0.30` exists, per §6's own
banked history, "purely to stop non-envelope-consistent veterans crashing" — i.e. it is a compensating
knob for the *same* CA-first seeding defect this section fixes, just showing up at the veteran end
(worldgen seeds mid-career players near their plateau, above the envelope, rather than on it) instead
of the youth end (the flop rate). **If seeding is genuinely envelope-consistent, veterans start on the
envelope, the aging pull term `K_DEC·(target_i − a_i)` is small by construction (there is little gap to
pull against), and raising `k_dec` toward `~1.0` should barely move the veteran slopes** — exactly
`§8.5`'s first re-fit step, run before anything else, so its result gates the rest. **If raising `k_dec`
still crashes the veteran slopes, the seeding change is incomplete** — some population (a start-age
range, `pool::youth_cohort`'s own generation path, or a re-roll edge case) is not actually landing on
the envelope — **and the re-fit stops** rather than compensating a second time with a suppressed
`k_dec`, which is the exact trap that produced `0.30` in the first place.
