# W1 amendment — what the measurement changed

Amends `WONDERKID_FLOP_DIAGNOSIS.md`. The original stays as the record; this is the correction pass.
Read §3 first if reading only one section — it is the part that changes the plan.

---

# 1. Scorecard

| Claim | Predicted | Measured | Verdict |
|---|---|---|---|
| `r₀` min ≈ 0.70 | 0.70 | **0.700** | ✓ |
| `r₀` mean ≈ 0.80 | ~0.80 | **0.808** (sd 0.041) | ✓ |
| `attainment ≥ r₀` always | bound | **0 violations / 1561 arcs** | ✓ |
| Flops stay 0.00 with growth off | 0.00 | **2.7%** | ✗ **falsified** |
| PA residual sd ≈ 2.3 | 2.3 | **3.18** | ✗ (but see §4) |
| Maturity at 16 ≈ 0.58 | 0.58 | **0.542–0.564** | ≈ |
| Maturity at 22 ≈ 0.98 | 0.98 | **0.897–0.927** | ✗ (gentler) |

**The falsification is mine and it is a real error, not a rounding issue.** §2.1 of the original
correctly derived that `r₀` can reach 0.70 at age 16 with a top-of-range headroom draw — and then
§2.3 asserted a uniform floor "3 points above the flop threshold" as if that corner case did not
exist. It does: **5.1% of the wonderkid cohort is born below 0.75.** The bound `attainment ∈ [r₀, 1]`
was right; "therefore no knob can produce a flop" did not follow, because `r₀` is a distribution, not
a constant. Good catch on the probe design — the growth-disabled run is exactly what exposed it.

---

# 2. The corrected mechanism, which is more interesting than the original

Proportional approach means attainment decomposes cleanly. Define the **career gap-closure fraction**

```
f = (attainment − r₀) / (1 − r₀)
```

Under a pure proportional law, `f = 1 − exp(−∫ k·E·plast·coaching·minutes dy)` — the gap cancels, so
**`f` is scale-invariant**: it does not depend on how far below the ceiling the player started.

From W1's numbers (`attainment − r₀` mean 0.095, `r₀` mean 0.808, so `1 − r₀` = 0.192):

```
f:  mean ≈ 0.50,  p10 ≈ 0.16
```

Half the gap closed over a career, at the median. That is the single most useful number W1 produced,
and it was not directly asked for.

Flopping requires `attainment < 0.75`, i.e.

```
f  <  (0.75 − r₀) / (1 − r₀)
```

| `r₀` | `f` needed to flop | Reachable? |
|---|---|---|
| 0.81 (current mean) | **negative** | **impossible at any knob setting** |
| 0.75 | 0.00 | only at exactly zero growth |
| 0.70 (current min) | 0.167 | just below `f`'s p10 — hence the measured ~0.00 |

**So the corrected claim, which survives:** for the *median* wonderkid, flopping is not merely
unlikely, it is arithmetically impossible — no `DevKnobs` value reaches it, because it would require
negative growth. Only the 5.1% birth-tail can flop at all, and reaching a 4% target would require
roughly 78% of that tail to grow essentially not at all over a whole career, while the median arc
simultaneously gains +0.095 to satisfy the ~56% hit rate. **The hit-rate and flop-rate targets are in
direct conflict given an `r₀` distribution this narrow.** That conflict is the real reason the harness
reads 0.00, and it is a stronger argument for fixing `r₀` than the one I originally made.

One mechanism worth naming, because it is counterintuitive and it explains the *stability* of the
0.00: growth is proportional to the gap, so **the players born furthest below their ceiling grow
fastest in absolute terms.** The model is structurally self-correcting against precisely the
population that ought to flop. That is why the birth-tail vanishes under any nonzero growth setting,
exactly as you diagnosed.

---

# 3. The alarming consequence: the fix as specified will overshoot, probably badly

`f` being scale-invariant cuts both ways. Applying the measured `f` distribution to the maturity
table W1 produced:

| Seeding | `r₀` at 16 | `f` needed to flop | Implied flop rate |
|---|---|---|---|
| Current | 0.81 | negative | 0.00 |
| Env-consistent (W1's measured maturity) | **0.55** | **0.44** | **~40%** |

At `r₀ = 0.55`, the flop threshold sits *just below the median of `f`*. Roughly four in ten
sixteen-year-old prospects would end below 0.75 attainment. Against a 4% target that is an order of
magnitude the wrong way — the mirror image of the current failure.

Two caveats, which push in opposite directions and do not cancel:

- **The pooled estimate overstates it.** The cohort admits `start_age ≤ 21`, and env-consistent
  seeding *raises* `r₀` for older prospects (maturity at 22 is 0.90–0.93, above today's 0.81) while
  lowering it for teenagers. Older prospects also have less plasticity left, so their `f` is lower.
  The two effects partly cancel, and the pooled arithmetic above cannot resolve the age-conditional
  joint distribution of `(r₀, f)`.
- **Scale-invariance breaks in the direction of *more* flops.** `f` is only gap-independent under a
  pure proportional law. With `max_step` quantization, the monthly step is
  `k·E·plast·gap·dt ≈ 0.031·gap` at median knobs — at today's typical gap (~16 points) that is ~0.5
  and the cap never binds; at an env-consistent gap (~38 points) it is ~1.2 and **the cap binds**,
  converting early growth from exponential to linear and *reducing* `f` exactly where the gap is
  largest. So the ~40% figure is, if anything, optimistic.

**This is not an argument against the fix.** The current seeding makes the target unreachable in
principle; that has to change. It is an argument that **W3 cannot land alone.** The original document
treated W4's knob re-fit as follow-up cleanup. It is not — it is co-required, and the growth knobs
will have to move *upward* (looser `plast`, higher `e_min`) in the opposite direction from the
re-fit that produced today's values.

That last point is worth dwelling on, because it retroactively explains the fit history:
`plast_*` was tightened (24.5, 2.5) → (23.5, 2.2) and `e_sigma`/`e_min` widened downward,
specifically *to squeeze a tail out of a floored distribution*. Those moves were correct
compensations for the seeding bug and will be wrong once it is fixed. Expect them to revert toward
the scratchpad's from-youth values — which is exactly what you would predict if the scratchpad was
right and the implementation diverged.

---

# 4. On the residual sd of 3.18

Looser than predicted, and your explanation (the `U(0,8)` term plus base-quality noise in CA) is
right as far as it goes. But **3.18 overstates the genuine uncertainty for the population that
matters**, for a reason worth pinning before it gets used as a baseline.

`headroom` is piecewise: `2(24 − age) + U(0,8)` below 24, `U(0,3)` at and above. A single linear fit
in `age` cannot represent that kink, so part of the 3.18 is **model misspecification** rather than
irreducible conditional uncertainty. Fit within the youth band alone and the residual should collapse
to `8/√12 ≈ 2.31` — the original prediction, for the sub-population where scouting actually happens.

**Recommendation:** before using this as B5.1's before-reading, re-fit restricted to `age < 24` and
report that number instead. Since the whole point of the baseline is "how much does fog-of-war have
to add", a figure inflated by a kink in the veteran band would understate the problem. My guess is
~2.3; worth ten minutes to confirm rather than assume.

Either way the directional claim holds comfortably: **±3 points on PA is not scouting.** Real
uncertainty about a sixteen-year-old's ceiling should be on the order of ±15–20.

---

# 5. Revised task gating

The change: **insert a cheap projection task before W2/W3**, and make W3 read the flop rate rather
than deferring it.

## W1b — Project the fix arithmetically, before implementing it

**This uses data W1 already collected and requires no `worldgen` change.**

Because `f` is (approximately) scale-invariant, the post-fix distribution can be *predicted* from the
existing arcs. For each arc already measured, `f` is known and `start_age` is known. Recompute the
hypothetical seeding ratio `r₀' = maturity(start_age − φ)` from W1's own maturity table, then

```
attainment' = r₀' + f · (1 − r₀')
```

and report the implied hit rate, flop rate, attainment mean, and sub-0.80 tail — **split by
start-age band**, which is the split the pooled arithmetic in §3 cannot do.

This is a paper simulation of W3's outcome at the cost of one pass over data already in hand, and it
resolves the §3 overshoot question before anything disruptive is touched.

**Two honest limitations to report alongside the numbers**, not to work around:

- `f` is only scale-invariant under a pure proportional law; `max_step` quantization and additive
  jitter break it, both in the direction of *lower* `f` at larger gaps. So the projected attainment
  is an **upper** bound and the projected flop rate a **lower** bound.
- `φ` must come from each player's recorded `development` profile, not re-drawn, or the projection
  measures a different population than the one it is predicting for.

**Decision rule:**

| Projected flop rate | Read |
|---|---|
| ≲ 10% | W3 is close to drop-in; proceed as originally planned, W4 as a normal re-fit |
| 10–30% | proceed, but W3 and W4 land **together** in one PR — a seeding change plus a growth re-fit are one change, and splitting them leaves the tree in a state where the harness fails for a known reason |
| ≳ 30% | stop. Either the maturity curve gives too much range (revisit `pa_sigma` and whether seeding should be on a *blend* of `env_c(age − φ)` and today's flatter curve), or the flop target needs re-deriving for a mixed-age cohort. Escalate as design, not fit. |

## Revised sequence

```
W1   ✓ done — floor confirmed, "no knob can flop" falsified, f ≈ 0.50 measured
W1b  arithmetic projection of the fix + youth-band-only PA residual   ← new, cheap, decisive
W2   pin the seeding rule in DEVELOPMENT_MODEL.md
W3   invert the draw in worldgen  ─┐ possibly one PR, per W1b's decision rule
W4   re-fit DevKnobs             ─┘
W5   re-bank the market harness
```

**Amend W3's stop conditions.** The original gated on goals/match moving more than ±0.10. Add: **read
the flop and hit rates immediately after the seeding change, before any knob movement, and compare
against W1b's projection.** If the measured flop rate misses W1b's projection by more than ~2×, the
scale-invariance assumption has broken somewhere and that needs understanding before a re-fit papers
over it.

**Amend W4.** It is no longer "re-fit if needed." Expect `plast_*` to loosen and `e_min` to rise —
the reversal of the re-fit that compensated for the bug. Fit `plast_*`, `e_sigma`, and `e_min`
**jointly** against the flop rate, hit rate, attainment mean, and sub-0.80 tail; they trade directly
against one another through `f`, and fitting them one at a time will oscillate. `k_dec` is separable
and should still go first.

---

# 6. What does not change

The core recommendation stands, and W1 strengthened rather than weakened it:

- The floor is real, dominant, and cannot be fitted around — for the median wonderkid, flopping
  requires negative growth.
- The hit-rate and flop-rate targets are mutually unsatisfiable at `r₀ ≈ 0.81 ± 0.04`. That is a
  proof, not an estimate.
- PA remains recoverable from `(CA, age)` to a few points, which is the finding that actually blocks
  Batch 5.
- The doc-vs-code divergence is still the root cause, and the note still wins.

What changed is the **risk profile**: the fix is a bigger lever than the original document assumed,
and it needs the projection in W1b to size it before it lands.
