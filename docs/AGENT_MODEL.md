# Agent Model — Phase 5 Design Note

The design record for the agent layer (`DESIGN.md` §4.4–§7, Phase 5): where agents are invoked
given a pure core, what they see and may decide, how their output is validated and recorded, how
narrative feeds back into the world, and how all of it is measured. Same spirit as the other
notes: a living artifact that pins decisions and the reasoning behind them.

`DESIGN.md` already settles the *principles* — propose/dispose, the Gym-shaped boundary, the
Event/Trace split, the three feedback rules. This note turns them into shapes concrete enough to
implement, and resolves the seven items `BACKLOG.md` §3.1 lists.

**Draft status.** Every decision below carries one of three marks:

- **Settled** — follows from an existing note or from code already merged; restated, not reopened.
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
  long-term memory (§10.2); agents for every club (§6 budgets a few); a PettingZoo-conformant
  wrapper (§3.4); player agents.

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

**Proposed.** Three agent roles in v1, in build order:

| Role | Decides | Decision value | Baseline it is swapped against |
|---|---|---|---|
| **Journalist** | which facts to report, and the prose | `Article { text, claims }` (§8) | `news::TemplateRenderer` |
| **Manager** (rival clubs) | team sheet, tactics, substitution plan; the window's transfer plan | `Lineup`; `Vec<TransferDecision>` | `ai_pick_lineup_vs`; `UtilityPolicy` |
| **Board** (the human's own club) | a season objective and a reaction to results | `BoardStance` (§9.4) | a rule-based stance |

The journalist is first because it is read-only on the decision side: it exercises the provider,
the trace and the gate with no effect on the simulation until §9's feedback is switched on. The
manager is second and is the ablation's subject. The board is last and smallest.

**The human is the manager of their own club and is never replaced by an agent.** Agents play the
people around them: rival managers, their own board, the press.

**Scouts are not agents in v1.** Scouting is the fog-of-war function of §5 — a deterministic
degradation of what an observer sees — not a persona that reasons. A scout *persona* that writes
reports from the fogged view is a natural later addition and needs nothing new from the core.

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

Today the log knows one controlled club: `GameStarted.player_club`, `GameState.pending_lineup`
(a single `Option`), and `LineupSubmitted` with no club field. Everything else is the built-in
policy. Agents need the fold to hold a pending decision for *several* clubs.

**Proposed:**

- `LineupSubmitted` gains `club: Option<ClubId>` with `#[serde(default)]`; `None` means the
  log's `player_club`, so every existing save loads unchanged (hard constraint 5).
- `GameState.pending_lineup`/`last_lineup` become `BTreeMap<ClubId, Lineup>`;
  `pending_transfer_decisions` becomes `BTreeMap<ClubId, Vec<TransferDecision>>`.
- `Command::SubmitLineup` and `SubmitTransferDecision` gain a `club`. The existing rule
  generalises without change: **a club with a pending decision plays it; a club without one
  plays the built-in policy.** `effective_player_lineup` becomes `effective_lineup(state, club)`.
- `market::resolve_window`'s `human_club: Option<ClubId>` becomes the set of clubs with a
  recorded plan. `RecordedPolicy` is already the right mechanism.

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

**Proposed:** an observer sees a player's 25 performance attributes as they are, and never sees
the fields `ATTRIBUTE_SCHEMA.md` already marks hidden: `Character.potential` (PA), the rest of
`Character`, and `DevProfile` (`E`, `φ`). In their place the observation carries a **scouted
estimate** of PA and nothing else.

Visible attributes are left exact on purpose. Fogging them too would be more realistic but would
inject noise into every lineup decision and every CA reading, and the question Phase 5 exists to
ask — can an agent act well under uncertainty about a player's *future* — is fully posed by PA
alone. One hidden quantity, one knob set, one thing to calibrate.

### 5.2 The estimate

```
pa_hat(observer, player, season) = PA + σ(observer, player) · z
z = standard normal from derive_stream(seed, SCOUT_NS | observer | player | season)
```

- **Deterministic and derived, never stored.** The estimate is a pure function of the seed and
  of facts already in the world, exactly as condition is (`MATCH_MODEL.md` §13). Nothing is added
  to `Player` or `Club`. Hard constraint 1 holds by construction.
- **Stable within a season.** The draw is keyed by season, not by date. An observer who looks
  twice sees the same number, so repeated looking cannot average the noise away.
- **Independent across observers.** Each club draws its own `z`. Twenty clubs now hold twenty
  different opinions of the same teenager — the divergence the transfer market has lacked.
- **`σ` shrinks with what is genuinely knowable:** with the player's age (a 27-year-old's ceiling
  is nearly his present), for the observer's own players (daily contact), and with the
  observing club's `reputation` as a stand-in for scouting reach. All three are knobs in a new
  `ScoutKnobs` table, plausibility-picked and then read on the harness like every other table.
- The estimate is clamped to `[best_role_ca, 100]`: nobody scouts a ceiling below the present.

Perceived value is then `value()` evaluated with `pa_hat` in place of PA and a *neutral*
development profile in place of the hidden one. Same function, degraded inputs.

### 5.3 Who is fogged

**Proposed: everyone who decides** — LLM managers, the built-in `UtilityPolicy` clubs, and the
human. Three reasons:

1. The ablation compares an agent against the utility baseline. If only the agent is fogged the
   comparison measures the handicap, not the agent.
2. Uniform omniscient valuation is the leading explanation for transfer volume sitting below
   target through three re-banks (`BACKLOG.md` §4.1): with every club pricing off the same
   `value()`, clubs converge on the same targets. Divergent estimates are the proposed cure.
3. A human who sees true PA while every rival guesses is playing a different, easier game.

**This re-rolls the market.** Fogging the built-in policy changes every AI club's decisions, so
`bin/market` must be re-banked and its guard re-read. Predictions, written down first as the
calibration discipline requires:

- transfers per club per window **rises** from 1.88, toward the 2–5 band;
- talent concentration (`top3_share_of_top20`) **falls** from its omniscient upper bound;
- the wonderkid flop rate as *experienced by buyers* (paid like a star, peaked as a squad
  player) becomes non-zero and is a new reported row;
- development and match readings do **not** move — fog touches no engine.

If volume does not rise, the §4.1 hypothesis is refuted and the cap is elsewhere in the clearing
loop. That would be a finding to record, not a reason to turn the noise up.

How the human's fog is *presented* (a range, a star band, a scout's sentence) is a layer-5
question and is **open** (§14).

### 5.4 The finding this must reckon with

`career_arc` measures how well PA can be recovered from what an observer can see. Two attacks:
the **naive** fit on `(CA, age)` and the **competent** fit on the per-category composites plus
age. After the seeding fix the naive residual is 5.57 points — so there *is* something to hide.
But the two attacks are near-identical at ages 16–18 and only separate at 19–21.

Read plainly: **at the ages wonderkid scouting is about, visible data contains almost no
exploitable signal about PA beyond the obvious.** No agent, however good, can see through that
fog by reasoning harder over attributes. Two consequences:

- **The skill available at 16–18 is not estimation, it is decision-making under a known-noisy
  estimate**: how much to pay given `pa_hat` and its uncertainty, how many lottery tickets to
  hold, when to sell one. The transfer axis of the ablation must score *that* (§12.2), not
  estimation accuracy.
- **The 16–18 band is a ready-made overconfidence probe.** An agent that claims to have
  identified a wonderkid from his attributes at 17 is asserting skill that the data provably does
  not support. That is measurable, and it is a more interesting research reading than a win rate.

---

## 6. The cost budget

**Proposed: a hard per-season call budget, set in configuration, enforced by the driver.** A
38-matchday season with `K` agent-managed rival clubs:

| Call | Frequency | Per season, `K = 3` |
|---|---|---|
| Manager: team sheet + tactics + plan (one call) | per rival per matchday | 114 |
| Manager: transfer plan | per rival per window | 6 |
| Journalist: the human's match | per matchday | 38 |
| Journalist: league round-up | per matchday | 38 |
| Board: stance | monthly | ~10 |
| **Total** | | **~206, about 5.4 per matchday** |

Rules that keep it there:

- **One call per decision, not per field.** A manager returns the whole `Lineup` at once.
- **Skip when nothing changed.** If a seat's observation hashes equal to the one behind its last
  decision, the last decision is reused and no call is made. A settled squad between injuries
  costs nothing. The hash and the reuse are recorded in the trace.
- **Routine prose on the cheap model**; the manager on the capable one. Model choice is per role,
  in configuration, and recorded per trace.
- **A deterministic degradation order when the budget runs short:** league round-up first, then
  the board, then the human's match report, and managers last — narrative degrades to templates
  before decisions degrade to the baseline.
- **One retry at most**, and only for a parse failure (§7). Never for a validation failure: an
  illegal decision is a datum, and retrying until legal would hide the rate the research wants.

`K` and the budget are **open** (§14); 3 and 250 are the proposed defaults.

---

## 7. Provider, fake provider, fallbacks

### 7.1 The interface

**Proposed:** one narrow trait, text in and text out, with everything football-specific outside
it.

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

### 7.2 The fake provider

**Proposed: four modes, all deterministic, all offline.**

| Mode | Returns | Proves |
|---|---|---|
| `Baseline` | the built-in policy's decision, serialized as the agent would | the plumbing is an identity: a full season through the agent path equals one without it |
| `Scripted` | a canned response keyed by `(role, observation hash)` | scenario replay; the regression suite for prompts and parsers |
| `Garbage` | malformed JSON, unknown players, illegal lineups, false claims | every fallback and every gate path |
| `Recorded` | the raw output stored in an existing trace | re-scoring old runs with a new parser or scorer at zero cost |

`Baseline` is the load-bearing one. **A season driven through `fforge-agents` with the `Baseline`
provider must produce a log byte-identical to the same season with no agents** — the agent
layer's equivalent of the neutral-tactics invariant, and the first test written.

### 7.3 Fallbacks

**Settled** (`DESIGN.md` §4.4): the game is fully playable with zero LLM calls.

| Role | Fallback | Mechanism |
|---|---|---|
| Manager | built-in policy | do not submit (§3.2) |
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
"narrative influence model" `DESIGN.md` §10 lists as a calibration-and-taste problem, and they
stay **open** (§14) — but the *shape* that makes any choice safe is now fixed.

### 9.4 The board

`BoardStance { objective: Objective, patience: u8 }`, set monthly, is presentation and pressure in
v1: it tells the human what is expected and how close the board is to losing faith, reading club
sentiment and the table. **Proposed: it has no power to sack the human or cut the budget in v1.**
Those are large mechanical effects that deserve their own stabilisers and their own calibration
pass, and nothing else in Phase 5 depends on them.

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
something definite to test against.

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
| random-legal | the floor |
| built-in utility | the baseline the agent must justify itself against |
| LLM agent | the subject |
| utility with fog removed | the ceiling: what perfect information is worth |

**Tactics axis — settled by `TACTICS_MODEL.md` §9:** non-dominance is squad-conditional, so the
measure is *does the agent match tactics to its squad*, scored as expected-points share against
the best fixed tactic for that squad, pooled across worlds. Not counter-picking the opponent.

**Transfer axis — proposed, in light of §5.4:** value captured per unit spent, measured against
ground truth at a fixed horizon, reported in two bands that must never be pooled:

- **ages 19 and over**, where visible data carries signal about PA and estimation skill can
  exist. This is the headline.
- **ages 16–18**, where it cannot. Here the reading is *calibration*: does the agent's spend track
  the uncertainty it was told about? Overpaying confidently in this band is the failure to count.

**Decision it changes:** whether LLM managers are worth their budget at all, or whether Phase 5's
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

1. The control seam (§3.2). *Identity test: the whole existing suite, untouched.*
2. Fog-of-war (§5) for every decider, then re-bank `bin/market` against §5.3's predictions.
3. Observation builders and the `info` payload.
4. `fforge-agents`: the driver, the `Provider` trait, the fake provider's four modes.
   *Identity test: a `Baseline` season is byte-identical to a no-agent season.*
5. Claims, the gate, `NarrativeBeat`, sentiment at `narrative_scale: 0.0`.
   *Identity test: golden baselines.*
6. The trace store and scenario replay.
7. The real-season match harness (§12.2).

At the end of step 7 the whole architecture exists, every test runs offline, and the game is
unchanged from the player's seat except that the transfer market is fogged. That is the point at
which to inspect the seams before spending a token.

The agent half — `BACKLOG.md` §3.3: a real provider; the journalist; switch on `narrative_scale`
and calibrate it; the manager and the ablation; the board.

---

## 14. Open questions

Calls this note's reasoning does not make. Each has a proposed default so that work is never
blocked on it.

1. **How many rival managers are agents (`K`), and which?** Default 3: the clubs nearest the
   human in the table, re-chosen each season. Alternatives: fixed rivals for continuity of
   character; the top of the table for visibility.
2. **Is the human fogged?** Default yes (§5.3). The alternative — the human sees truth, as a
   difficulty setting — is cheap to offer later because fog is a wrapper.
3. **How is the human's fog shown?** A numeric range, a star band, or a scout's sentence. A
   presentation question with real effect on feel; `UI_TOOLKIT_EVIDENCE.md` should inform it.
4. **The valence table and its rebounds** (§9.3) — `DESIGN.md` §10's narrative influence model.
   Taste, then calibration. Nothing structural waits on it.
5. **Is one sentiment consumer enough?** Default yes for v1. The obvious second is club sentiment
   nudging `Club.reputation`, which touches revenue and transfer consent and is therefore a much
   larger loop to stabilise.
6. **Should the board ever act** (budget, dismissal)? Default no in v1 (§9.4).
7. **Which provider and models.** Deliberately not decided here; the interface is the decision.
8. **Does a journalist have a house style per outlet?** Personas support it; whether several
   outlets are worth the budget is a game-feel question.
