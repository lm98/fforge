# Football Manager Sim — Design Document

This is the design record for the project: the decisions reached during planning and
the reasoning behind them. It is meant to be referenced and extended as the project
evolves. Decisions marked **open** in §10 are deliberately unresolved.

---

## 1. Vision & Goals

- **Primary goal:** a single-player football management sim — team management, training,
  transfers, match simulation — built to personal taste.
- **Secondary goal (deliberate):** extract a reusable *sim–agent–evaluation platform* from
  this project, usable for similar future projects. The football game is always the primary
  deliverable; the platform is what falls out of building it with clean seams.
- **Distinctive bet:** LLM agents as *in-the-loop* football actors (managers, presidents,
  directors, journalists) whose decisions and narrative genuinely shape the simulated world —
  not a read-only overlay.

## 2. Guiding Principles

- **Deterministic sim first.** The LLM agents are an enhancement layered on top, never the
  foundation. An agent is only as good as the structured data it reasons over, and that data
  is the *output* of the sim.
- **Reusability is an extraction, not a prediction.** Build football concretely; place clean,
  narrow seams where a future platform would cut. Do **not** design general abstractions ahead
  of the evidence — you cannot generalize well from zero concrete instances (WET before DRY /
  rule of three).
- **Single-player, single-process.** From distributed systems, import only *clean boundaries,
  reproducibility, and event streams*. No services, queues, or actual distribution.
- **Thin vertical slice first, then deepen.** Get the whole loop running end-to-end before
  perfecting any one subsystem.
- **The calibration/evaluation harness is a first-class citizen**, not an afterthought.

## 3. Architecture

### Layered structure

Five layers, with a hard rule: **lower layers know nothing about higher ones.**

1. **Domain model** — entities and the attribute schema. The most important design artifact;
   everything downstream depends on it.
2. **Simulation core** — deterministic, seedable, UI- and LLM-agnostic. Contains the match,
   development, market, and calendar/time engines. A near-pure function of `(state, seed)`.
3. **Decision layer** — club AI (utility/rule-based): transfers, tactics, contract offers.
4. **Narrative / LLM layer** — pluggable and *optional*.
5. **Presentation** — CLI first; the egui management UI and an optional Bevy match viewer come
   later (§8).

### Cross-cutting invariants

- **Determinism & seeding from day one.** Every stochastic engine takes an explicit RNG seed.
  Reproducible bugs, replayable matches, trustworthy calibration.
- **Event sourcing.** Game state is an append-only event log. Save/load, replay, and debugging
  follow almost for free. A management game *is* a stream of events mutating a world.
- **The deterministic core is a pure fold over the event log.** All sources of impurity are
  pushed to the edge and captured (see §6).

## 4. Simulation Subsystems

### 4.1 Match engine

> **Phase 2a implementation detail lives in `MATCH_MODEL.md`** — the pinned design record for the
> event-based possession engine (five-zone state space, actor-centric resolution, the wide route,
> the role→zone presence table, the Rust `play_match` seam, and the calibration knobs/targets). This
> section is the high-level commitment; that note is the settled shape.

- **Committed to the middle tier:** event-based possession simulation — not pure outcome models
  (Elo/Poisson), not full spatial/agent-based physics.
- **Model:** a possession-based, Markov-ish process. State = `(team in possession, pitch zone,
  time)`. At each step, transition probabilities (pass, tackle, interception, shot, foul, card,
  injury) are modulated by attribute matchups, tactics, morale, and fatigue. **Events emerge as
  a narrative** — which is most of what a management game's match experience actually is.
- **Team quality** = role-weighted contribution *with interaction effects* (a world-class
  striker starved of service underperforms), not flat averages.
- **Tactics** = modifiers to the transition matrix; aim for soft rock-paper-scissors matchups.
- **Morale** = a mean-shift *and* a variance-reducer (form makes teams consistent).
- **Unexpected events** = low-probability, high-impact perturbations, weighted toward the
  players whose profiles make them likely.
- **Calibration harness (build early):** run thousands of simulated seasons; check emergent
  statistics against reality — goals/game (~2.6), home advantage, favorite win rates vs
  bookmaker-implied probabilities, scoreline distributions. This is an eval pipeline. *(The Phase-2a
  harness covers all of these, including the bookmaker-implied comparison: since there are no real
  odds in a synthetic world, it scores the empirical expected-points-vs-strength-gap curve against
  a documented Elo-expected-score reference curve — a favourite-discrimination check, not a fit
  target; see `MATCH_MODEL.md` §10.)*
- **The match event stream is a shared, design-once artifact** (like the valuation function):
  four consumers will depend on it — text commentary, stats aggregation, the journalist agent, and
  the eventual graphical viewer. Design its schema for *narratability* — the minute-by-minute beats
  (this pass, that tackle, the save at 73'), not merely final outcomes. Every viewer is a swappable
  *pure consumer* of this stream, structurally identical to how the eval spine consumes the game
  event stream. (Schema richness is a Phase 2 deliverable — see §9.)

### 4.2 Player development

> **Phase 3 implementation detail lives in `DEVELOPMENT_MODEL.md`** — the pinned design record for the
> monthly development engine (the PA-scaled age-envelope the attributes track, per-`DevCategory` curve
> parameters, PA-gating on best-role peak CA, the once-resolved per-player noise for flops/late
> bloomers, the append-only `DevelopmentTick` seam, and the career-arc calibration harness with its
> re-fit knob table). This section is the high-level commitment; that note is the settled shape.

- **CA/PA model:** current ability split across attributes + a hidden potential ceiling.
- Development is a trajectory from CA toward PA, modulated by an age curve, training focus,
  playing time, coaching quality, and noise, with **diminishing returns near PA**.
- **Age curves are position- and attribute-dependent:** physicals peak ~24–27 and decline;
  technical/mental attributes grow into the 30s. Keep "invest in youth vs buy ready-made" a real
  decision; keep enough noise for wonderkids who flop and late bloomers.
- **Slow loop** (weekly/monthly). Validate by simulating a decade and checking career arcs.

### 4.3 Transfer market

> **Phase 4 implementation detail lives in `TRANSFER_MODEL.md`** — the pinned design record for the
> centralized valuation function, club needs assessment and utility-based buy/sell policy, the
> simultaneous deferred-acceptance market clearing loop, club finances and contracts, youth intake
> and retirement, and the market pathology harness. This section is the high-level commitment; that
> note is the settled shape.

- A **multi-agent resource-allocation problem.** Utility-based club agents: needs (positional
  gaps, age profile, budget), a valuation function, a decision policy, inside time-gated windows.
- **Centralize the valuation function** (attributes, age, potential, contract length, form,
  positional scarcity). It is reused by match-engine role-weighting, the transfer AI, *and* the
  LLM agents. **Design it once.**
- **Test for market pathologies over long runs:** rich-get-richer runaway, talent monopolization,
  wage/fee inflation. Add stabilizers (squad-size limits, financial constraints, players wanting
  minutes) and pathology checks to the harness.

### 4.4 LLM agents — overview

(Interface and reproducibility detail in §5–7.)

- **Principle: LLM proposes, deterministic system disposes.** The numerical sim is the source of
  truth. An LLM **never** computes a transfer fee or a match result.
- Agents read a compact structured state summary and emit either natural language (reports, press
  lines, board reactions, negotiation dialogue) or *constrained* structured outputs that bias the
  deterministic layer.
- Invoked only for the player's club + a few rivals, at low frequency; with caching and cheaper
  models for routine flavor. A full season is too many events to LLM-simulate every club.
- Each persona has a **stable character sheet** (personality, philosophy, biases, relationships)
  as structured data, plus a **bounded, summarized memory store**. Reference: Park et al.,
  generative agents ("Smallville", 2023) — but start far more minimal.
- **The entire narrative layer is optional with templated fallbacks.** The game is fully playable
  with zero LLM calls. This single decision de-risks development enormously.

## 5. Agent Interface & Evaluation Platform

- **The load-bearing design is the world↔agent boundary, shaped as the RL agent–environment
  interface.** The sim *is* an environment: state, a scoped **observation** per agent, an
  **action**, a **step**. Each agent is a policy `π(action | observation)`. Evaluating an agent
  is policy evaluation.
- **Conform to the Gym/Gymnasium contract** (`reset()`, `step(action) -> (observation, info)`);
  **PettingZoo** for the multi-agent case. These APIs are proven reusable across thousands of
  unrelated environments — borrowed prior art, not speculative abstraction.
- **Agent-side contract:** receives a serializable `Observation`, returns a constrained
  serializable `Decision`. No agent reads world internals or mutates state directly.
- **The `info` channel** is where evaluation ground truth lives: `step()` returns what the agent
  perceives (`Observation`) *and* an `info` payload it does not. Observation = what the policy
  sees; info = what the *evaluator* sees. Scoped agent knowledge and privileged evaluator
  knowledge fall out of the same existing pattern.

### The reusable kernel

- The genuinely reusable platform is **not** the game or the agent framework — it is the
  **trace + replay + scoring spine**, which is domain-independent. The trace shape is fixed:
  `(observation seen, raw output, parsed decision, world delta, ground-truth info)`. Football
  lives in the *contents* of those fields, never in their structure. This is the one place where
  designing generically upfront is correct, because the shape is fixed by the interface, not
  predicted.
- Because the game is event-sourced, **agent interactions are just another event category**, and
  the evaluation layer is a **passive observer subscribing to the event stream — a pure consumer
  that never writes to the world.**
- Build generically only these three:
  1. **Trace capture** — every agent invocation logged in full and replayable.
  2. **Scenario replay** — freeze a world state (a canned Observation), run any agent version
     against it. The regression suite for agents.
  3. **Pluggable scoring** — scorers that consume traces and emit metrics.

### Evaluation discipline

- **Before measuring anything: what decision will this metric change?** If a number can't move a
  choice (keep/kill a prompt, ship/hold a persona, prefer model A over B), don't measure it yet.
- **Two evaluations, different rigor:**
  - *Game evaluation* (fun / immersive / coherent) — scrappy, qualitative, rubric-plus-vibes.
    Serves the primary goal.
  - *Research evaluation* (how LLM actors behave) — controls, baselines, statistical care.
    Serves the side interest.
- **Two superpowers the deterministic sim hands you:**
  - **A ground-truth oracle.** "Correct" is defined for every quantitative claim, so factual
    grounding is cheaply checkable: did the journalist report the real score? Does the manager's
    read match the player's hidden CA? Does the director "remember" a rivalry the log denies?
    Memory fidelity and hallucination become a diff against the log.
  - **Perfect experimental control via seeding.** Hold the entire world fixed, vary only the
    agent (same fixture/squad/board; swap model, persona, or prompt). Flawless A/B controls, and
    the enabler of the LLM-vs-utility-baseline ablation — a config change, not a rewrite.
- **Scoring axes, by how cheaply the sim makes them measurable:** factual grounding (diff vs log
  — nearly free) → decision quality (run choices forward vs baselines — the ablation) → persona
  consistency/drift (periodic probes vs character sheet + history) → believability (LLM-as-judge
  with a rubric, validated against your own spot-checks — weakest rigor, treat with suspicion).

## 6. In-the-Loop Agents & Reproducibility

- **Decision: agents are in-the-loop.** Their outputs are recorded as **first-class events**;
  reproducibility comes from **replaying events**, never from re-invoking the LLM.
- **Principle: an LLM is an external nondeterministic input, in the same class as the RNG and the
  wall clock** — record what it produced and feed it back. This is the deterministic-lockstep
  trick from RTS netcode: record *inputs*, not *state*; the sim is bit-deterministic given
  inputs. The agent-decision event is the lockstep "input."
- **Temp-0 is not reproducible** (batching, hardware). Recording the output is therefore the
  *only* robust option, not merely the convenient one.

### Event vs trace split

- **Event** (the authoritative replay input) = the **resolved, validated `Decision`** — a clean
  typed value (e.g. `SignPlayer{player_id, fee, wage, years}`), already rule-checked. This is
  what the core folds over.
- **Trace** (rides alongside, never fed to the core) = raw LLM text, the `Observation` seen, the
  validation outcome, and full model/prompt/persona versioning.
- **Why the split:**
  - *Replay robustness:* recording the resolved Decision keeps the parser out of the
    deterministic path. If you recorded raw text and re-parsed on load, improving the parser
    later would silently change every old save.
  - *Evaluation:* an in-the-loop LLM will regularly emit illegal or unparseable output (a bid for
    a departed player, malformed JSON, a hallucinated name). *How often* is a clean quality
    metric — which only survives if raw output and the validation verdict are kept.

### Two replay operations

- **Faithful replay** — feed recorded decisions back, never call the LLM (save/load, bug repro,
  regression fixtures). The event log doubles as an **LLM-output cache**: re-running a recorded
  season costs zero tokens, so the deterministic engines can be calibrated against reference
  seasons without touching the agents.
- **Counterfactual re-simulation** — replay to a point, then switch to live invocation and let
  history fork ("git for simulations"; any recorded state is a valid branch point). This, plus
  the seeding-control property, is the research substrate: fork a real trajectory at an arbitrary
  point and swap one agent.

### Deterministic ordering

- When several agents act in one window, the **order their decisions resolve is part of world
  state** (two clubs chasing one player). That ordering must itself be deterministic and
  captured — no nondeterministic scheduler.
- **Sequential vs simultaneous** — settled in favour of *simultaneous with explicit conflict
  resolution* (§10; `TRANSFER_MODEL.md` §5's deferred-acceptance clearing loop for transfer
  windows), which kills the "processed-first-always-wins" artifact and is the same mechanism
  Phase 5's LLM agents substitute into at the same seam, so their measured decision quality never
  depends on queue position.

## 7. Narrative Feedback & Safety

- **Decision: narrative output influences the world** (press / punditry / sentiment can move
  morale, board confidence, etc.).
- This **closes a loop**: the `world → agents → world` pipeline becomes a cycle. Feedback changes
  the system's *dynamics* — it is the source of the desired emergent storylines (sack race,
  crisis club, wonderkid buckling under hype) *and* of every failure below, because they are the
  same mechanism.

### Three failure modes and their defenses

1. **Runaway** (positive feedback, no damping): bad result → bad press → lower morale → worse
   result, compounding to absurdity.
   → **Narrative effects must be bounded and time-decaying, never unbounded accumulators.**
   Sentiment is a leaky reservoir trending back to baseline. Encode natural negatives (a
   thrashing motivates, pressure galvanizes).
2. **Oscillation / amplification** ("microphone next to speaker"): if narrative reflects
   continuous state and feeds back proportionally, it screams.
   → **Feedback triggers on events** (discrete, bounded, self-limiting), **never on continuous
   state levels.** The single most important structural rule.
3. **Hallucination-feedback** (unique to LLM writers): an invented claim gains mechanical force,
   manufactures the facts to make itself true, and bootstraps fiction into ground truth.
   → **A narrative event feeds back only if its factual claims validate against the event log.**
   The report is a *proposal* through the same propose-then-validate gate as any agent decision.
   Validated claims land; invented claims carry **zero** mechanical weight (still printable as
   flavor). Ground truth is the circuit breaker.

### The convergence

The guardrail the game needs and the metric the research wants are **the same object**: the
validation gate is a free, live **hallucination detector**, emitting a factual-grounding
datapoint per invocation as a side effect of play. The architecture that keeps the game stable is
the architecture that makes the agents measurable.

### Three rules to bank (cheap as decisions, expensive as retrofits)

1. Feedback triggers on **events, never on continuous state levels.**
2. Narrative effects are **bounded and time-decaying, never unbounded accumulators.**
3. A claim feeds back **only if it validates against the log**; unvalidated claims are
   flavor-only.

## 8. Technology

**Settled.** Guiding observation: the three heavy workloads (simulation, calibration, UI) have
different technology pressures, and the layering lets each go where it is strongest rather than
forcing one stack on all of them.

- **Simulation core → Rust, built once.** Runtime speed is *not* the driver — a single-player sim
  advances one fixture-set at a time, which any language handles. The real drivers are
  (a) **determinism you can trust**: explicit RNG, no GIL/threading surprises, no accidental
  hashmap-iteration-order nondeterminism — load-bearing because bit-reproducibility (*same-build*,
  not the harsh cross-platform guarantee lockstep multiplayer needs) underpins the whole
  architecture; and (b) it is the better long-term home given Rust fluency.
  **Prototype-in-Python-then-port is rejected:** it builds the deterministic core *twice*, and a
  port is exactly where determinism bugs hide silently. Instead build the core once in Rust, and
  use Python only as a **throwaway scratchpad** for the one algorithm whose *shape* is genuinely
  uncertain (the match model) — explore in a notebook, discard, implement the settled design in
  Rust.
- **Calibration / analysis harness → Python, via PyO3 on the Rust core.** Where Python's ecosystem
  genuinely earns its place (pandas/scipy/matplotlib over thousands of seasons). Run the real Rust
  engine, pull results into Python to analyze — both, without a rewrite (the Polars/tokenizers
  pattern).
- **LLM / agent layer → Rust.** I/O-bound (network calls), so no performance argument; and the
  deliberately thin, provider-agnostic interface means the Python-ecosystem pull is weak (no
  LangChain dependency). Rust async (reqwest/tokio) calls LLM APIs fine and keeps the single-binary
  property. Because it talks to the sim only through the serializable `Observation`/`Decision`
  contract, it can be split into a separate process later without touching the core.
- **Management UI → egui** — *held as a lean, not a hard commitment; revisit once the management
  screens have been felt in practice.* Immediate-mode, good at the tables/forms/panels that are
  ~95% of a management UI, ships in the same binary as the sim, one language, no FFI seam. Its
  ceiling is lower than a web stack for very deeply nested navigation — but the UI reads sim state
  across a clean boundary, so outgrowing it later is UI rework that *never* touches the core.
  **Fallback if UI ambition rises: Tauri + a web frontend** (TypeScript over the Rust backend;
  native, lightweight system-webview).
- **Match viewer → Bevy; optional; v2/v3.** The one place the game-engine instinct is legitimate.
  Any viewer is a **pure renderer of the match event stream** (§4.1), so it is fully swappable and
  never entangled with the sim; Bevy competence makes it a low-cost reach. `bevy_ui` is the wrong
  tool for the data-dense management screens — Bevy stays reserved for the rendered match viewer
  and nowhere else.
- **Storage → SQLite.** Queryable and more than sufficient; nothing heavier needed.
- **LLM access** behind a thin, provider-agnostic interface.

*Note on "single process":* a truly self-contained single binary points at all-Rust + egui (sim +
UI + LLM calls in one executable). Tauri gives a higher UI ceiling but bundles a webview —
single-*app*, technically not single-process. The egui lean reflects a preference for the former;
that is the axis to weigh if the UI choice is reopened.

## 9. Development Phases

The meta-principle: **thin vertical slice first, then deepen.**

- **Phase 0 — Design & data model.** Attribute schema, entity model, engine interfaces, a small
  seed dataset (one league).
- **Phase 1 — Walking skeleton.** League + attributes + a *crude* match engine (Elo/Poisson);
  advance a full season; produce a table. No transfers/development/LLM. **Instrument the
  trace/telemetry spine from the start** — it rides on the event log and is far cheaper to build
  in than retrofit, and it is useful for inspecting the deterministic engines before any LLM
  exists.
- **Phase 2 — Match engine depth.** Replace the crude engine with the event-based possession
  model; build the calibration harness; iterate to believable aggregate stats. Likely the longest
  phase. **Design the match event stream schema here for *narratability*, not just outcomes:** four
  consumers depend on it (text commentary, stats aggregation, the journalist agent, the future
  graphical viewer), and enriching it later is a core retrofit — the same
  cheap-as-a-decision / expensive-as-a-retrofit shape the narrative-feedback rules avoid. Forcing
  function: build a **humble text match view** in this phase (it just prints the stream). It is
  nearly free and it proves the stream can tell a match's story minute-by-minute before any
  graphical renderer exists — the thin-vertical-slice instinct applied to the match. The graphical
  viewer itself is deferred to v2/v3 (§8).
  **Sub-phases:** Phase 2 splits into **2a** — the match core (five-zone possession model, the wide
  route, actor-centric resolution; pinned in `MATCH_MODEL.md`) — through **2e** (tactics as
  transition-matrix modifiers, cards/fouls, injuries, set pieces, substitutions, and the
  character/hidden attributes), all landing behind the same `play_match` call site. 2a is settled,
  ported to Rust (`fforge-core::match_engine`), and calibrated: the harness
  (`fforge-core::match_engine::calibrate`, `bin/calibrate`) re-fit `b_beat` against real `worldgen`
  and guards the result with a regression test.
- **Phase 3 — Player development.** CA/PA, age curves, training, diminishing returns; validate
  over decade-long runs. **Pinned in `DEVELOPMENT_MODEL.md`** (the six deferred development decisions
  resolved: PA-gating, the `DevCategory` curve parameters, the event-log seam, in-scope inputs,
  Natural Fitness, validation targets). Settled and implemented in Rust (`fforge-core::development`,
  a monthly `DevelopmentTick`); the career-arc harness (`fforge-core::career_arc`) is built and has
  re-fit the knob table against real `worldgen`, exactly as the match engine's `b_beat` was re-fit.
- **Phase 4 — Transfer market.** Club decision AI, the shared valuation function, windows;
  stress-test for pathologies. **Pinned in `TRANSFER_MODEL.md`** (the centralized valuation
  function, club needs/utility policy, simultaneous deferred-acceptance clearing — resolving the
  sequential-vs-simultaneous ordering question — club finances/contracts, youth intake and
  retirement, and the market pathology harness). Settled and implemented in Rust
  (`fforge-core::{valuation, club_ai, market, pool}`); the market harness
  (`fforge-core::market::calibrate`) is built and has re-fit `ValueKnobs::beta` and
  `FinanceKnobs::revenue_per_reputation` against real `worldgen`, exactly as the match engine's
  `b_beat` and development's knob table were re-fit.
- **Phase 5 — LLM narrative layer + feedback loop.** Agents, propose/dispose, the validation
  gate, bounded/decaying influence; optional with fallbacks. The **agent evaluation methodology**
  (which axes, baselines, rubrics) crystallizes here, once real agents produce real traces.
- **Phase 6 — UI/UX, balancing, content.**

Throughout: the sim stays **headless**; the calibration/eval harness stays **first-class**.

## 10. Open Questions (deliberately unresolved)

*(Resolved and moved out of this list: **immediate next step** — Phase 0, the attribute schema, is
decided; **tech-stack commitment** — settled in §8, Rust core built once; **agent resolution order**
within a window — settled simultaneous, deferred-acceptance (Gale–Shapley-flavoured), `TRANSFER_MODEL.md`
§5.)*

- **The narrative influence model:** how a validated beat maps to a bounded morale/confidence
  nudge (Phase 5; a calibration/taste problem, not architectural).
- **UI toolkit:** egui (the current lean) vs Tauri + web — held open until the management screens
  have been felt in practice (§8). Not blocking: Phase 0 interfaces are language- and
  toolkit-agnostic.
