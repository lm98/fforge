# Wonderkid Flop Diagnosis — recovered pointer, not the original document

**This file is referenced throughout the codebase as if it exists — `BACKLOG.md` §2 ("Full analysis
in `WONDERKID_FLOP_DIAGNOSIS.md` and its W1 amendment") and `fforge-core::career_arc`'s own W1b module
doc comment (which cites "the amendment's §2/§3/§4/§5" by number) — but it was never committed to this
repository.** A full search of every branch and the complete git history turned up no trace of it. This
stub exists so the reference chain those two sources already assume isn't silently dead, and so the
gap is recorded rather than rediscovered the hard way (the project's own stated convention,
`BACKLOG.md` §5's header). **It is not a reconstruction of the lost analysis prose** — nobody should
treat it as the original W1/W1b writeup. What follows is only what is independently recoverable from
sources that do exist in the repo, cross-checked against a live run of the harness the original
analysis produced.

## What the original evidently covered

Reconstructed from `BACKLOG.md` §2's "Most recent work" summary and `career_arc.rs`'s doc comments —
paraphrased, not quoted, since the originals aren't available to quote:

- **W1 (measurement-only, no `worldgen`/knob changes).** Confirmed the PA-attainment floor
  `DEVELOPMENT_MODEL.md` §6 already flagged, and refuted a stronger claim that no knob combination
  could ever produce a hard flop: 5.1% of the wonderkid cohort is already born with
  `start_ca/PA < 0.75` (a flop before any growth happens), and a growth-disabled probe (`k = 0`,
  `e_base`/`e_min` at floor — `career_arc::run_growth_disabled_probe`) surfaces 2.7% of it even with
  the growth mechanism switched off entirely, i.e. purely from where players are *seeded*.
- **The W1 amendment, §§2–5 (numbered as `career_arc.rs` cites them).** Derives the W1b projection
  method: since the proportional-growth law makes the gap-closure fraction
  `f = (attainment − r0) / (1 − r0)` approximately scale-invariant (amendment §2), the fraction each
  already-traced arc closed can be applied to a *hypothetical* envelope-consistent starting point
  `r0' = maturity(start_age − φ)` without re-simulating anything (§5's decision rule, on the resulting
  projected flop rate: ≤10% close to drop-in, 10–30% implement-and-refit-together, ≥30% stop and
  escalate to design). Two honest limitations the amendment states alongside the method, reproduced
  verbatim from the `career_arc.rs` doc comment that survives it: `f` is only *exactly* scale-invariant
  for a pure proportional law, and `max_step` quantization plus additive jitter both reduce `f` at
  larger gaps — so the projection's `attainment'` is an upper bound and its projected flop rate a
  lower bound, not a point estimate. §3 is the pooled-arithmetic limitation that motivated splitting
  the projection by start-age band rather than reporting one pooled number (the same split
  `DEVELOPMENT_MODEL.md` §8.3 now pins for the real, post-fix headline). §4 is the `age < 24` PA-fit
  restriction `fit_pa_from_ca_age_youth` implements, avoiding the kink in `worldgen`'s piecewise
  `headroom` formula.

## What is verified, not reconstructed

The following are read directly off a live run of the merged harness
(`cargo run --release --bin career_arc`), not reconstructed from prose, and they corroborate the
above closely enough that the normative numbers in `DEVELOPMENT_MODEL.md` §8 can be trusted even
without the original document in hand:

- Wonderkid cohort born below 0.75 PA-attainment: **5.31%** (matches "5.1%" above).
- Growth-disabled probe flop rate: **2.72%** (matches "2.7%" above).
- `fit_pa_from_ca_age_youth` residual_sd: **~2.65** at a 4-seed spot check (the properly-pooled banked
  reading `DEVELOPMENT_MODEL.md` §8.4 cites is **2.61**).
- W1b projection, pooled overall gap-closure fraction: **~0.416** at the same spot check (§8.6 cites
  the banked **0.412**).
- W1b projection, `start_age ≤ 18` flop rate computed by hand from the per-band table at that same
  spot check: **~0.194**, against the pooled-all-bands **~0.179** — both close to §8.3's banked
  **0.191** / **0.176**.

## Where the normative content actually lives now

`DEVELOPMENT_MODEL.md` §8 is the destination this investigation was gated on reaching — the seeding
rule, the divergence and its resolution, the cohort/headline split, the primary success criterion
(`residual_sd`, not the flop rate), the fixed re-fit procedure, and both escalation clauses. Read that
section; treat this file as a pointer to it and to `BACKLOG.md` §2's task history, not as a source of
new analysis.

If the original `WONDERKID_FLOP_DIAGNOSIS.md` exists somewhere outside this repository (an
uncommitted local copy, a different remote), it should replace this stub — its numbered sections
(§2–§5) are cited by name from `career_arc.rs` and deserve to be restored verbatim rather than
represented secondhand as they are here.
