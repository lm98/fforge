# Agent Model — Phase 5 Design Note

The design record for the agent layer (`DESIGN.md` §4.4–§7, Phase 5): where agents are invoked
given a pure core, what they see and may decide, how their output is validated and recorded, how
narrative feeds back into the world, and how all of it is measured. Same spirit as the other
notes: a living artifact that pins decisions and the reasoning behind them.

`DESIGN.md` already settles the *principles* — propose/dispose, the Gym-shaped boundary, the
Event/Trace split, the three feedback rules. This note turns them into shapes concrete enough to
implement, and resolves the seven items `BACKLOG.md` §3.1 lists.

**Draft status.** Every decision below carries one of three marks:

- **Settled** — follows from an existing note, from merged code, or from an owner decision
  (marked as such); restated, not reopened.
- **Proposed** — this note's recommendation, with its reasoning. Stands unless overruled.
- **Open** — a call of taste or scope that the reasoning does not decide. Collected in §14.

---

## 1. Purpose & status

- **Status:** Phase 5 — *draft; no implementation lands with this note.*
- **In scope:** the invocation model; the multi-club control seam in the event log; scouting
  fog-of-war; the provider interface, fake provider and fallbacks; the cost budget; the claim
  structure and validation gate; the narrative-effect mapping; persona, memory and trace shape;
  the evaluation axes and the ablation design.
- **Deferred:** negotiation dialogue and multi-round bargaining (`TRANSFER_MODEL.md` §1 defers
  the rounds themselves); press conferences and any agent↔human conversation; agent-authored
  long-term memory (§10.2); learned fast policies (§2.4); scouting upgrades (§5.2 leaves the
  hook); a PettingZoo-conformant wrapper (§3.4); player agents.

### 1.1 What already exists

Phase 5 is smaller than `DESIGN.md` makes it look, because four seams were built and exercised
before any agent existed. This note mostly connects them.

| Seam | Where | What Phase 5 does with it |
|---|---|---|
| Policy substitution | `club_ai::ClubPolicy`, `RecordedPolicy` | An agent's transfer plan enters exactly as the human's does |
| Pre-commitment | `Lineup.tactics`/`sub_plan`, `TransferDecisionSubmitted` | Every decision is submitted *before* the pure step that consumes it |
| Event/Trace split | `MatchOutcome.stream`, `WindowOutcome`, `news` | Raw agent output is a Trace; only the validated decision is an Event |
| Provenance | `NewsItem.sources: Vec<EventRef>` | The validation gate checks claims against the events they cite |

One thing does **not** exist and `DESIGN.md` §7 assumes it: there is no morale, board confidence
or sentiment anywhere in the model. Narrative feedback currently has nothing to push on. §9
introduces the smallest surface that makes §7 implementable.

---

## 2. The roster

### 2.1 Three kinds of mind

**Settled (owner decision, with the reasoning recorded).** Work is assigned by what it needs, not
by role. There are three kinds of decider, in increasing cost:

| Kind | What it is | What it does here | Cost |
|---|---|---|---|
| **Fast policy** | a pure function in `fforge-core` | *executes*: team sheet, tactics, in-match plan, transfer plan — for every club, every decision | none |
| **Decision model** | a small model that answers typed questions about a state with a probability for each answer | *chooses intent*: a manager's stance, a board's stance | small, local |
| **Language model** | a generative model | *writes*: the journalist's prose and claims | the budget of §6 |

**Decision models** — also called System One models — are a model class built for exactly the
middle row: given a state as text or JSON and a set of typed questions, one forward pass returns
a calibrated probability for every answer. Liquid AI's `d1` (about 3B parameters) and `d1-omni`
(about 600M) are the reference examples, served locally through Ollama. *(Recorded from the
vendor's listing as reported in November 2026; this note depends on the class, not on a
product, and the interface in §7.1 is what the code depends on.)*

Three properties make the class fit this project unusually well:

- **The output cannot be malformed.** There is no text to parse and no schema to violate: the
  answer is one of the options offered. A `ManagerStance` is eight questions with five answers
  each, so every reply is a legal stance by construction.
- **It is cheap enough to ask for every club.** The whole league's managers can have a
  considered stance for a fraction of what three language-model managers would cost.
- **The probabilities are themselves data.** A manager who answers `risk: StrongToward` with
  0.9 confidence about a seventeen-year-old is making a measurable claim about how sure he is,
  where the fog says nobody can be (§5.4). The overconfidence probe stops being an inference
  from behaviour and becomes a number read off the reply (§12.2).

The split exists because the workloads have opposite shapes. A league makes roughly 760
team-sheet decisions and 40 transfer plans a season, almost all routine and combinatorial — pick
eleven from twenty-five — which is what a pure policy is for and what a question-answering model
is poorly shaped for. A season also contains moments where a manager's *character* should show:
what kind of side he builds, how he reacts to a bad run. Those are few, and they are choices
among a handful of options. Only the press actually needs to produce language.

**The connection between execution and intent is a `ManagerStance`** (§2.3): a small, bounded,
typed value that conditions a club's fast policy on every decision until it is next set.

**A language model can stand in for a decision model** — ask it the same questions and parse the
reply — and that substitution is an arm of the ablation (§12.2), not the default.

### 2.2 The roles

| Role | Kind | Decides | Decision value | Baseline |
|---|---|---|---|---|
| **Journalist** | language model | which facts to report, and the prose | `Article { text, claims }` (§8) | `news::TemplateRenderer` |
| **Manager's intent** | decision model | how his side should play and trade | `ManagerStance` (§2.3) | the neutral stance |
| **Manager's execution** | fast policy | team sheet, tactics, plan; transfer plan | `Lineup`; `Vec<TransferDecision>` | the same policy at the neutral stance |
| **Board** (the human's club) | decision model | a season objective and its patience | `BoardStance` (§9.4) | a rule-based stance |

The journalist is built first because it is read-only on the decision side: it exercises the
provider, the trace and the gate with no effect on the simulation until §9's feedback is switched
on. The manager is second and is the ablation's subject. The board is last and smallest.

**The human is the manager of their own club and is never replaced by an agent.** Agents play the
people around them: rival managers, their own board, the press.

**There is no architectural limit on how many clubs are agent-managed.** The number is
configuration (§6). With intent and execution split, the natural default is *all* rivals.

**Scouts are not agents in v1.** Scouting is the fog-of-war function of §5 — a deterministic
degradation of what an observer sees — not a persona that reasons. Scouting *upgrades* that
narrow the fog (§5.2) are a planned extension and need no agent either.

### 2.3 The stance

**Settled (owner decision on the axes and their grain).** A stance is a bounded adjustment to the knob tables the built-in policies already
read (`AiTacticKnobs`, `UtilityKnobs`), plus the one new preference fog makes meaningful:

```rust
pub struct ManagerStance {
    pub mentality: Lean,      // shifts the strength-gap thresholds: readier to attack, or to sit
    pub tempo: Lean,
    pub width: Lean,
    pub pressing: Lean,
    pub rotation: Lean,       // how readily tired legs are rested
    pub youth: Lean,          // weight on succession and potential when buying
    pub ambition: Lean,       // the quality target relative to the club's reputation
    pub risk: Lean,           // which end of a scouted range he prices on (§5.4)
}

pub enum Lean { StrongAgainst, Against, Neutral, Toward, StrongToward }
```

- **Typed and closed**: five steps on eight axes — eight typed questions, five answers each,
  which is the native shape of a decision model's reply. Every value is legal, so a stance
  cannot fail validation; the worst a model can do is choose badly.
- **Chosen by argmax in v1.** The model returns a distribution per axis; the driver takes the
  most probable answer and keeps the whole distribution in the trace. Sampling from it instead
  would give managers more variety and is a one-line change, but argmax makes two runs of the
  same model on the same state agree, which the ablation wants first.
- **Recorded**: `Command::SetManagerStance { club, stance }` → `Event::ManagerStanceSet`, folded
  into `GameState.stances: BTreeMap<ClubId, ManagerStance>`. The policies read it from state, so
  they stay pure and a replay needs no model.
- **`Neutral` on every axis is the identity**: the policies behave exactly as today, bit for
  bit. This is the stance's identity setting and joins the list in `fforge-core/CLAUDE.md`.
- **Each `Lean` maps to a fixed multiplier or threshold shift in a `StanceKnobs` table.** The
  mapping is calibrated once so that no stance dominates — the same non-dominance requirement
  tactics had to meet, and measured the same way (§12.2).

A stance is what a persona *does*: the persona sheet is part of the state the decision model is
asked about. The persona (§10) is who the manager is; the stance is that
character rendered into numbers the simulation can act on.

### 2.4 Trained policies — where they fit, and why not yet

The fast policy in v1 is the existing utility policy, parameterised by a stance. A fast policy
*trained in this simulation* — a small network or tree model, distinct from the general-purpose
decision models of §2.1 — is the obvious next step
and the seam is already the right shape for it: it is another `ClubPolicy` and another lineup
picker, taking the same observation and the stance as conditioning input.

**Proposed: do not build it in Phase 5.** Three reasons, in order of weight:

1. **It needs a training pipeline that does not exist** — rollouts, a reward, a trainer, weight
   storage — and `DESIGN.md` §2 puts the game before the platform. The parameterised utility
   policy gives the whole two-speed architecture with nothing to train.
2. **A learner will find whatever is dominant in the simulation.** That is a hazard for the game
   and a gift for calibration: a policy trained to win is the best dominance detector available,
   and would have found the tactics imbalances in hours rather than two calibration passes. It
   belongs in `fforge-harness` first, as a probe, before it belongs in the game.
3. **Where fog leaves no signal, learning has nothing to learn** (§5.4). The gain over a
   well-set utility policy is real but bounded, and should be measured before it is assumed.

When it is built it enters the ablation as one more rung (§12.2), and the comparison between a
trained policy and a language model on the same decisions is a result worth having either way.

---

## 3. The boundary

### 3.1 Observation, Decision, info

**Settled** (`DESIGN.md` §5): an agent receives a serializable `Observation`, returns a
constrained serializable `Decision`, and never reads world internals. The evaluator additionally
receives `info`, the ground truth the agent was not shown.

**Proposed: there is no new kind of decision.** Every decision an agent can make already exists as
a typed, validated value with a `Command` that submits it and an `Event` that records it:

| Decision | Value | Command | Event |
|---|---|---|---|
| Team sheet, tactics, in-match plan | `Lineup` | `SubmitLineup` | `LineupSubmitted` |
| Transfer plan | `Vec<TransferDecision>` | `SubmitTransferDecision` | `TransferDecisionSubmitted` |
| Manager's intent | `ManagerStance` | `SetManagerStance` (new, §2.3) | `ManagerStanceSet` (new) |
| Article | `Article` | `PublishArticle` (new, §8) | `NarrativeBeat`s (new, §9) |
| Board stance | `BoardStance` | `SetBoardStance` (new, §9.4) | `BoardStanceSet` (new) |

An agent is therefore *a thing that produces `Command`s for a seat the human does not occupy*.
This is the strongest form of "LLM proposes, deterministic system disposes": the agent has no
channel into the world that the human does not also have, and `commands::step` validates both
identically. It also means the legality of an agent's output is already tested — by every
existing `validate_lineup` and `validate_transfer_decisions` test.

**Proposed: `Observation` is one enum with a variant per decision**, each a plain struct built by
a pure function of `(GameState, seat)`:

```rust
pub enum Observation {
    TeamSheet(TeamSheetObservation),   // own squad with condition/availability, the opponent's public record
    TransferWindow(ClubObservation),   // already exists; gains fog (§5)
    Newsroom(NewsroomObservation),     // audience-scoped NewsItems with their EventRefs
    Boardroom(BoardroomObservation),   // table position, finances, the objective, recent results
}
```

`ClubObservation` is reused unchanged in shape. An observation never contains another seat's
pending decision: a manager does not see the opponent's submitted team sheet.

### 3.2 The control seam in the log

Before this seam the log knew one controlled club: `GameStarted.player_club`, a single
`GameState.pending_lineup`, and `LineupSubmitted` with no club field. Everything else was the
built-in policy. Agents need the fold to hold a pending decision for *several* clubs.

**Settled, and implemented** (step 1 of §13):

- `Command::SubmitLineup` and `SubmitTransferDecision` name the `club` they are for, and
  validation runs against that club's squad. A club outside the world is `UnknownClub`.
- `LineupSubmitted` gains `club: Option<ClubId>` with `#[serde(default)]`; `None` means the
  log's `player_club`, so every existing save loads unchanged (hard constraint 5). New events
  always carry `Some`.
- `GameState.pending_lineups: BTreeMap<ClubId, Lineup>` and
  `pending_transfer_plans: BTreeMap<ClubId, Vec<TransferDecision>>` hold what has been
  submitted. Lineups are consumed by the matchday they were submitted for; transfer plans by
  the window's close.
- `market::resolve_window` takes the map of recorded plans. A club in the map replays its plan
  through `RecordedPolicy`; a club not in it runs `UtilityPolicy`.

**The rule for a club that has submitted nothing differs by seat, deliberately:**

| | Team sheet | Transfer plan |
|---|---|---|
| **The human's club** | last sheet used if still valid, else an auto-pick | **nothing happens** |
| **Any other club** | the built-in policy | the built-in policy |

The human's row is unchanged from before this seam: `TRANSFER_MODEL.md` §10's "a human who says
nothing gets nothing" is a decision about the *player's* agency and stays. The other row is what
makes not-submitting a safe fallback for an agent. A non-human club's last sheet is *not* reused:
an agent that stops answering should fall back to a policy that reads today's squad, not to a
frozen team sheet. For that reason `last_lineup` stays a single value, the human's.

An **empty** recorded transfer plan is a decision — "do nothing this window" — and is distinct
from no plan. For the human's club the two coincide; for any other club they do not.

**The core does not record which clubs are agent-controlled, and must not.** Control is a
property of the *driver* (§4), not of the world. The log holds decisions; who authored one is
Trace. This is what makes the ablation "a config change, not a rewrite" (`DESIGN.md` §5): the
same log format serves a human, an LLM, the utility baseline, or a replay.

One consequence is deliberate: a club's fallback when its agent fails is *not submitting*, which
yields the built-in policy by the rule above. There is no separate fallback path to test.

### 3.3 Why this is safe for calibration

With no agent submitting anything, nothing above changes a single draw: the built-in policies run
as today. The golden baselines, every snapshot and all three harnesses stay bit-identical. That
is this note's **identity setting**, and its test is the existing suite passing untouched after
§3.2 lands.

### 3.4 On Gym and PettingZoo

**Proposed: honour the contract's shape, do not build the wrapper.** `observe(state, seat) ->
Observation`, `step(state, command) -> events` and an `info` payload are the Gym contract in all
but name. A literal `reset()/step()` adapter earns its keep only when a second, non-football
environment wants to share the evaluation kernel; `DESIGN.md` §2's rule of three says wait.

---

## 4. Invocation — staged gathering

`fforge-core` is a pure fold and `play_match` cannot pause for I/O (`TACTICS_MODEL.md` §7). An
LLM call is an impure, slow, fallible input in the same class as stdin.

**Proposed: agents are invoked by a driver that lives outside the core, between commands, in
stages.** One matchday:

```
1. GATHER    freeze state S. For every agent seat with a decision due:
                 obs  = observe(S, seat)          pure
                 info = ground_truth(S, seat)     pure, never shown to the agent
2. INVOKE    for every seat, possibly concurrently:
                 raw      = provider.complete(prompt(persona, memory, obs))
                 decision = parse(raw)            or fallback (§7)
3. SUBMIT    in ClubId order:
                 session.execute(Command::Submit…{ club, decision })
                 a CommandError means the decision was illegal → fallback (do not submit)
4. ADVANCE   session.execute(Command::AdvanceMatchday)     the pure step
5. REPORT    journalist stage: gather → invoke → PublishArticle, on the post-advance state
```

**Only model seats appear in stages 1–3.** A fast policy is not invoked by the driver
at all: it runs inside the pure advance, where the built-in policies already run, and reads its
club's stance from `GameState`. On most matchdays stages 1–3 are therefore empty for managers —
a stance is due only at its fixed points and triggers (§6) — and the stage carries the
journalist and, monthly, the board.

Three properties fall out of the staging:

- **Simultaneity.** Every seat in a stage observes the same frozen `S`. No agent sees another's
  decision from the same stage, so measured decision quality never depends on invocation order —
  the lineup-stage analogue of the deferred-acceptance loop (`DESIGN.md` §6).
- **Determinism of everything except the model.** Observation building, prompt assembly, parsing,
  validation and submission order are all pure and ordered. The only nondeterministic input is
  `raw`, and it is recorded.
- **Faithful replay costs nothing.** A recorded log already contains every `LineupSubmitted` and
  `TransferDecisionSubmitted`. `GameState::replay` never reaches stage 2. The log *is* the
  LLM-output cache (`DESIGN.md` §6).

**Where a human is in the loop**, the human's own submission is simply another stage-3 command,
made whenever they like before they advance. Agent stages run when the human presses advance, so
the player waits once per matchday, behind one progress indicator, never mid-match.

**Counterfactual re-simulation** is the same loop started from a replayed prefix: replay to event
*n*, then run stages 1–5 live.

### 4.1 Where the code lives

**Proposed: a new crate, `fforge-agents`, layer 4**, depending on `fforge-core`; `fforge-game`
depends on it optionally. It holds the driver, the provider interface, the personas, the parsers
and the trace store. `TRANSFER_MODEL.md` §6.1 deferred this extraction "to Phase 5's first
agent"; that moment is now, and the harness split has since shown a crate boundary costs little
and enforces what a module boundary only documents.

What goes in `fforge-core` instead, because it is deterministic and the built-in policies need
it too: the control seam (§3.2), observation builders, fog-of-war (§5), claim validation (§8) and
the narrative fold (§9). The test for placement is simple: *if a replay needs it, it is core.*

`fforge-harness` gains the agent evaluation runs (§12), which is where `DESIGN.md` §3 now says
the evaluation kernel should grow.

---

## 5. Fog-of-war

**Settled** (`TRANSFER_MODEL.md` §2.6): `value()` is ground truth and belongs to `info`. A club's
perceived valuation is that same function over degraded inputs — a wrapper, not a rewrite.

### 5.1 What is hidden

**Settled (owner decision).** What an observer knows depends on whose player it is:

| | Own players | Everyone else's |
|---|---|---|
| Performance attributes, and so current ability | exact | **fogged** |
| Potential | **fogged** | **fogged**, more widely |
| Market value | follows from the above | **fogged** |
| `Character`, `DevProfile` | never shown | never shown |

Nobody sees a true potential for anyone, their own squad included. Nobody sees a true current
ability or a true value for a player at another club. **Every fogged quantity is presented as a
range, and the width of the range is the uncertainty.**

### 5.2 The estimate

Each observer holds two hidden offsets per player per season, drawn from its own stream:

```
(z_now, z_pot) = two standard normals from derive_stream(seed, SCOUT_NS | observer | player | season)

attributes seen  = true attributes + σ_now · z_now      one shift for all 25, clamped to the scale
ability seen     = current_ability(attributes seen)     the same function as everywhere else
potential seen   = PA + σ_pot · z_pot                   clamped to [ability seen, 100]
```

- **One shift, not twenty-five.** A scout misjudges a player's *level*, not each attribute
  independently. Shifting every attribute together keeps the player's shape — who is quick, who
  can pass — legible while his overall standing is uncertain, and it means perceived ability is
  `current_ability` of what was seen: one encoding of ability, as hard constraint 4 requires.
- **Deterministic and derived, never stored.** The estimate is a pure function of the seed and of
  facts already in the world, exactly as condition is (`MATCH_MODEL.md` §13). Nothing is added to
  `Player` or `Club`.
- **Stable within a season.** The draw is keyed by season, not by date. Looking twice shows the
  same thing, so repeated looking cannot average the noise away.
- **Independent across observers.** Twenty clubs hold twenty opinions of the same teenager — the
  divergence the transfer market has lacked.
- **For an observer's own players `σ_now` is zero** and `σ_pot` is at its smallest.

**The range.** An observation carries `Range { lo, hi }` for each fogged quantity: the estimate
plus and minus `k · σ`. The value range is `value()` evaluated at the low and high corners, which
is sound because value is monotone in both ability and potential.

**Proposed: `k` is chosen so the truth lies inside the range about nine times in ten, not
always.** A range that always contains the truth is a guarantee, and a guarantee can be gamed —
bid as though the top of every range were real and you can never be badly wrong. A range that is
occasionally wrong is what makes a scouting report a judgement rather than a bound, and is the
only version in which a flop can surprise anyone.

**What narrows it.** `σ` is the product of terms in a `ScoutKnobs` table:

- the player's **age** — a 27-year-old's ceiling is nearly his present;
- **familiarity** — own players least, then players in the same league, widest for the unseen;
- the observer's **scouting level**. In v1 this is derived from `Club.reputation`. It is the
  hook for the planned upgrades: investment in scouting becomes a recorded fact that lowers this
  term, with no change to the model above.

### 5.3 Who is fogged

**Settled (owner decision): everyone who decides** — the human, the fast policies of every club,
and any language model. Three reasons, recorded because the consequences are large:

1. The ablation compares stances and policies against a baseline. If only one side is fogged the
   comparison measures the handicap.
2. Uniform omniscient valuation is the leading explanation for transfer volume sitting below
   target through three re-banks (`BACKLOG.md` §4.1): with every club pricing off the same
   `value()`, clubs converge on the same targets. Divergent estimates are the proposed cure.
3. A human who sees truth while every rival guesses is playing a different, easier game.

**Policies act on ranges, and `risk` says how** (§2.3): a cautious manager prices a target near
the bottom of its range, a gambler near the top, the neutral stance on the midpoint.

**This re-rolls two harnesses.** The transfer policy now reads fogged candidates, and the tactics
policy reads a fogged opponent strength. Predictions, written down first:

- transfers per club per window **rises** from 1.88, toward the 2–5 band;
- talent concentration (`top3_share_of_top20`) **falls** from its omniscient upper bound;
- the buyer's-eye flop rate (paid like a star, peaked as a squad player) becomes non-zero and is
  a new reported row;
- `bin/calibrate`'s league aggregates move **within their per-seed spread**: an opponent's
  strength is a mean over eleven players, so its error is about `σ_now / √11`, and only fixtures
  sitting on a mentality threshold can flip;
- development readings do **not** move — fog touches no engine.

If volume does not rise, the §4.1 hypothesis is refuted and the cap is elsewhere in the clearing
loop. That would be a finding to record, not a reason to turn the noise up.

**In the UI** every fogged figure is shown as its range, never as a point. Squad, scouting and
transfer screens change; the snapshots are re-pinned once. Whether a wide range also gets a
verbal gloss is a presentation detail left to `UI_TOOLKIT_EVIDENCE.md`'s findings.

### 5.4 The finding this must reckon with

`career_arc` measures how well PA can be recovered from what an observer can see. Two attacks:
the **naive** fit on `(CA, age)` and the **competent** fit on the per-category composites plus
age. After the seeding fix the naive residual is 5.57 points — so there *is* something to hide.
But the two attacks are near-identical at ages 16–18 and only separate at 19–21.

Read plainly: **at the ages wonderkid scouting is about, visible data contains almost no
exploitable signal about PA beyond the obvious.** No agent, however good, can see through that
fog by reasoning harder over attributes. Two consequences:

- **The skill available at 16–18 is not estimation, it is decision-making under a known-noisy
  estimate**: how much to pay given a range, how many lottery tickets to hold, when to sell one.
  In stance terms this is the `risk` axis, and the band is where it earns or loses its keep. The transfer axis of the ablation must score *that* (§12.2), not
  estimation accuracy.
- **The 16–18 band is a ready-made overconfidence probe.** An agent that claims to have
  identified a wonderkid from his attributes at 17 is asserting skill that the data provably does
  not support. That is measurable, and it is a more interesting research reading than a win rate.

### 5.5 As implemented

Step 2 of §13 is done: `fforge-core::scouting` and `market::WindowView`. What the code settled
that the sections above left open or got wrong:

- **The asking price is the seller's opinion, not the buyer's.** Before fog every club priced a
  player off one shared value, so "ask" and "value" were the same number scaled by
  `asking_markup`. Under fog that would have made every buyer's surplus a constant fraction of
  its own estimate again. A player's ask is now his *owner's* valuation times the markup — one
  posted price list that every club reads — and a buyer's surplus is its view against the
  seller's. A free agent asks his true worth: he knows himself. This is the mechanism §5.3's
  first prediction rests on, and it was missing from the design until the code needed it.
- **A club appraises only players within its means.** Valuing a player integrates his whole
  projected career and is the market's hot path; under fog all twenty clubs must do their own.
  The buying policy already discards anyone it cannot pay for before reading his value, so
  `club_ai::observe_as` skips the appraisal for those and records `value: Money(0)`.
  `ClubObservation::within_means` is the one statement of that test, shared by both. Appraisals
  are computed on first use and remembered for the window, which cannot change a result because
  a value is a pure function of the frozen view.
- **What it costs.** A window is still about 2.2× slower to resolve than it was, because clubs
  can afford most of the market and so appraise most of it. `valuation::project_ca_series` is
  the place to look if that needs to come down.
- **A sighting is a whole `Player`.** `Scout::sight` returns the player as seen — attributes
  shifted, potential replaced, `Character` and `DevProfile` neutralised to their midpoints — so
  every function that reads a player reads a sighting unchanged, and "same function, degraded
  inputs" is literally what runs. `seen_world` is the world with every player swapped for his
  sighting; `seen_squad` is one opponent's squad alone, which is all a team sheet needs.
- **The identity is exact.** `ScoutKnobs::omniscient()` shows every observer the world itself,
  borrowed rather than copied, and every pre-fog test of the market runs at it unchanged.
- **Where the human sees it.** The squad screen shows potential and value as ranges with exact
  ability; the transfer screens show ability, potential and value as ranges for every player at
  another club. `GameState::scout_reports` is the one source for both: the value range is
  `value()` at the pessimistic and optimistic corners of the sighting.
- **One deliberate exception:** the club picker on the new-game screen still shows every club's
  true squad strength. No club has been chosen yet, so there is no observer to fog for.
- **Not built:** the buyer's-eye flop rate row promised in §5.3. It needs each transfer's fee
  tracked against the player's later peak, which is harness work of its own.

### 5.6 Readings against §5.3's predictions

`bin/market`, 24 seeds × 15 seasons, and `bin/calibrate`, 24 seeds, each run on the commit before
fog and the commit with it. All four pooled guards pass unchanged in both.

| Prediction | Before | After | Verdict |
|---|---|---|---|
| Transfers per club per window **rises** toward 2–5 | 1.880 (sd 0.148) | **4.537** (sd 0.192) | **confirmed** — about fourteen per-seed sds, and inside the band for the first time |
| Talent concentration **falls** | early 0.568, late 0.335 | early 0.525, late 0.388 | **not confirmed** — both moves are inside one sd (~0.09) and point opposite ways |
| Match aggregates move **within their spread** | 2.50 gpm; H/D/A 42.0/28.3/29.7 | 2.50 gpm; 41.9/28.5/29.6 | **confirmed** — nothing moved |
| Development does **not** move | — | — | not re-run; fog touches no engine and the `career_arc` guard passes |

`BACKLOG.md` §4.1's hypothesis — that uniform omniscient valuation, not cash, capped volume —
is confirmed, and that item is closed.

**Two readings nobody predicted, recorded so they are not mistaken for intent:**

- **The median fee halved**, 1.47M → 0.71M (p90 3.10M → 1.84M), and the last-season-to-first
  fee ratio fell from 1.27 to 0.57. With two and a half times as many deals, most of the new
  ones are small: trades of ordinary players that omniscient clubs never made, because every
  club agreed on what such a player was worth and so nobody saw a surplus in him.
- **The league became more balanced.** Late-season points-Gini fell 0.251 → 0.201 and rank
  churn rose 1.31 → 1.51.

**One caveat on the headline.** Some of the new volume may be churn that fog manufactures
rather than trade it enables. A club buys the players it happens to overrate; next season its
estimate is redrawn, the overrating is gone, and the same player looks surplus. That is a
winner's curse followed by a correction, and it would raise volume even if no club were better
off for any of it. The reading above cannot tell the two apart. Telling them apart is what the
buyer's-eye flop row is for, which makes it the first thing to build before leaning on 4.537.

### 5.7 The buyer's eye: is the new volume trade or churn?

§5.6's caveat needs a reading that follows a signing after it is made. `MarketTelemetry` now
keeps every fee-paying transfer as a `Signing` and reports five rows, each one value per seed:

| Row | Definition |
|---|---|
| Fee / true value, median | the fee over what `value()` says of the true player as the window opens. The harness may read truth; the buyer could not |
| Share paying above true value | the same ratio, counted above 1 |
| Share gone within a year | signings the buyer sold or released within 365 days, among those followed that long |
| Resale fee / purchase fee, median | for the ones sold within the year |
| Star signings written off in two years | fee in the seed's top quarter, then released or sold for under half of it within 730 days. This is the flop rate §5.3 promised |

The comparison is the same harness at `ScoutKnobs::omniscient()`, both under the match
calibration of `MATCH_MODEL.md` §8.1, 24 seeds × 15 seasons.

**Predictions, written before either run.** Omniscient: the fee ratio sits at or under 1 and
nobody overpays, because every ask is a markup of at most 1 on the shared true value. Fogged:
the median ratio rises above 1 (1.1–1.3) with well over half of buyers overpaying, since the
club that bids is the one that overrates. If the volume is churn, the share gone within a year
at least doubles against the omniscient run, the resale ratio drops well under 1, and the flop
rate rises from under 10% to 15–30%. If the share gone within a year moves by less than half,
the churn reading is refuted and the volume is trade.

**First reading: the question was wrong.** The omniscient run alone settled something neither
prediction covered. With no fog at all, 80% of signings left the buyer within a year, at a
median resale ratio of exactly 1.000, and a 4-seed fogged run put 59% of signings as *sold on in
the window they arrived*. Players were being passed from club to club inside one window: a
buyer's new signing left a role over template, the memoryless policy listed him next round at
the same frozen ask, and the next club did the same. Every hop counted as a transfer. That
predates fog, and it inflates both the 1.88 and the 4.54 of §5.6.

`TRANSFER_MODEL.md` §5 step 4 now says a player moves at most once per window, and the clearing
loop enforces it. **Predictions for the re-run, written before it:** same-window resale is 0 by
construction; fogged volume falls to 1.9–2.6 (the old figure less its 59%, plus some
substitution) and omniscient volume to 0.5–0.9, so fog still raises volume by a factor of two
to four; the share gone within a year falls under 30% in both; the median fee rises.

**Readings with the rule in place** (24 seeds × 15 seasons, per-seed sd in brackets):

| Row | Omniscient | Fogged |
|---|---|---|
| Transfers per club per window | 2.48 (0.10) | 4.33 (0.24) |
| Fee / true value, median | 0.949 (0.000) | 0.961 (0.008) |
| Share paying above true value | 1.9% | 42.6% |
| Share gone from the buyer within a year | 70.7% (1.5) | 73.5% (1.6) |
| Resale fee / purchase fee, median | 1.07 | 1.08 |
| Sold on in the window they arrived | 0 | 0 |
| Star signings written off in two years | 0.1% | 0.7% |
| Median fee | 0.89M | 0.59M |
| Points-Gini, late | 0.160 | 0.137 |

Against the predictions:

- **Fog manufactures churn: refuted.** The share gone within a year is 71% without fog and 74%
  with it. Fog adds a mild winner's curse (two buyers in five pay over true value, by little)
  and no flops. §5.6's caveat is answered: the extra volume under fog is not extra churn.
- **Volume falls once hops are removed: refuted.** Omniscient volume *rose*, 1.90 to 2.48, and
  fogged volume barely moved, 4.54 to 4.33. The clubs that used to receive a passed-on player
  bought someone else instead. Fog still raises volume, by a factor of 1.75 rather than 2.4.
- **The share gone within a year falls under 30%: refuted, and this is the finding.** Seven
  signings in ten are sold again within a year, at a small profit, fog or no fog. The carousel
  did not stop; it slowed from one hop a round to one hop a window. No club keeps what it buys.
  That is a property of `UtilityPolicy`, not of fog and not of the clearing loop: it buys
  whatever shows a surplus over the ask and lists whoever is weakest in an over-template role,
  with no notion of having just signed someone. It is filed as `BACKLOG.md` §4.8 and not fixed
  here, because the fix is a change to what the policy wants, and that needs its own design in
  `TRANSFER_MODEL.md` §6.
- The flop rate is near zero by construction rather than by good judgement: a club can always
  sell at about what it paid, so nothing is ever written off.

---

## 6. The cost budget

**Settled (owner decision): the number of agent-managed clubs is configuration, with no
architectural cap.** The budget below is what keeps any chosen number affordable.

**Proposed: a per-season language-model budget, set in configuration, enforced by the driver.**
A 38-matchday season with *every* rival agent-managed:

| Call | Model | Frequency | Per season |
|---|---|---|---|
| Journalist: the human's match | language | per matchday | 38 |
| Journalist: league round-up | language | per matchday | 38 |
| **Language-model total** | | | **76, two per matchday** |
| Manager's intent: stance, 19 rivals | decision | pre-season, each window, on a trigger | ~76 |
| Board: stance | decision | monthly | ~10 |

The fast policies cost nothing and are not in the table. Decision-model calls are counted but
not budgeted: they are local and small, and their limit is the per-club trigger cap below, which
exists for stability rather than cost.

### 6.1 When a manager reconsiders

**Settled (owner decision): a stance is revisited when things go badly.** Outside the two fixed
points — pre-season and the opening of each window — a manager reconsiders only on one of:

| Trigger | Fires when | Read from |
|---|---|---|
| **Form** | the last five league results yield three points or fewer | `GameState.results` |
| **Standing** | the club falls a set number of places below its reputation-implied position | `league_table`, `Club.reputation` |
| **Confidence** | club sentiment (§9.3) crosses a negative threshold | recorded beats |
| **Loss** | a first-choice player is ruled out for two months or more | `Player.injured_until` |

*Confidence* is the rival-club stand-in for board unhappiness: rivals have no board agent, and
club sentiment is the same quantity the human's board reads. Thresholds live in a `TriggerKnobs`
table.

Three rules, all for stability:

- **Edge-triggered, with re-arming.** A trigger fires once when its condition *becomes* true and
  cannot fire again until the condition has cleared — the discipline `news::NewsObserver`
  already uses for its warnings. A club stuck in a slump is asked once, not weekly.
- **Capped per club per season**, default four reconsiderations.
- **Bad news only in v1.** A manager on a winning run changes nothing. Whether success should
  make a manager bolder is a real question and a second feedback loop; it waits until the first
  is shown stable.

A stance change is itself a response to results that then affects results, so it is a feedback
loop of the kind §9.5 guards. The edge-triggering and the cap are its damping, and §9.5's
streak-inflation guard is run with stance triggers live.

Rules that keep it there:

- **Skip when nothing changed.** If a seat's observation hashes equal to the one behind its last
  decision, the last decision stands and no call is made. Recorded in the trace.
- **Routine prose on the cheap model.** Model choice is per role, in
  configuration, recorded per trace.
- **A deterministic degradation order when the budget runs short:** league round-up first, then
  the human's match report. Stances do not degrade: they are not on the budget.
- **One retry at most**, and only for a parse failure (§7). Never for a validation failure: an
  illegal output is a datum, and retrying until legal would hide the rate the research wants.

**Direct mode**, in which a language model picks a club's team sheet and transfer plan itself
instead of a decision model setting a stance, remains available as configuration for a chosen few clubs. It is an
experimental arm of the ablation (§12.2), not the default, and it is where the cost goes if it
is switched on.

Defaults: all rivals stance-managed, none in direct mode, language-model budget 100.

---

## 7. Provider, fake provider, fallbacks

### 7.1 The interfaces

**Proposed:** two narrow traits, one per kind of model, with everything football-specific
outside both.

```rust
pub trait Provider {
    fn complete(&self, requests: &[Request]) -> Vec<Result<Response, ProviderError>>;
}

pub struct Request  { pub model: ModelId, pub system: String, pub prompt: String, pub max_tokens: u32 }
pub struct Response { pub text: String, pub usage: Usage }
```

It takes a batch so a stage's calls can run concurrently inside an implementation without `async`
leaking into the driver or the trait. No tool calling, no provider-specific structured-output
mode in the contract: the agent is asked for JSON and the driver parses it. That keeps every
provider, and the fake one, interchangeable — and it means parse failures are *observed*, which
is a metric (`DESIGN.md` §6), rather than prevented by a vendor feature.

```rust
pub trait Decider {
    fn ask(&self, queries: &[Query]) -> Vec<Result<Answers, ProviderError>>;
}

pub struct Query    { pub model: ModelId, pub state: String, pub questions: Vec<Question> }
pub struct Question { pub id: String, pub text: String, pub options: Vec<String> }
pub struct Answers  { pub distributions: Vec<Vec<f64>> }   // one per question, over its options
```

`Decider` is the decision-model contract: a state and typed questions in, a distribution per
question out. Its implementations are a local decision model, a language model wrapped to answer
the same questions (the ablation arm of §12.2), and the fakes of §7.2. A `Decider` reply has no
parse step and so no parse-failure rate; what it has instead is calibration (§12.2).

### 7.2 The fake provider

**Proposed: four modes, all deterministic, all offline.**

| Mode | Returns | Proves |
|---|---|---|
| `Baseline` | the built-in policy's decision, serialized as the agent would | the plumbing is an identity: a full season through the agent path equals one without it |
| `Scripted` | a canned response keyed by `(role, observation hash)` | scenario replay; the regression suite for prompts and parsers |
| `Garbage` | malformed JSON, unknown players, illegal lineups, false claims | every fallback and every gate path |
| `Recorded` | the raw output stored in an existing trace | re-scoring old runs with a new parser or scorer at zero cost |

Each mode exists for both traits: the fake `Decider` returns, respectively, all mass on
`Neutral`, scripted distributions, deliberately uniform or degenerate ones, and recorded ones.

`Baseline` is the load-bearing one. **A season driven through `fforge-agents` with the `Baseline`
provider must produce a log byte-identical to the same season with no agents** — the agent
layer's equivalent of the neutral-tactics invariant, and the first test written.

### 7.3 Fallbacks

**Settled** (`DESIGN.md` §4.4): the game is fully playable with zero LLM calls.

| Role | Fallback | Mechanism |
|---|---|---|
| Manager's intent | the stance already in force, or neutral | do not submit |
| Manager (direct mode) | the fast policy | do not submit (§3.2) |
| Journalist | `TemplateRenderer` | publish the template article and its claims |
| Board | rule-based stance | do not submit |

A fallback fires on: provider error or timeout; unparseable output after the one retry; a
`CommandError` at submission; budget exhausted. **The reason is recorded in the trace and the
game continues without surfacing it.** A fallback never produces an `Event` that says a fallback
happened — the log records decisions, and "no decision" is already representable.

---

## 8. Claims and the validation gate

**Settled** (`DESIGN.md` §7, rule 3): a claim feeds back only if it validates against the log.

### 8.1 What a claim is

A journalist's output is prose plus the facts the prose rests on:

```rust
pub struct Article {
    pub headline: String,
    pub text: String,
    pub claims: Vec<Claim>,
}

pub struct Claim {
    pub fact: Fact,
    pub sources: Vec<EventRef>,   // the events this fact was read from
}

pub enum Fact {
    MatchResult { fixture: FixtureId, home_goals: u8, away_goals: u8 },
    Scored      { fixture: FixtureId, player: PlayerId, goals: u8 },
    SentOff     { fixture: FixtureId, player: PlayerId },
    Injured     { player: PlayerId, days_out: u16 },
    Transfer    { player: PlayerId, to: ClubId, fee: Money },
    Run         { club: ClubId, matches: u8, wins: u8, draws: u8, losses: u8 },
    Standing    { club: ClubId, position: u8 },
}
```

**Proposed: a claim is typed, closed and sourced.** The journalist's observation is a list of
`NewsItem`s, each already carrying its `EventRef`s. The agent may assert only facts expressible
in `Fact`, and must cite the events it read them from. Typed IDs, never names: a hallucinated
player has no `PlayerId` to be written as.

`Fact` deliberately mirrors `NewsKind`. They are the same vocabulary seen from two sides — what
the world can report and what a writer may assert — and should be kept in step.

### 8.2 The gate

`validate(claim, state, log) -> Verdict` is a pure function in `fforge-core`:

| Verdict | Meaning | Mechanical weight |
|---|---|---|
| `Supported` | the cited events exist and establish the fact | eligible for §9 |
| `Contradicted` | the cited events exist and say otherwise | none |
| `Unsupported` | a cited event does not exist, or does not bear on the fact | none |

Every verdict is written to the trace. The share of `Supported` is the factual-grounding metric,
produced as a side effect of play (`DESIGN.md` §7's convergence).

### 8.3 The limit of the gate, stated honestly

The gate checks `claims`. It does not read `text`. A journalist can list three true claims and
then write a paragraph inventing a dressing-room row.

**Proposed: accept this, and make it harmless by construction.** Mechanical effect flows *only*
from validated claims (§9), so invented prose has exactly zero force — it is "flavor-only" in
`DESIGN.md`'s own words. Whether the prose is *faithful to its claims* is then a research-rigor
question, scored offline by a separate grounding scorer over traces (§12.1), not a safety
question the live game depends on. The game is safe without the scorer; the scorer makes the
journalist measurable.

---

## 9. Narrative feedback

### 9.1 The missing surface

`DESIGN.md` §7 speaks of morale and board confidence. Neither exists. **Proposed: add one derived
quantity, `sentiment`, and give it exactly one mechanical consumer in v1.**

### 9.2 From claim to beat

A `Supported` claim may produce a recorded event:

```rust
Event::NarrativeBeat { date: GameDate, subject: Subject, kind: BeatKind, source: EventRef }

pub enum Subject  { Club(ClubId), Player(PlayerId) }
pub enum BeatKind { Triumph, Setback, Humiliation, Breakthrough, Scandal /* closed set */ }
```

Three rules, each a structural implementation of one of §7's:

1. **The beat's kind and valence come from a fixed table keyed on the fact, never from the
   agent.** A 4–0 defeat is a `Humiliation` worth a fixed amount whoever writes it up and however
   lurid the prose. The LLM cannot choose a magnitude. *(Rule 3: claims have force only through
   the log.)*
2. **At most one beat per `(source event, subject)`, ever.** Ten articles about the same defeat
   produce one beat. Repetition cannot amplify. *(Rule 1: feedback triggers on events.)*
3. **A beat's `source` must be a simulation event, never another beat or an article.** Narrative
   cannot cite narrative. *(The loop is cut at one hop, which is what stops hallucination
   bootstrapping.)*

So what does the journalist actually control? **Selection.** A stage has more reportable facts
than an article has room for, and only claims that are *made* can become beats. Choosing to lead
with the humiliation rather than the debutant's goal is real, bounded influence over the world —
bounded by the table, by rule 2, and by the fact that the template fallback selects too (by
`salience`), so the world has narrative dynamics with zero LLM calls.

### 9.3 From beats to sentiment

```
sentiment(subject, today) = clamp( Σ over beats  valence(kind) · exp(−(today − date) / τ),  ±S_max )
```

A leaky reservoir, **derived from recorded beats and never stored** — the same shape as match
condition over recent appearances, and for the same reason. It is bounded by the clamp and
decays by construction, which is `DESIGN.md` §7's rule 2 as arithmetic rather than as discipline.
`τ` and `S_max` live in a `NarrativeKnobs` table.

**The one consumer, proposed:** player sentiment shifts the *centre* of the per-match Consistency
draw (`MATCH_MODEL.md` §17) by `narrative_scale · sentiment`, bounded to a few percent. A player
the press has built up plays slightly above himself for a while; one being pilloried, slightly
below. Reasons for choosing this channel:

- it reuses a mechanism that already exists, has its own RNG stream and its own identity setting;
- `narrative_scale: 0.0` reproduces today's engine bit for bit, so the golden baselines pin the
  identity exactly as they do for every other 2e feature;
- it is small and bounded at the point of use, on top of being bounded at the source.

Club sentiment has no match-engine consumer in v1. It feeds the board (§9.4) and the UI.

**The natural negatives `DESIGN.md` asks for** (a thrashing galvanises) belong in the valence
table: `Humiliation` can carry a short positive rebound after its negative. Their sizes are the
"narrative influence model" `DESIGN.md` §10 lists as a calibration-and-taste problem; §9.5 fixes
the requirement they are calibrated to.

### 9.4 The board

`BoardStance { objective: Objective, patience: u8 }`, set monthly, is presentation and pressure in
v1: it tells the human what is expected and how close the board is to losing faith, reading club
sentiment and the table. **Settled (owner decision): it has no power to sack the human or cut the budget in v1.**
Those are large mechanical effects that deserve their own stabilisers and their own calibration
pass, and nothing else in Phase 5 depends on them.

### 9.5 Tangible, and provably not runaway

**Settled (owner decision): narrative must have a tangible positive or negative effect without
inducing an endless feedback loop.** Both halves are made testable.

**Tangible.** At full sentiment a player's expected rating shifts by a margin a player of the
game would notice over a month — the calibration target is **about a quarter of a rating point**
at saturation, decaying with a half-life of **about three weeks**. Below that the system is
decoration; far above it, the press picks the team.

**Not runaway.** The loop is *result → beat → sentiment → performance → result*. Its gain is the
product of four bounded terms: a result produces at most one beat per subject (§9.2, rule 2); a
beat's valence is a table constant; sentiment is clamped at `S_max`; and the performance shift
is `narrative_scale · sentiment`. The requirement is that **one extra defeat raises the expected
number of further defeats attributable to narrative by well under one** — loop gain below one —
so every disturbance dies out geometrically. The clamp guarantees boundedness even if the gain
were misjudged; the gain condition guarantees there is no slow drift toward the clamp.

Three pooled guards in `fforge-harness`, comparing seasons with narrative on against the same
seeds with it off:

1. **No amplification:** the spread of final league points does not widen beyond its per-seed
   noise. Runaway shows up as the strong pulling away and the weak collapsing.
2. **No streak inflation:** the distribution of winning and losing run lengths stays in band.
   This is the direct signature of a result feeding the next result.
3. **Return to baseline:** after an injected shock — a block of beats on one club — its results
   recover to the no-narrative trajectory within a few half-lives.

Guard 3 is the one that distinguishes "bounded" from "stable", and it is the reason beats are
recorded events: a shock can be injected into a replay without touching any engine.

---

## 10. Persona and memory

### 10.1 Persona

**Proposed:** a persona is structured data with a version, held in `fforge-agents` configuration
and keyed by `StaffId`:

```rust
pub struct Persona {
    pub id: PersonaId,          // includes a version; any edit is a new version
    pub seat: SeatRole,         // Journalist | Manager | Board
    pub traits: Vec<Trait>,     // closed enum: Pragmatic, Idealist, Abrasive, YouthFirst, …
    pub philosophy: String,     // a few sentences, bounded length
}
```

Personas are **not in `World`** and not in the log. Replay does not need them, since it never
invokes anything; a counterfactual run names the persona set it used in its trace header. Keeping
them out of the domain model also honours `fforge-domain`'s freeze.

Traits are a closed enum rather than free text so that persona-consistency probes (§12.4) have
something definite to test against — and the first such probe is free: a `YouthFirst` manager who
sets `youth: Against` has contradicted his own sheet, and the trace shows it.

### 10.2 Memory

**Proposed: in v1 memory is derived from the log, not written by the agent.** A seat's memory is
a deterministic digest built at gather time: its own last `N` decisions and their outcomes, plus
the audience-scoped `NewsItem`s since. Bounded by `N`, rebuilt every time, never stored.

This is `DESIGN.md` §4.4's "bounded, summarized memory" with the summariser replaced by a pure
function, and it is a deliberate narrowing. Agent-authored memory is where generative-agent
systems drift: a slightly wrong summary becomes the premise of the next one. Deriving memory
from the log makes memory fidelity perfect by construction, so every error an agent makes is
attributable to *reasoning over true memories* — a cleaner experimental object. LLM-summarised
memory is a later, separately measurable step, and the comparison between the two is itself a
result worth having.

---

## 11. The trace

**Settled** (`DESIGN.md` §5): the trace shape is fixed and domain-independent — observation seen,
raw output, parsed decision, world delta, ground-truth info.

**Proposed record**, one per invocation, appended to a JSON-lines file beside the save:

```rust
pub struct TraceRecord {
    pub at: EventRef,                 // log position when the observation was built
    pub seat: Seat,                   // role + club
    pub persona: PersonaId,
    pub prompt_version: String,
    pub model: ModelId,
    pub observation: Observation,
    pub observation_hash: u64,
    pub raw: Option<String>,          // None when no call was made (cache reuse, budget)
    pub parsed: Option<Decision>,     // trace-side wrapper over the existing values of §3.1
    pub outcome: Outcome,             // Submitted | Reused | Fallback(reason)
    pub verdicts: Vec<Verdict>,       // journalist only
    pub events: Vec<EventRef>,        // what the submission appended: the world delta
    pub info: Info,                   // ground truth withheld from the agent
    pub usage: Usage,
}
```

- **The trace is never read by the core and losing it loses no game.** A save without its trace
  file replays perfectly. A trace without its save is still scorable for grounding.
- **`info` is where fog is undone**: for a transfer decision it carries true PA and true `value()`
  for every candidate the agent was shown an estimate of. Decision quality is scored against it.
- **Scenario replay** is a `TraceRecord`'s `observation` fed to a different persona, prompt or
  model with the `Scripted` or a live provider. No world needs to be reconstructed.

---

## 12. Evaluation

`DESIGN.md` §5's discipline first: *what decision will this metric change?* Each axis below names
its decision. Agent evaluation runs live in `fforge-harness`.

### 12.1 Factual grounding

The `Supported` share from the gate, plus an offline prose-versus-claims scorer (§8.3).
**Decision it changes:** ship or hold a journalist prompt; cheap model or capable one for prose.

### 12.2 Decision quality — the ablation

Hold the world and seed fixed; put policy `A` and policy `B` in the same seat; compare. The
ladder of comparators, all under the same fog:

| Policy | Role |
|---|---|
| random-legal decisions | the floor |
| fast policy, neutral stance | the baseline everything must justify itself against |
| fast policy, random stance | separates "a stance helps" from "any change helps" |
| fast policy, best fixed stance for that squad | what a perfect stance-setter would achieve |
| fast policy, stance set by a decision model | **the default subject** |
| fast policy, stance set by a language model | is a generative model better at intent, and at what cost? |
| language model in direct mode | the expensive arm: is per-decision deliberation worth it? |
| learned fast policy (§2.4, later) | a trained policy on the same decisions |
| fast policy with fog removed | the ceiling: what perfect information is worth |

The stance model makes the first question sharper than "is the LLM better than the baseline".
It becomes: **does a model choose a stance that suits its squad** — close to the best fixed stance
and above a random one — and does that survive a change of squad? The direct-mode arm then
prices the alternative.

**A precondition, inherited from tactics:** the stance axes must be calibrated to non-dominance
first. If one stance is simply best, "the model chose well" and "the model chose the dominant
setting" are indistinguishable, and a random-stance arm that scores as well as the model arm
would be the tell.

**Calibration of confidence**, a reading decision models make available for free: bucket every
stance answer by the probability the model gave it, and compare against how often that answer
was in fact the best one for the squad. On the `risk` axis in the 16–18 band, where §5.4 shows
no one can know, any confidence far from uniform is overconfidence by construction.

**Tactics axis — settled by `TACTICS_MODEL.md` §9:** non-dominance is squad-conditional, so the
measure is *does the agent match tactics to its squad*, scored as expected-points share against
the best fixed tactic for that squad, pooled across worlds. Not counter-picking the opponent.

**Transfer axis — proposed, in light of §5.4:** value captured per unit spent, measured against
ground truth at a fixed horizon, reported in two bands that must never be pooled:

- **ages 19 and over**, where visible data carries signal about PA and estimation skill can
  exist. This is the headline.
- **ages 16–18**, where it cannot. Here the reading is *calibration*: does the agent's spend track
  the uncertainty it was told about? Overpaying confidently in this band is the failure to count.

**Decision it changes:** which kind of model sets a manager's intent, and whether any of them
beats leaving every manager neutral — that is, whether Phase 5's
value is the narrative layer over the utility baseline.

**In-match plans** cannot be scored on `bin/calibrate`, which has no `GameState` and runs at the
condition identity (`BACKLOG.md` §4.3). **A real-season match harness is a prerequisite of this
axis and should be built with the deterministic half**, not after it.

### 12.3 Illegal-output rate

Parse failures and `CommandError`s per invocation, by role and model.
**Decision it changes:** prompt and schema design; which model a role can run on.

### 12.4 Persona consistency and believability

Periodic probes against the trait list; an LLM-judge rubric for believability, spot-checked by
hand. `DESIGN.md` §5 ranks these weakest and this note keeps them last. **Deferred** until real
traces exist to look at; designing rubrics before seeing output is designing ahead of evidence.

---

## 13. Build sequence

The deterministic half — `BACKLOG.md` §3.2. No LLM, no network, everything offline:

1. The control seam (§3.2). *Identity test: the whole existing suite, untouched.* **Done.**
2. Fog-of-war (§5) for every decider and in the UI, then re-bank `bin/market` and `bin/calibrate`
   against §5.3's predictions. **Done** — §5.5, §5.6.
2b. `ManagerStance` (§2.3) at neutral, then calibrate `StanceKnobs` to non-dominance.
   *Identity test: golden baselines and all three harnesses, untouched at neutral.*
3. Observation builders and the `info` payload.
4. `fforge-agents`: the driver, the `Provider` trait, the fake provider's four modes.
   *Identity test: a `Baseline` season is byte-identical to a no-agent season.*
5. Claims, the gate, `NarrativeBeat`, sentiment at `narrative_scale: 0.0`, and §9.5's three
   guards. *Identity test: golden baselines.*
6. The trace store and scenario replay.
7. The real-season match harness (§12.2).

At the end of step 7 the whole architecture exists, every test runs offline, and the game is
unchanged from the player's seat except that the transfer market is fogged. That is the point at
which to inspect the seams before spending a token.

The agent half — `BACKLOG.md` §3.3: a real provider; the journalist; switch on `narrative_scale`
and calibrate it to §9.5; a real decision model, model-set stances and the ablation; the board. Narrative can be switched
on with templates alone, before any language model exists, since templates make claims too.

---

## 14. Open questions

**Answered by the owner and now settled in the text:**

- the number of agent-managed clubs is configurable with no cap (§2.2, §6);
- everyone is fogged, the human included; fog is shown as ranges, a range holds the truth about
  nine times in ten, and scouting upgrades narrow it later (§5);
- the board does not act in v1 (§9.4);
- narrative must be tangible without running away (§9.5);
- execution goes to fast policies, intent to decision models, text to language models (§2.1);
- the stance has eight axes of five steps (§2.3);
- a manager reconsiders on bad news — form, standing, confidence, a serious loss (§6.1).

**Still open.** Each has a proposed default, so no work is blocked.

1. **The trigger thresholds** (§6.1): how bad is bad. Defaults are in the table; they are knobs
   and will be read on the harness like any others.
2. **Should good news also move a stance?** Default no in v1 (§6.1).
3. **Argmax or sampling for a stance** (§2.3). Default argmax.
4. **Is one sentiment consumer enough?** Default yes for v1. The obvious second is club sentiment
   nudging `Club.reputation`, which touches revenue and transfer consent and is a much larger
   loop to stabilise.
5. **When to build a trained fast policy** (§2.4). Default: after Phase 5, in the harness first.
6. **Which models.** `d1` for intent is the working assumption, to be confirmed by measuring it
   against `d1-omni` and a language model on §12.2's ladder rather than by reputation.
7. **Does a journalist have a house style per outlet?** Personas support it; whether several
   outlets are worth the budget is a game-feel question.
