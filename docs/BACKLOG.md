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
| 3 — Player development | implemented, calibrated, guarded — **one open defect, §2** |
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

**Most recent work.** W1 of the wonderkid investigation is merged — measurement only, no `worldgen`
or knob changes. It confirmed the attainment floor, refuted the stronger "no knob can produce a
flop" claim (5.1% of the cohort is born below 0.75, and a growth-disabled probe surfaces 2.7% of
it), and landed the W1b projection machinery.

---

## 2. Critical path — the wonderkid seeding fix

**This is the only thing blocking Phase 5, and it should be finished before `AGENT_MODEL.md` is
written.**

`worldgen::gen_player` derives `potential = best_ca + headroom` — CA first, PA bolted on — while
`DEVELOPMENT_MODEL.md` §2.1 has always specified the opposite: seed a player on the age envelope
beneath a drawn ceiling. Two consequences: the wonderkid flop rate is structurally near-zero, and
**PA is recoverable from (CA, age) to within a few points**, which leaves Phase 5's scouting
fog-of-war with nothing to hide and the agent ablation's decision-quality axis with no range.

Full analysis in `WONDERKID_FLOP_DIAGNOSIS.md` and its W1 amendment.

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

1. **Pin the seeding rule** in `DEVELOPMENT_MODEL.md`: envelope-consistent seeding is normative; PA
   is a primary drawn quantity anchored on club quality; seeding reads `env_c(age − φ)`, which is
   what makes PA non-trivially inferable. Record the divergence and its resolution.
2. **Invert the draw** in `gen_player`, reusing `development`'s existing ceiling/`NORM` machinery
   rather than re-encoding the envelope. `youth_discount` and `headroom` both disappear.
   `pool::youth_cohort` inherits it. Expect a world re-roll and a re-pinned golden baseline; a
   re-roll is not a re-fit.
3. **Re-fit `DevKnobs`.** `k_dec` first — it currently sits at 0.30 purely to stop
   non-envelope-consistent veterans crashing, and should move toward the scratchpad's 1.0. Then
   `plast_*`, `e_sigma`, `e_min` jointly against flop rate, hit rate, attainment mean, and the
   sub-0.80 tail; they trade against each other and fitting them singly will oscillate. Expect them
   to loosen — the reverse of the re-fit that compensated for the bug.
4. **Re-bank the market harness.** Youth pricing shifts. Read transfer volume explicitly (§4) but do
   not fit toward it.

**Resolve as part of this work:** the cohort admits `start_age ≤ 21`, mixing 16-year-olds (maturity
~0.55) with 21-year-olds (~0.91). Post-fix those populations have structurally different flop
probabilities. Decide deliberately whether the headline metric tightens to `start_age ≤ 18` rather
than retuning to hit 4% on a cohort whose composition the target never contemplated.

---

## 3. Phase 5 — the agent layer

Scoping in `BATCH5_SCOPING.md`. **Blocked on §2** for the fog-of-war dependency.

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

**4.1 — Transfer volume: 1.805/club/window against a 2–5 target.** Survived two re-banks unchanged,
which localises it upstream of both form and tactics — in the clearing loop or the utility policy's
surplus filter. Working hypothesis: with `asking_markup` uniform and every club pricing off the same
omniscient `value()`, the surplus term is a near-constant fraction of value for every candidate, so
`utility = need · surplus` collapses toward `need · value` and clubs converge on the same targets.
**Fog-of-war may fix this for free** by making valuations genuinely divergent — which is an argument
for re-reading it after §2 and after B5.1 rather than fixing it blind.

**4.2 — Mental plateau onset reads 26.4 against an early-30s target**, and the veteran mental slope
reads ~+0.02 against ~+0.3. The same fact seen twice: the Mental envelope's late build and gentle
decline are not surviving into the measured composite. Career-shape fidelity with no Phase 5
consequence. Worth fitting deliberately at some point, not folded into a re-bank pass.

**4.3 — Substitution effects are unmeasured against league play** (see §5.1). The predicted subs
per match, late-match goal share, and gpm effect cannot be read until an AI plan exists.

---

## 5. Known gaps and filed divergences

Things the code does not do that a reader might reasonably assume it does. Recorded so nobody
rediscovers them the hard way.

**5.1 — No AI substitution plan or bench-selection policy.** `ai_pick_lineup` fields every
AI-controlled side with an empty bench and an empty `sub_plan`. The mechanism is fully built and
tested, but nothing generates a plan, so **no AI match in the entire league ever makes a
substitution.** This is `ai_pick_tactics`'s sibling seam left unfilled, and it is probably the
single largest realism gap in the sim right now: fatigue, condition, and the three-substitution cap
are all live and none of them ever bites for nineteen of twenty clubs. It also blocks §4.3's
measurements. Not hard — the rule vocabulary already expresses forced-cover, fatigue, and
chase/hold plans.

**5.2 — Concentration is designed but not implemented.** `MATCH_MODEL.md` §17 describes both halves
of the `ATTRIBUTE_SCHEMA.md` §9 item 2 resolution (Consistency per-match, Concentration
per-contest); only Consistency ever got an implementation task. There is no `Attribute::Concentration`
reference anywhere in `fforge-core`. The "split holds" verdict is therefore half-tested.

**5.3 — Set pieces.** The one item deferred past 2e. No downstream dependency; the worst
calibration-cost-to-value ratio in the match model. Deferred deliberately, not forgotten.

**5.4 — Stale note.** `MATCH_MODEL.md` §16 states the human side has no bench/plan UI. Batch 4's G3
built one (`fforge-game/src/flows/subs.rs`). Correct the note.

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
