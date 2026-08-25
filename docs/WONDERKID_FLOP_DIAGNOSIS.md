# The wonderkid flop rate is 0.00, and no development knob can fix it

Investigation and proposed resolution for the first of the three model-vs-target gaps T14 filed
(`DEVELOPMENT_MODEL.md` §6). Blocking for Batch 5 (`BATCH5_SCOPING.md` §1.1).

---

# 1. The finding

`career_arc` reads, pooled 8 seeds × 16 seasons:

| Metric | Target | Reads |
|---|---|---|
| Wonderkid (PA ≥ 80) hit rate, attainment ≥ 0.90 | ~0.56 | 0.59–0.65 ✓ |
| Wonderkid flop rate, attainment < 0.75 | ~0.04 | **0.00** |
| PA-attainment sub-0.80 tail | ~0.13 | 0.18 |

**The shape of that triple is the whole diagnosis.** An 18% mass below 0.80 with *exactly zero*
below 0.75 is not a thin tail — a genuinely thin tail with 18% under 0.80 would put roughly 6–9%
under 0.75. Zero means the distribution is **clipped**, not sparse. Something is holding a hard
floor at about 0.78.

`DEVELOPMENT_MODEL.md` §6 already named it, correctly, during the original re-fit:

> **Hard wonderkid flops (<0.75 PA) are ~0 — an attainment floor, not a dead mechanism.** Worldgen's
> youth discount is only ~2 CA/yr, so a prospect starts at CA ≈ 0.76–0.85 of PA, and pre-peak players
> never decline — peak-CA/PA is *floored* above 0.75 by construction.

That note is right and this document does not overturn it. What it adds: the floor is **stronger than
"a worldgen change would be needed"** — it is arithmetically impossible for any `DevKnobs` setting to
produce a flop, which is worth proving rather than asserting — and the flop rate is a **symptom of a
larger structural problem** that lands directly on Phase 5.

---

# 2. Why it is a floor, in three steps

## 2.1 — PA is *defined* as CA plus bounded headroom

`worldgen::gen_player`:

```rust
let (_, best_ca) = best_role(&attributes, &ROLE_WEIGHTS);
let headroom = if age < 24 {
    (24 - age) * 2 + rng.range_i32(0, 8)
} else {
    rng.range_i32(0, 3)
};
let potential = (best_ca as i32 + headroom).clamp(best_ca as i32, 97) as u8;
```

Attributes are drawn first, from club quality. **PA is then derived from CA**, never drawn. So the
starting attainment ratio is

```
r₀ = CA / PA = CA / (CA + h) = 1 / (1 + h/CA)
```

and `h` is bounded above by `2(24 − age) + 8`:

| Age at generation | max `h` | `r₀` for a PA-85 prospect | `r₀` for PA exactly 80, max `h` |
|---|---|---|---|
| 16 | 24 | 0.76 (typical `h ≈ 20`) | 0.70 |
| 18 | 20 | 0.81 | 0.75 |
| 20 | 16 | 0.86 | 0.80 |
| 21 | 14 | 0.88 | 0.83 |

To start below 0.75 you need `h > CA/3` — for a CA-60 prospect, `h > 20`, which only the 16–17
band can produce and only in the top of its `U(0,8)` draw. So `r₀ < 0.75` is already a corner case
before development runs at all.

## 2.2 — Attainment is a maximum, so it is monotone in the starting point

`career_arc` computes `attainment = arc.peak_ca() / arc.pa`. `peak_ca` is the **max** of best-role CA
over the observed arc, and pre-peak players never decline (`DEVELOPMENT_MODEL.md` §2.1: the downward
pull only acts past `peak_c`). Therefore

```
attainment ≥ r₀,  always.
```

Combined with §2.1: **attainment ≥ ~0.76 for essentially every prospect in the cohort.** The floor is
not approximate — it is a bound.

## 2.3 — No `DevKnobs` setting can breach it

This is the part worth stating precisely, because it determines that the fix cannot live in
development at all.

Every growth term in `DevKnobs` — `k`, `e_base`, `e_min`, `e_sigma`, `plast_mid`, `plast_width`,
`coaching_*`, `minutes_*` — multiplies a **non-negative** rate applied to a **positive** gap
(`target_i − a_i` while below target). Setting any of them to zero produces `attainment = r₀`
exactly. Setting them high produces `attainment → 1`. **The knob range is `[r₀, 1]`.** There is no
setting anywhere in the table that reaches 0.75 from a starting point of 0.78.

A sanity check on how far the existing knobs are from mattering. Take the most stunted prospect the
model can produce — `E` at its floor of 0.15, and the `minutes_absent` multiplier throughout (a
teenager who never plays). The growth exponent over a career is

```
∫ k · E · plast(y − φ) · coaching · minutes dy  ≈  0.55 × 0.15 × ~8 × 1.0 × ~0.4  ≈  0.26
```

(the plasticity integral from 16 to 30 is worth roughly 8 year-equivalents at
`plast_mid = 23.5, plast_width = 2.2`). Fraction of gap closed ≈ `1 − e^{−0.26}` ≈ **23%**. For a
PA-85 prospect starting at CA 66, that is +4.3 CA → peak 70.3 → **attainment 0.83**.

So the worst career the development model can produce still lands well above the flop threshold, and
it lands there *because of where it started*, not because of how it grew. The 18% sub-0.80 tail the
harness does report is real and is the plasticity re-fit working — but it is the model operating in
the narrow band between `r₀ ≈ 0.78` and 1.0, and 0.75 is simply outside that band.

**Conclusion: this is a `worldgen` problem, not a `development` problem.** Any attempt to fit it in
`DevKnobs` will fail, and the T14 note's instinct to file rather than chase it was correct.

---

# 3. The flop rate is a symptom; the disease is that PA is trivially inferable

This is the finding I would most want weighed, because it is worth more than the number that led to
it and it lands squarely on Phase 5.

Rearrange the generation rule:

```
PA − CA = 2(24 − age) + U(0, 8)
```

**Given a player's current ability and age, his potential is known to within an 8-point window** —
and the window's centre is a deterministic, publicly-derivable function of age. An observer who can
see CA and a birthday can compute

```
PA ≈ CA + 2(24 − age) + 4      (±4)
```

That is not scouting. That is arithmetic. Three consequences, in ascending order of seriousness:

1. **The flop rate is zero** — §2, the presenting symptom.
2. **PA carries no information beyond CA.** Since PA is a monotone function of (CA, age), ranking
   prospects by PA and ranking them by age-adjusted CA give the *same order*. The hidden attribute
   the whole development model is gated on adds no signal to the decision layer.
3. **Phase 5's fog-of-war has nothing to hide.** `TRANSFER_MODEL.md` §2.6 frames `value()` as
   omniscient ground truth against which a club's perception is a masked observation, and
   `BATCH5_SCOPING.md` §2.4 makes that masking a B5.1 deliverable. But masking PA is pointless when
   PA is recoverable from unmasked CA. To hide anything you would have to mask *current ability* —
   which breaks the game (you cannot pick a team without knowing your own players) and is not what
   any of the design notes intend.

Point 3 is why this blocks Batch 5 rather than merely sitting in the backlog. B5.1 would otherwise be
built as a masking layer over a quantity that is not actually hidden, and the ablation in B5.9 would
score agents on a judgment call that has a closed-form answer.

Note also what this does to the strategic tension `DEVELOPMENT_MODEL.md` §3 says playing time exists
to create — *"invest in youth vs buy ready-made"*. If a prospect's ceiling is knowable and his floor
is 0.78 of it, buying youth is a low-variance, positive-expected-value trade with no downside. The
decision the model was built to pose is not currently posed.

---

# 4. Root cause: the code and the design note disagree, and the note is right

`DEVELOPMENT_MODEL.md` §2.1 already specifies the correct seeding, and has since Phase 3:

> Worldgen initializes a youth's attributes *on* this curve (`(PA/NORM)·env_c(15)` + noise), so being
> advanced-for-age is itself the visible PA signal a scout reads, and development continues a
> consistent trajectory rather than fighting the initial state.

That is **PA-first, envelope-consistent seeding**: draw the ceiling, then place the player on the age
envelope beneath it. `gen_player` does the opposite — CA-first from club quality, then PA bolted on
as a linear age-indexed markup. The note's own phrase *"being advanced-for-age is itself the visible
PA signal a scout reads"* describes exactly the inference problem §3 says is missing, and it is
missing because the implementation never adopted the seeding that would create it.

This is the project's own "docs are authoritative, reconcile divergences" rule with a live case
attached. It is also the same class of finding as T14's RNG-stream reconciliation — except that one
resolved in the code's favour, and this one resolves in the doc's.

**Two more divergences fall out of the same root**, both currently patched with constants that exist
only to absorb it:

- **`k_dec = 0.30`**, with the comment *"`worldgen` seeds veterans **above** their aging envelope (it
  is not env-consistent), so a proportional pull that fast would crash their physicals ~20 pts in a
  few seasons… a from-youth env-consistent career declines gentler still."* The scratchpad value was
  1.0. `k_dec` is currently a correction for the seeding bug, not a fitted decline rate.
- **`youth_discount = 2 CA/yr below 21`**, an ad-hoc linear stand-in for the maturation envelope —
  a second, inconsistent encoding of the same curve `env_c` already defines.

Fixing the seeding lets both revert to something meant rather than compensatory.

---

# 5. The fix — invert the draw

## 5.1 What changes

```rust
// 1. Resolve the dev profile FIRST — seeding needs φ.
let development = resolve_dev_profile(rng, determination, professionalism, dev_knobs);
let phi = development.bloomer_phase();

// 2. Draw PA, anchored on club quality. PA is now a primary quantity.
let potential = rng.normal(club_quality + pa_premium, pa_sigma).clamp(...);

// 3. Seed each attribute at the development law's OWN target for this age,
//    i.e. exactly `ceiling_i · env_c(age − φ) / NORM`, plus noise.
let attributes = seed_on_envelope(rng, potential, role, age - phi, dev_knobs);
```

Step 3 should **reuse `development`'s existing `ceiling_i` / `NORM` machinery rather than
re-implementing the envelope.** That is the single most important implementation constraint here:
worldgen and development must not carry two encodings of the same curve, and the project already has
the precedent (`ai_pick_tactics` reusing `formation_p_wide`: *"no second encoding of how wide this
team is"*). Seeding a player *is* asking the development law where he should be at his age.

`youth_discount` and `headroom` both disappear — the envelope does the first job and the draw does
the second.

## 5.2 What that gives you: the maturity curve

The envelope blend `env_c(y)/NORM` for a roughly equal-weighted outfielder, from the current
`DevKnobs` defaults (hand-computed — **W1 should measure the real per-role values**, these are
indicative):

| Age | maturity `env/NORM` | Implied `r₀` |
|---|---|---|
| 16 | ~0.58 | 0.58 |
| 18 | ~0.73 | 0.73 |
| 20 | ~0.87 | 0.87 |
| 22 | ~0.98 | 0.98 |

Against today's `r₀ ≈ 0.78` flat across the youth band, this opens roughly **20 points of dynamic
range at 16** and correctly closes it by 22 — a 22-year-old who has not developed genuinely *is*
mostly known, which is the right behaviour and is what makes late-round prospects a different asset
class from teenagers.

The 4% flop target then becomes reachable rather than impossible: it requires a low-`E`, low-minutes,
early-`φ` prospect to stall in the low 70s from a start in the high 40s — which is a career the
model can already produce, and currently never gets the chance to.

## 5.3 The part that solves §3 for free: seed on `age − φ`, not `age`

`φ ~ N(0, 1.8 yr)` — the bloomer phase — **already exists**, is already resolved per player, and
already shifts the whole envelope in the development law. Seeding on `env_c(age − φ)` rather than
`env_c(age)` means two 16-year-olds with identical PA are placed at maturity 0.45 (`φ = +2`, late
bloomer) and 0.73 (`φ = −2`, early developer) — a **~24 CA-point spread at the same ceiling**.

Invert that and it is precisely the scouting problem:

```
PA = CA / maturity(age − φ),   φ unobserved
```

An observer seeing a strong 16-year-old cannot tell whether he is a modest talent who matured early
or a great one who matured on schedule. **PA becomes genuinely uncertain from (CA, age)** — with an
uncertainty of roughly ±25% rather than today's ±4 points — and the uncertainty is attached to a
*real per-player trait that also drives his actual trajectory*, so a scout inferring φ is inferring
something true about the player rather than sampling unstructured noise. That is a far better
foundation for B5.1 than a noise term would be, and it costs one argument.

It also makes `DEVELOPMENT_MODEL.md` §2.1's own sentence true for the first time: *"being
advanced-for-age is itself the visible PA signal a scout reads."*

## 5.4 The cheap alternative, and why I would not take it

**Widen the headroom draw**: `headroom = (24 − age) * 2 + rng.range_i32(0, 24)`. Ten lines, no
restructuring, no re-roll of the world beyond the knob, and it would produce flops.

I would reject it, with one caveat.

- It does not fix §3. PA stays a deterministic function of (CA, age) plus flat noise — knowable to
  ±12 instead of ±4. Better, and still not a scouting problem worth an agent's attention.
- The uncertainty it adds is **structureless**: the `U(0,24)` term correlates with nothing about the
  player and predicts nothing about his trajectory. An agent cannot get better at estimating it, so
  it adds variance to the ablation without adding a decision-quality axis. `φ`-based uncertainty adds
  both.
- It keeps `youth_discount` as a second encoding of the maturation curve, so the two will drift.
- It leaves `k_dec = 0.30` as a permanent correction for a bug.

**The caveat, stated honestly:** the cheap fix is genuinely much cheaper, and if W1 or W3 turns up
something that makes the inversion look disruptive beyond appetite, it is a legitimate stopgap that
buys a real flop rate. It should be a fallback taken deliberately, not a default slid into.

---

# 6. What this breaks

**This is the most disruptive change since Phase 2a, and that should be weighed openly.**

| Breaks | Severity | Note |
|---|---|---|
| Every world re-rolls | expected | A pure re-roll is not a re-fit — the T3 `natural_fitness` precedent. Guards are wide-band tripwires and should survive. |
| T5 golden baseline | expected | Re-pin, exactly as T3 did. |
| Youth CA drops sharply (16-year-olds from ~50 to ~36) | **real** | Realistic — teenagers should not be first-team players — but it changes squad depth, `club_avg_ca`, and possibly `ai_pick_lineup` at weak clubs. |
| `career_arc` knobs | **needs re-fit** | `k_dec` in particular should be re-fit upward toward the scratchpad's 1.0 now that its correction is gone. `plast_*` and `e_sigma` were fitted *against the floored distribution* and will move. |
| `market` harness | **needs re-read** | Youth become cheap-and-uncertain instead of cheap-and-known. This is the one place I would expect a genuine surprise. |
| `bin/calibrate` gpm / H-D-A | probably not | Relative squad strength is roughly preserved; the XI at most clubs is unchanged. **Stated as a falsifiable prediction — if gpm moves more than ±0.10, something else is going on and W3 should stop and report.** |

**One speculative upside worth measuring rather than assuming.** `BATCH5_SCOPING.md` §1.2's transfer
volume gap (1.805 vs the 2–5 band) was hypothesised to come from every club pricing off the same
omniscient `value()`, collapsing `utility = need · surplus` into `need · value` and driving
convergent targeting. Widening the CA↔PA relationship changes `project_ca`'s spread across the youth
population, which is a direct input to `ca_eff`. It may move volume. W5 should read it; nobody should
count on it.

---

# 7. Tasks

Gated, stop-and-report, in the house style. **W1 before anything else** — it is cheap and it either
confirms this whole document or kills it.

## W1 — Confirm the floor and measure the inferability baseline

**Goal.** Turn §2 and §3 from arithmetic into measurements, before any code changes.

**Deliverable.** Two new `career_arc` telemetry cuts and one knob probe:

1. **The starting-ratio distribution.** For the wonderkid cohort, report `r₀ = start_CA / PA` —
   mean, sd, p0, p10. §2.1 predicts p0 ≈ 0.70 and a mean near 0.80.
2. **Growth achieved.** Report `attainment − r₀`, and assert it is non-negative for every arc.
   §2.2 predicts a strictly positive distribution, confirming attainment is bounded below by `r₀`.
3. **The knob probe.** Re-run the harness with growth crushed — `e_min` and `e_base` at their
   minimum, `k` near zero — and confirm the flop rate is *still* 0.00 and attainment collapses onto
   `r₀`. This is the decisive test: if flops stay at zero when growth is switched off entirely, no
   `DevKnobs` setting can produce them and §2.3 is proven.
4. **The inferability baseline.** Across all generated players, regress `PA` on `(CA, age)` and
   report the residual spread. §3 predicts a residual sd of roughly 2.3 (a `U(0,8)` window). Record
   it — this is the number B5.1's fog-of-war exists to raise, and it is the before-reading.

**Also report, since it costs nothing here and §5.2 depends on it:** the actual `env_c(y)/NORM`
maturity curve at ages 16, 18, 20, 22 per `Role`, rather than the hand-computed estimates in this
document.

**Stop and report.** If (3) produces a non-zero flop rate, this document's central claim is wrong —
stop, report, and the fix is a `DevKnobs` re-fit after all.

**Scope fence.** Measurement only. No `worldgen` change, no knob change committed.

---

**Claude Code prompt:**

> Read `docs/DEVELOPMENT_MODEL.md` §2, §2.3, §6 and `fforge-core/src/worldgen.rs`'s `gen_player`.
>
> **Context.** `career_arc` reports a wonderkid flop rate (attainment < 0.75) of 0.00 against a ~4%
> target, while reporting 18% of the cohort below 0.80. That shape suggests a clipped distribution
> rather than a thin tail. The hypothesis to test: `gen_player` derives `potential` as
> `best_ca + headroom` with `headroom` bounded by `2(24 − age) + 8`, so a prospect's starting
> attainment ratio `r₀ = CA/PA` is floored around 0.76; and since `career_arc`'s attainment uses
> `peak_ca()` (a maximum) while pre-peak players never decline, `attainment ≥ r₀` always. If both
> hold, no `DevKnobs` value can produce a flop, because every growth knob multiplies a non-negative
> rate on a positive gap and so can only move attainment *up* from `r₀`.
>
> **Task — measurement only. Do not change `worldgen` or commit any knob change.**
>
> 1. Extend `career_arc`'s per-arc record with the starting best-role CA, and report over the
>    wonderkid (PA ≥ 80) cohort: the distribution of `r₀ = start_CA / PA` (mean, sd, min, p10), and
>    the distribution of `attainment − r₀`. Assert `attainment >= r₀` for every arc; if that ever
>    fails, report it — it would mean pre-peak decline is reachable and the analysis is incomplete.
> 2. Add a probe that re-runs the harness with growth effectively disabled (`k` ≈ 0, `e_base` and
>    `e_min` at minimum) and reports the flop rate and the attainment distribution. **This is the
>    decisive test.** Expected: flop rate still 0.00, attainment ≈ `r₀`.
> 3. Across all players generated by `worldgen` (not just the cohort), fit `PA ~ a·CA + b·age + c`
>    and report the residual standard deviation. Expected ≈ 2.3, i.e. PA recoverable from (CA, age)
>    to within a few points. Report the number plainly; it is a baseline for later comparison.
> 4. Report the maturity ratio `env_c(y)/NORM` at ages 16, 18, 20, 22 for each `Role`, using the
>    existing `DevTables` machinery rather than re-deriving the envelope.
>
> **Report all four results and stop.** Do not proceed to a fix. If (2) produces a non-zero flop
> rate, say so prominently — it falsifies the hypothesis and changes what the fix should be.

---

## W2 — Pin the seeding rule in the design note

**Goal.** Resolve the doc-vs-code divergence in the doc first, per design-note-first discipline.

**Deliverable.** A new `DEVELOPMENT_MODEL.md` section (or a substantial §2.1 amendment) pinning:

- **Envelope-consistent seeding is normative.** `worldgen` places a player at the development law's
  own target for his age; the two never carry separate encodings of the envelope.
- **PA is a primary drawn quantity**, anchored on club quality, not derived from CA. The note
  currently specifies the seeding but has never said how PA is drawn — that is genuinely new and
  needs pinning, including `pa_sigma` (how much talent varies within a club) as a named knob.
- **Seeding uses `age − φ`**, and why: it makes PA non-trivially inferable, which is the property
  §2.1's "advanced-for-age is the visible PA signal" sentence already claims and the implementation
  never delivered.
- **The divergence itself**, in the style the `MATCH_MODEL.md` §11 RNG-stream correction is recorded:
  what the code did, what the doc said, which won, and why. Record that `k_dec = 0.30` and
  `youth_discount` were both corrections for this bug and are expected to move.

**Scope fence.** Doc only. No code.

---

## W3 — Invert the draw in `worldgen`

**Goal.** Implement W2's pinned rule.

**Deliverable.** `gen_player` restructured per §5.1: dev profile resolved first; PA drawn from club
quality; attributes seeded at the development law's per-attribute ceiling scaled by
`env_c(age − φ)/NORM`, reusing `development`'s existing machinery; `youth_discount` and `headroom`
removed. `pool::youth_cohort` inherits the change automatically — verify that it does and that
intake players land at plausible CA.

**Then re-run `bin/calibrate` and report**, against the §6 predictions:

- gpm, H/D/A, and the favourite-discrimination guard. **Predicted: gpm within ±0.10.**
- `club_avg_ca` and the squad CA distribution by age band, before and after.

**Stop and report if gpm moves more than ±0.10**, or if any of the four pooled guards fails. A
re-roll should not move league aggregates that much; if it does, something beyond the seeding changed
and it needs diagnosing before the knob re-fit builds on top of it.

**Scope fence.** `worldgen` only. **Do not re-fit `DevKnobs` in this task** — that is W4, and mixing
them makes it impossible to tell a seeding effect from a knob effect. Re-pin the T5 golden baseline
(a re-roll, not a re-fit — the T3 `natural_fitness` precedent).

---

**Claude Code prompt:**

> Read the new seeding section in `docs/DEVELOPMENT_MODEL.md` (landed in W2), plus §2.1, §2.2, and
> `fforge-core/src/worldgen.rs`'s `gen_player`.
>
> **Context.** `gen_player` currently draws attributes from club quality and then *derives*
> `potential = best_ca + headroom`. The design note has always specified the opposite — seed a player
> on the age envelope beneath a drawn ceiling — and the code never adopted it. W1 measured the
> consequences: a hard attainment floor and a PA that is recoverable from (CA, age) to within a few
> points.
>
> **Task.** Restructure `gen_player`:
>
> 1. Resolve `development` (φ, E) **before** attributes — seeding needs φ.
> 2. Draw `potential` from club quality (`normal(club_quality + pa_premium, pa_sigma)`, clamped),
>    as a primary quantity. Add `pa_premium` and `pa_sigma` to the relevant knob struct.
> 3. Seed attributes at the development law's own per-attribute target for `age − φ` —
>    `ceiling_i · env_c(age − φ) / NORM` plus the existing role-weight shaping and noise.
>    **Reuse `development`'s existing `ceiling_i` / `NORM` / `DevTables` machinery. Do not
>    re-implement the envelope in `worldgen`** — two encodings of the same curve will drift, and the
>    project's precedent (`ai_pick_tactics` reusing `formation_p_wide`) is explicit about this.
> 4. Remove `youth_discount` and `headroom`.
> 5. Confirm `pool::youth_cohort` inherits the change and that intake players land at plausible CA.
>
> **Then run `cargo run --release --bin calibrate`** and report goals/match, H/D/A, the
> favourite-discrimination guard, and `club_avg_ca` broken down by age band, before and after.
>
> **Predicted: goals/match moves less than ±0.10**, because relative squad strength is roughly
> preserved and the fielded XI at most clubs is unchanged. **If it moves more than that, or if any
> pooled guard fails, stop and report rather than adjusting anything** — a re-roll should not move
> league aggregates that far, and the cause needs diagnosing before knobs are touched.
>
> **Scope fence.** `worldgen` only. Do NOT re-fit `DevKnobs` — that is the next task, and mixing the
> two makes a seeding effect indistinguishable from a knob effect. Re-pin the T5 golden baseline; a
> world re-roll is not a re-fit (the T3 `natural_fitness` precedent).

---

## W4 — Re-fit `DevKnobs` against the new distribution

**Goal.** The knobs were fitted against a floored distribution. Re-fit them against the real one.

**Deliverable.** Re-run `career_arc` and re-fit toward the §6 targets, in this order:

1. **`k_dec` first.** It is currently 0.30 purely to stop non-env-consistent veterans crashing. With
   seeding fixed, expect it to move substantially toward the scratchpad's 1.0. Fit it against the
   veteran physical slope (~−2.7 CA/yr) and re-check the physical peak age.
2. **`plast_*` and `e_sigma`/`e_min`.** Both were fitted to squeeze a tail out of a distribution
   that had a floor 3 points above the flop threshold. Re-fit against the flop rate (~0.04), the hit
   rate (~0.56), the attainment mean (0.85–0.92), and the sub-0.80 tail (~0.13) **jointly** — they
   trade against each other and fitting them one at a time will oscillate.
3. Re-read W1's inferability regression. **Predicted: residual sd rises from ~2.3 into the double
   digits.** This is the number that determines whether B5.1's fog-of-war has anything to hide, so
   report it explicitly rather than as an aside.

**A measurement-definition question to resolve here, not paper over.** The cohort admits
`start_age ≤ 21`, which mixes 16-year-olds (maturity ~0.58, enormous headroom) with 21-year-olds
(maturity ~0.98, almost none). Post-fix those two populations have structurally different flop
probabilities, and the ~4% target came from a notebook cohort seeded from youth. **Report the flop
rate split by start-age band** and decide deliberately whether the headline metric should tighten to
`start_age ≤ 18`. Do not silently retune to hit 4% on a mixed cohort whose composition the target
never contemplated.

**Stop and report** if the flop rate cannot reach ~0.04 without pushing the hit rate below ~0.45 or
the attainment mean below 0.85. That would mean the maturity curve gives too much range and
`pa_sigma`/the envelope needs revisiting — a design question, not a fit.

---

## W5 — Re-bank the market harness

**Goal.** The market prices off CA and projected CA; both distributions just changed.

**Deliverable.** Re-run `bin/market` at its banked pooling (24 seeds × 15 seasons) and re-read
`TRANSFER_MODEL.md` §9's table. Re-fit `ValueKnobs::beta` / `FinanceKnobs::revenue_per_reputation`
only if a metric leaves its own per-seed spread.

**Read transfer volume explicitly** (`BATCH5_SCOPING.md` §1.2 — 1.805 against a 2–5 target, unmoved
by two prior re-banks). Widening the CA↔PA relationship changes `project_ca`'s spread across the
youth population, which feeds `ca_eff` directly, so this pass may move it. **Report it either way and
do not fit toward it** — if it moves, that is evidence about the convergent-targeting hypothesis; if
it does not, that is evidence too, and B5.1 remains the next place to look.

---

# 8. Ordering

```
W1  measure the floor + inferability baseline     ← cheap, decisive, do first
      ├ flops appear when growth is off → hypothesis falsified, stop and rethink
      └ confirmed → W2 pin the seeding rule in the note
W2  doc
W3  invert the draw in worldgen; re-read bin/calibrate
W4  re-fit DevKnobs (k_dec first, then plast/e jointly); re-read inferability
W5  re-bank the market harness; report transfer volume
────────────────────────────────────────────────
then B5.0 — AGENT_MODEL.md, with fog-of-war designed against a PA that is actually hidden
```

W1 is the gate on the whole sequence and costs almost nothing. W3 is the disruptive one and is where
the appetite question actually lands — if it turns out worse than §6 predicts, §5.4's headroom
widening is the deliberate fallback.

---

# 9. Documentation amendments

| Doc | Change |
|---|---|
| `DEVELOPMENT_MODEL.md` | New seeding section (W2). §6: replace the "attainment floor" note with the resolution and the post-fix readings. Record the `k_dec` and `youth_discount` corrections as consequences of the divergence rather than as fitted values. |
| `TRANSFER_MODEL.md` §9 | W5's re-bank; the transfer-volume reading either way |
| `TRANSFER_MODEL.md` §2.6 | Note that PA is now genuinely uncertain from observables, since the ground-truth-vs-perception framing depends on it |
| `DESIGN.md` §9 | Phase 3's entry should note the seeding correction — it changes what "development is calibrated" means |
| `BATCH5_SCOPING.md` §1.1 | Close, with the resolution |

Record the correction in the style `MATCH_MODEL.md` §11's RNG-stream divergence is recorded: state
what the code did, what the note said, which one won, and why. Here the note wins — which is worth
saying out loud, because the previous two reconciliations both went the other way, and a reader who
has only seen those could reasonably conclude the docs always lose.
