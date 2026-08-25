# Backlog

The running record of where fforge stands and what comes next. `DESIGN.md` says what the project
*is*; this file says what is **done**, what is **next**, and what is **deliberately not being done
yet**. Items are coarse — each is a piece of work with a clear finish line, not a task list. Task
breakdowns live in the per-batch documents that spawn from them.

**Conventions.** Items are gated: nothing marked *blocked* starts before its blocker resolves.
Anything that changes a subsystem's shape gets its design note amended **first** — the
design-note-first rule is not relaxed for backlog items. When code and a design note disagree, the
note is authoritative and the divergence is filed as a bug (§5 below is where those live).

---

## 1. Status

| Phase | State |
|---|---|
| 0 — Design & data model | complete |
| 1 — Walking skeleton | complete |
| 2a — Match core | complete, calibrated, guarded |
| 2e — Match depth | complete except set pieces |
| 3 — Player development | implemented, calibrated, guarded |
| 4 — Transfer market | complete end to end, calibrated, guarded |
| 5 — Agent layer | **not started** — the next frontier |
| 6 — UI/UX & balancing | evidence gathered (`UI_TOOLKIT_EVIDENCE.md`), decision pending |

**Crates.** `fforge-domain` (pure model) · `fforge-core` (deterministic event-sourced sim; the active
front) · `fforge-game` (CLI — Batch 4 complete: grouped menu, `Sem` colour vocabulary, pure
snapshot-tested screens, inbox, finances, tactics and substitution-plan editors).

**Harnesses.** Four, all pooled over many seeds and guarded by wide-band regression tripwires:
`bin/calibrate` (match), `bin/career_arc` (development), `bin/market` (transfer pathologies), plus
the `fforge-game` snapshot suite. Pooled goals/match reads **2.59** with all of 2e live and
`AI_TACTICS_ENABLED = true`.

**Most recent work.** The wonderkid seeding fix (§2) is fully landed and closed: W3 inverted
`worldgen::gen_player`'s draw (PA first, attributes seeded on the envelope beneath it,
`DEVELOPMENT_MODEL.md` §8.1), W4 re-fit `DevKnobs` to match (§6), and the market harness is
re-banked against both that and S1b's AI substitution/bench policy (`TRANSFER_MODEL.md` §9). Phase 5
is unblocked.

---

## 2. Critical path — the wonderkid seeding fix

**[Closed — all four items below are done. Phase 5 is unblocked; `AGENT_MODEL.md` can be written.]**

`worldgen::gen_player` derives `potential = best_ca + headroom` — CA first, PA bolted on — while
`DEVELOPMENT_MODEL.md` §2.1 has always specified the opposite: seed a player on the age envelope
beneath a drawn ceiling. Two consequences: the wonderkid flop rate is structurally near-zero, and
**PA is recoverable from (CA, age) to within a few points**, which leaves Phase 5's scouting
fog-of-war with nothing to hide and the agent ablation's decision-quality axis with no range.

Full analysis in `WONDERKID_FLOP_DIAGNOSIS.md` and its W1 amendment. **The seeding rule, the
divergence's resolution, the cohort/headline split, the primary success criterion, the re-fit
procedure, and both escalation clauses are now pinned normatively in `DEVELOPMENT_MODEL.md` §8** —
read that section before starting items 2–4 below; item 1 is done.

**Immediate next action: read W1b's projection output.** The machinery is merged
(`run_career_arc_with_projection`, `print_seeding_projection`) and prints its own decision rule. It
predicts the post-fix flop rate arithmetically, per start-age band, from arcs already traced. The
number gates everything below:

- **≤ 10%** — the seeding change is close to drop-in; implement, then re-fit as normal.
- **10–30%** — implement and re-fit in a single change; a seeding change that leaves the harness
  failing for a known reason should not sit on the branch alone.
- **≥ 30%** — stop. The maturity curve gives too much range, or the flop target needs re-deriving
  for a mixed-age cohort. That is a design conversation, not a fit.

**The work, once the gate clears:**

1. **[Done — `DEVELOPMENT_MODEL.md` §8.]** Pin the seeding rule: envelope-consistent seeding is
   normative; PA is a primary drawn quantity anchored on club quality; seeding reads
   `env_c(age − φ)`, which is what makes PA non-trivially inferable. The divergence and its
   resolution are recorded there too — this is a *note-wins* reconciliation, the first on record
   (§8.2 explains why, against the project's two prior code-wins reconciliations).
2. **[Done — `DEVELOPMENT_MODEL.md` §8.1.]** Invert the draw in `gen_player`, reusing `development`'s
   existing ceiling/`NORM` machinery rather than re-encoding the envelope. `youth_discount` and
   `headroom` both disappeared. `pool::youth_cohort` inherits it via a shared `SeedTables`. Golden
   baselines re-pinned; a re-roll, not a re-fit.
3. **[Done — `DEVELOPMENT_MODEL.md` §6's "§8.5's W3+W4 re-fit" subsection, landed together with item 2
   per `WONDERKID_W1_AMENDMENT.md` §5's decision rule.]** `k_dec` moved 0.30 → the scratchpad's 1.0
   first, doubling as the W3 correctness check (veteran slopes barely moved, confirming seeding landed
   env-consistent). Then `plast_*`/`e_sigma`/`e_min` jointly: `(23.5, 2.2)/(0.42, 0.15)` →
   `(31.25, 4.6)/(0.095, 0.61)`. Every accept-band holds (24-seed pool) except the veteran physical
   slope, which — per a stage-1 finding, not a stage-2 fitting failure — reads a plateaued ~−2.0 to
   −2.1 regardless of `k_dec`, likely unreachable without moving `env_phys` (forbidden). Max-step
   saturation checked clean (0.0000 before and after).
4. **[Done — re-banked at 24 seeds × 15 seasons against the T14 table, `TRANSFER_MODEL.md` §9.]**
   Several metrics moved well outside their T14 spread — fee median +19%, fee-inflation ratio
   0.505→1.265, clubs insolvent 5.34/20→0.01/20, clubs hoarding 0.68/20→5.85/20, top-3 share of
   top-20 (late) 0.658→0.335 — attributed to item 3's finance-side mechanism, not S1b's
   substitutions: wages are set once at contract time off `best_ca`, and the seeding invert now
   seeds young players genuinely below their ceiling rather than near it, so wage bills at signing
   run lower across a large, compounding share of every squad — no knob was fit toward this, it is
   the readout of item 2/3 reaching finance through a channel this pass did not touch. `ratings`
   (S1b's own channel into the market, via `MarketContext.form`) has no causal path to wages at all,
   and was already shown to wash out at population scale in the T13 re-bank. Transfer volume held at
   1.880 (sd 0.148) against T14's 1.805 (sd 0.250) — unmoved despite the shock above, corroborating
   §4.1's surplus-collapse hypothesis over a cash-constraint one (insolvency vanished; volume still
   didn't rise). A new youth-valuation cut (`start_age ≤ 18`) is now printed and banked for the first
   time. Full reading in `TRANSFER_MODEL.md` §9's new table.

**Resolve as part of this work:** the cohort admits `start_age ≤ 21`, mixing 16-year-olds (maturity
~0.55) with 21-year-olds (~0.91). Post-fix those populations have structurally different flop
probabilities. Decide deliberately whether the headline metric tightens to `start_age ≤ 18` rather
than retuning to hit 4% on a cohort whose composition the target never contemplated.

**[Resolved — `DEVELOPMENT_MODEL.md` §8.3.]** Yes: the wonderkid hit/flop headline tightens to
`start_age ≤ 18`, reported per band and pooled, `n_wk` printed with every rate. Attainment mean,
the sub-0.80 tail, and attainment p10 stay on the full `≤ 21` cohort — only hit/flop narrow. This
makes the headline harder (projected 0.191 on `≤ 18` vs. 0.176 pooled across all bands), which is
the deliberate consequence of targeting the population the original ~4% figure was actually derived
for.

---

## 3. Phase 5 — the agent layer

Scoping in `BATCH5_SCOPING.md`. **Unblocked — §2 is closed.** The fog-of-war dependency (PA no
longer trivially recoverable from `(CA, age)`, `DEVELOPMENT_MODEL.md` §8.1) is in place;
`AGENT_MODEL.md` can be written next.

Four seams already exist and have each been exercised once: `ClubPolicy`/`RecordedPolicy`,
`NewsItem` provenance (`sources: Vec<EventRef>`), the Event/Trace split, and the pre-commitment
idiom. Phase 5 should be smaller than it looks because of them.

**3.1 — `AGENT_MODEL.md`.** The design note. Must resolve: where invocation happens given a pure
core (staged gathering before the advance, rather than blocking inside it); a cost budget in
calls-per-season with a named fallback for every agent; the fake provider; what fog-of-war masks and
whether the human is fogged like the agents; what a "claim" is structurally, so the validation gate
can parse one; the narrative-effect mapping; and persona/memory/trace shape.

**3.2 — The deterministic half.** No LLM in it: the Gym-shaped `Observation`/`Decision`/`info`
boundary with fog-of-war masking; the provider-agnostic interface plus a deterministic fake provider
and templated fallbacks; trace capture, scenario replay, and the recorded-trace-as-output-cache
property; the validation gate reading `NewsItem.sources`. At the end of this the whole agent
architecture exists, every test runs offline, and the game is unchanged from the player's seat. That
is the right place to inspect the seams before spending a token.

**3.3 — The agent half.** Journalist (read-only, so first — but keep it small rather than polishing
it); narrative feedback; the manager agent via `ClubPolicy`; president/director; pluggable scoring
and the ablation.

**Decide the ablation's decision-quality axes before building the manager agent, not after.**
`DESIGN.md` §9 already narrowed the tactics axis to "matches tactics to squad" rather than
"counter-picks opponent" — non-dominance turned out squad-conditional. The transfer axis needs the
same treatment, and §2 and §4 are both really about whether it has any range in it.

**3.4 — The eval spine is the extractable platform kernel** and the project's stated secondary goal.
It is also the piece most likely to get compressed under pressure to see an LLM say something
football-flavoured. Protect its scope explicitly.

---

## 4. Open calibration questions

Each is filed with a reading, not a fix. None blocks Phase 5 on its own.

**4.1 — Transfer volume: 1.880/club/window against a 2–5 target** (BACKLOG.md §2 item 4's re-bank,
24 seeds × 15 seasons — up from 1.805, well inside both readings' own spread). Survived three
re-banks unchanged now — form, tactics, and this pass's seeding-invert-driven finance shock — which
sharpens the localisation: the same clearing loop and utility policy left volume flat while fee
levels, insolvency, hoarding, and talent concentration all moved sharply. Working hypothesis
unrefuted, and now with a positive corroborating reading, not just an absence of movement: with
`asking_markup` uniform and every club pricing off the same omniscient `value()`, the surplus term is
a near-constant fraction of value for every candidate, so `utility = need · surplus` collapses toward
`need · value` and clubs converge on the same targets — a structural cap independent of valuation
*level*. If volume were cash-constrained instead, insolvency vanishing (5.34/20 → 0.01/20 clubs) and
hoarding quintupling should have loosened it; it didn't move. **Fog-of-war may fix this for free** by
making valuations genuinely divergent — which is an argument for re-reading it after B5.1 rather than
fixing it blind.

**4.2 — Mental plateau onset reads 26.4 against an early-30s target**, and the veteran mental slope
reads ~+0.02 against ~+0.3. The same fact seen twice: the Mental envelope's late build and gentle
decline are not surviving into the measured composite. Career-shape fidelity with no Phase 5
consequence. Worth fitting deliberately at some point, not folded into a re-bank pass.

**[Answered — `bin/calibrate`, 24 seeds, post-seeding-invert.]** Subs/match **0.13** (sd 0.02,
range 0.10–0.18) — far below `MATCH_MODEL.md` §16's +3 to +5/match prediction; late-match (75'+)
goal share **16.6%** (sd 1.1) against a +1 to +3pt prediction; non-XI mean minutes **45.1** (sd 3.6)
against a ~10–20 prediction, well above; pooled gpm **2.50** (sd 0.31) against the 2.59 baseline —
inside a single seed's own spread, effectively unmoved. Read together: the S1b bench/plan policy
fires far less often than predicted (`MATCH_MODEL.md` §16's forced-cover-only-on-injury design is
conservative by construction), so its downstream match-shape effects (late goals, gpm) stayed inside
noise while non-XI minutes rose more than predicted simply because *any* bench player now gets on the
pitch at all, which was previously impossible. Not re-fit — a reading, not a target.

---

## 5. Known gaps and filed divergences

Things the code does not do that a reader might reasonably assume it does. Recorded so nobody
rediscovers them the hard way.

**[Closed — S1b, `MATCH_MODEL.md` §16.]** `ai_pick_lineup` now fills a real bench and default
`sub_plan` for every AI-controlled side (forced-cover, fatigue, and chase/hold rules — the vocabulary
already expressed all three). Measured in §4.3: substitutions fire far less often than the design
note's own prediction (0.13/match against 3–5), so this closes the mechanism gap but the resulting
match-shape effect is small in practice, not the "single largest realism gap" it was filed as.

**5.2 — Concentration is designed but not implemented.** `MATCH_MODEL.md` §17 describes both halves
of the `ATTRIBUTE_SCHEMA.md` §9 item 2 resolution (Consistency per-match, Concentration
per-contest); only Consistency ever got an implementation task. There is no `Attribute::Concentration`
reference anywhere in `fforge-core`. The "split holds" verdict is therefore half-tested.

**5.3 — Set pieces.** The one item deferred past 2e. No downstream dependency; the worst
calibration-cost-to-value ratio in the match model. Deferred deliberately, not forgotten.

**[Closed — S1a.]** `MATCH_MODEL.md` §16's stale note (claiming the human side has no bench/plan UI)
is corrected; Batch 4's G3 UI (`fforge-game/src/flows/subs.rs`) is acknowledged there.

---

## 6. Phase 6 and beyond

**6.1 — The UI toolkit decision.** `DESIGN.md` §10 held egui-vs-Tauri open until the management
screens had been felt in practice. **That evidence is now complete**, including the G3 substitution
builder it was explicitly waiting for: `UI_TOOLKIT_EVIDENCE.md`. The strongest findings are that
four separate screens independently reinvented a workaround for having no second pane, and that one
colour channel is not enough for the squad screen. The document deliberately makes no
recommendation. Phase 6 decides.

**6.2 — Deferred by decision, not by oversight.** Loans, negotiation rounds, transfer clauses, and
multi-league transfers (`TRANSFER_MODEL.md` §1). Player-directed training focus. The graphical match
viewer (Bevy, v2/v3) — the match event stream was designed as a swappable-consumer artifact
precisely so this stays cheap to add later.

---

## 7. How to pick up work here

1. Read the relevant design note first. If it disagrees with the code, the note wins and the
   disagreement is the finding.
2. Check the item's gate. Several of these are ordered for a reason stated in the item.
3. Measurement-only tasks come before disruptive changes. W1 is the pattern: confirm the problem
   exists in isolation before restructuring anything to fix it.
4. Write down falsifiable per-change predictions, so a fix can be confirmed or refuted rather than
   assessed. `TACTICS_MODEL.md` §8 and the W1 amendment are both worked examples.
5. A single synthetic league is a noisy estimator. Pool over seeds and report per-seed spread, never
   just the pooled mean.
