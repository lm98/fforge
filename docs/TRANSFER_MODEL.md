# Transfer Model — Phase 4 Design Note

The design record for the transfer market, the club decision layer, and the centralized valuation
function (`DESIGN.md` §4.3, Phase 4). It pins the decisions reached in discussion and the reasoning
behind them, in the same spirit as `DESIGN.md`, `ATTRIBUTE_SCHEMA.md`, `MATCH_MODEL.md`, and
`DEVELOPMENT_MODEL.md`: a living artifact to reference and extend.

This note resolves the long-standing **agent resolution order** question (`DESIGN.md` §10) and pins
the **valuation function** — the last unbuilt design-once artifact, and the one Phase 5's agent layer
depends on.

---

## 1. Purpose & status

- **Status:** Phase 4 — *structure settled here; knob table plausibility-picked and re-fit by the
  Rust harness (§11).* No `fforge-core` implementation lands with this note; it is the settled design
  the implementation follows.
- **In scope (this pass):** the centralized valuation function; club needs assessment and a
  utility-based buy/sell policy; simultaneous market clearing inside time-gated windows; club
  finances and player contracts; youth intake and retirement (the pool's two ends); the append-only
  event-log seam; the market pathology harness.
- **Deferred:** **human transfer decisions** (§10 — the human's club is AI-run in the market for
  v1); **form as a valuation input** (§2.5 — no per-player performance signal exists yet);
  **loans**, **agents/negotiation rounds**, **multi-league transfers**, **transfer clauses**
  (release clauses, sell-on percentages); **scouting fog-of-war** (§2.6 — Phase 5, a wrapper on this
  function, not a change to it).

### 1.1 On the missing Python scratchpad — a deliberate departure

`MATCH_MODEL.md` and `DEVELOPMENT_MODEL.md` were both preceded by a throwaway Python shape-finder.
**Phase 4 skips it, on purpose.**

The two earlier scratchpads were fitting *single-entity* laws — one possession sequence, one career
arc — that a toy could faithfully reproduce. A transfer market's behaviour is **emergent from twenty
real squads under real policies**: rich-get-richer, fee inflation, and talent concentration cannot be
shape-found on synthetic stand-ins, because the stand-ins are precisely what's in question. A Python
replica would have to reimplement `worldgen`, `ROLE_WEIGHTS`, and the development fold to say
anything — at which point it is a second implementation, not a scratchpad.

Both earlier knob tables were re-fit against real `worldgen` by their Rust harnesses anyway
(`b_beat` for the match engine; `env_phys`/`plast_*`/`e_*` for development). **The `market` harness
(§11) is Phase 4's shape-finder.** The knob table below is a documented plausibility-picked starting
point in the same sense `ELO_SCALE_S` is — not a fitted result.

*One exception:* the valuation curve alone (§2) is a **single-player pure function** and is cheaply
plottable. A ~40-line numpy sanity plot — value vs. age for a handful of (CA, PA) pairs, checking the
wonderkid/veteran crossover sits where taste says it should — is worth doing before the Rust lands.
That is a plot, not a prototype.

---

## 2. The valuation function — the design-once artifact

`DESIGN.md` §4.3: *"Centralize the valuation function (attributes, age, potential, contract length,
form, positional scarcity). It is reused by match-engine role-weighting, the transfer AI, and the LLM
agents. Design it once."*

Its consumers: the club AI's buy/sell utility (§6), the market's reservation prices (§5), the Phase-5
agent observations, and the eventual management UI. It **consumes** `ROLE_WEIGHTS`
(`ATTRIBUTE_SCHEMA.md` §5) and the development envelope (`DEVELOPMENT_MODEL.md` §2.1) — it is a
consumer of those artifacts, not a rival to them.

**Home:** `fforge-core::valuation`. It needs `DevKnobs`, which is core's — so it cannot live in
`fforge-domain`. It is a Layer-2 pure function consumed by Layer 3 and Layer 4, which is exactly the
intended shape: the *decision* layer decides, the *sim* layer prices.

```rust
pub fn value(world: &World, player: PlayerId, today: GameDate,
             ctx: &MarketContext, knobs: &ValueKnobs) -> Money
```

Pure. No RNG, no clock, no I/O — the same bar `play_match` and `tick_changes` meet.

### 2.1 The shape

```
value = V0 · exp(β · (ca_eff − ca_ref)) · contract_mult · scarcity_mult

ca_eff = Σ_{t=0..H} δᵗ · ca_proj(p, t)  /  Σ_{t=0..H} δᵗ
```

Two structural choices carry it.

### 2.2 Convexity in ability

**Decision: exponential in CA.** A linear (or even mildly convex) valuation makes the club AI prefer
quantity to quality — three 70s beat one 85 on aggregate CA at equal cost — squads homogenise, and
elite players stop being worth building a club around. Real markets are radically convex; so is this
one.

Exponential rather than a power law because **CA is an interval scale, not a ratio scale**: zero CA
is not a meaningful zero, so `(ca/ca_ref)^γ` has no principled anchor. The exponential's parameter
has a clean tuning statement instead: *every `ln2/β` CA points doubles value*.

### 2.3 `ca_eff` — pricing the career, not the day

**Decision: the base curve takes a discounted mean of *projected* future CA, not current CA.**

A fee buys a player's future ability. Putting the projection *inside* the exponential (rather than
bolting an `age_mult` and a `pa_mult` outside it) means there is **one curve**, and the convexity
applies to the whole career profile.

`ca_proj(p, t)` runs the `DEVELOPMENT_MODEL.md` §2 growth law forward from the player's **current**
attributes with **jitter = 0**, then reports best-role CA at year `t`. Worked shape:

| Player | now | projected path | `ca_eff` | priced as |
|---|:-:|---|:-:|---|
| 19yo, PA 88 | 62 | 62 · 68 · 74 · 79 · 83 · 86 · 87 · 88 | **~76** | well above current |
| 33yo | 80 | 80 · 78 · 75 · 72 · 68 · 63 · 57 · 50 | **~71** | below current |

Age depreciation, PA headroom, the plasticity window, and — for free — **`DevCategory`-dependent
aging** (a physically-reliant winger depreciating faster than a technical centre-back, per
`ATTRIBUTE_SCHEMA.md` §7) all emerge from machinery already built and already calibrated.

*Alternative rejected:* separate hand-fitted `age_mult` × `pa_mult` factors. Cheaper to write, but it
is a second encoding of curves that already exist — two tables to keep consistent, and they will
drift. The whole point of §2.1's envelope was that the `DevCategory` tag *is* the curve family.

**Projection granularity — monthly, not annual.** The instinct is to project annually for cheapness,
but that requires a second, coarser integrator that can drift from the real monthly law. Cost of the
faithful option: 480 players × 8 years × 12 months × ~25 attributes ≈ 1.2M attribute-steps for a
full-league pass — sub-millisecond in Rust, twice a season. **Reuse `tick_changes`'s law directly**
(a `project`-flavoured variant with noise disabled); no second integrator exists to drift.

**Projection assumptions — both neutral, both deliberate:**

- **Minutes: `minutes_regular` (1.0).** The economic question a fee answers is *what is he worth to a
  club that will play him*, not what he is worth rotting on his current bench.
- **Coaching: 1.0**, not the current club's `coaching_milli`. Same reason: a player is not worth less
  because his present academy is poor.

Both are documented counterfactuals, not oversights. They also keep valuation independent of the
holding club, which §5's simultaneity requires.

**Horizon and discount:** `H = 8` years, `δ = 0.88`. Starting points; harness-refit targets.

### 2.4 The multipliers

**Contract.** `contract_mult = 1 − c · (1 − min(yrs_left, T)/T)`, with `T = 3`, `c = 0.6`. Full value
at 3+ years; ~0.6 at one year; ~0.4 in the final months. This creates the sell-now-or-lose-him
decision — one of the genuinely good decisions the genre offers — and it is the mechanism that makes
§4's `ContractRenewed` load-bearing rather than bookkeeping.

**Scarcity.** League-wide supply of role-capable players against formation-implied demand, **bounded
to [0.85, 1.20]**. Near-1.0 at t=0 by construction (`SQUAD_TEMPLATE` is uniform); its job is to react
to drift over decades, and it is the natural home for youth-intake imbalance (§8). Bounded because an
unbounded scarcity term is an inflation engine.

### 2.5 Form — deliberately absent from v1

`DESIGN.md` §4.3 lists form as a valuation input. **It is not implementable today**, and the reason
is worth recording rather than quietly skipping.

The match event stream (`MATCH_MODEL.md` §9) carries `MatchEvent { minute, side, zone, kind }` — the
resolver samples an actor from the presence table (§6) and then **discards their identity**. There is
no per-player performance signal anywhere in the system: `MatchPlayed` records the two XIs, so
*appearances* exist, but nothing distinguishes a striker who scored a hat-trick from one who missed
five.

Team-results-and-appearances would be a poor proxy, and a poor proxy in a design-once artifact is
worse than an honest absence. **Form enters valuation when the stream carries actors** — see §12
item 1, tracked as the Phase-2b addendum.

### 2.6 Hidden information — and why that is correct

`ca_proj` reads `Character::potential` and `DevProfile` — both **hidden**. This is deliberate, and it
resolves cleanly against `ATTRIBUTE_SCHEMA.md` §1's note that attribute visibility "is precisely the
`Observation` vs `info` split."

**`value()` is ground truth — the `info` channel.** A club's *perceived* valuation is a masked, noisy
observation of it. Phase 5's scouting fog-of-war is therefore a **wrapper** on this function, not a
change to it: same function, degraded inputs.

**v1 consequence, flagged so the harness is read correctly:** with no wrapper, every club is an
omniscient valuer. All twenty clubs identify the same wonderkids and price them identically. The
market will look *eerily efficient*, and any talent-concentration reading from §11 is an
**upper bound** on concentration under perfect information — not a prediction of the fogged game.
That is the right baseline to measure fog-of-war against later; it is not a bug to tune away.

### 2.7 Caching and the frozen snapshot

Valuation is called once per (club × shortlist candidate) during clearing. Compute **once per window**
into a `BTreeMap<PlayerId, Money>` and read from it. This is not only an optimisation: it *guarantees*
every club values against the same frozen world snapshot, which §5's simultaneity requires.

---

## 3. Domain extension — the first new-feature change to `fforge-domain` since Phase 0

`fforge-domain/CLAUDE.md` states changes at this stage are "corrections or clarifications to the
Phase 0 deliverable, not new features." Phase 4 is the sanctioned exception `Club`'s own comment
anticipates (*"Finances/budget arrive with Phase 4"*). Kept to the minimum that earns its keep.

```rust
/// Whole currency units — not cents. Nobody negotiates a fee to the cent, and
/// the extra two digits buy nothing but overflow headroom we do not need.
pub struct Money(pub i64);

pub struct Contract { pub wage: Money, pub expires: GameDate }   // wage annual

pub struct Finances { pub balance: Money, pub wage_budget: Money }
```

- **`Player.contract: Option<Contract>`** — `None` = free agent.
  *Alternative rejected:* a `World`-level `BTreeMap<PlayerId, Contract>`. It must be kept in sync with
  `Club.players`, which is exactly the store-then-resync bug `ATTRIBUTE_SCHEMA.md` §1 designed CA to
  make impossible. The contract is a property of the employment; it belongs with the employee.

- **`Club.players` remains the sole club↔player index**; add `World::club_of(PlayerId) -> Option<ClubId>`.
  No `Player.club` field — that is denormalisation, and a transfer would then have two places to
  update and one chance to disagree. `club_of` is O(clubs) on 20 clubs; memoize outside the domain if
  it ever matters.

- **`Club.finances: Finances`** and **`Club.reputation: u8`** (0–100), both resolved at worldgen from
  the existing quality anchor. Reputation clears the rule-of-three: it scales revenue (§4), gates
  player willingness to sign (§5), and seeds the Phase-5 board/president persona.

- **`Money` is `i64`, signed.** Balances genuinely go negative when a club overreaches, and the
  pathology harness (§11) needs to *see* insolvency rather than have it clamped away.

Contracts are **not** float-free-adjacent trouble: `Money` and `GameDate` are both integers, so
`Contract` derives `Eq` and serializes exactly, holding the domain's float-free invariant.

### 3.1 Worldgen additions

Every existing player needs a contract at t=0, and every club finances. Both are resolved in
`worldgen` and ride in the `World` snapshot `GameStarted` already records — **no new event, no
migration**, exactly the `event.rs` principle-1 shape.

- Contract expiries spread 1–5 years out, correlated with age and quality (young, good players on
  longer deals) so the first window has natural expiry pressure rather than a uniform cliff.
- `balance` and `wage_budget` scale with `reputation`; initial wages scale with player value so the
  league starts near its own wage equilibrium rather than lurching toward it in season one.

**Wage budget is a constraint, not a second cash pot.** Committed annual wages (Σ over the squad's
contracts) must stay ≤ `wage_budget`. FM's two-pot model needs inter-pot transfers and accumulates
fiddly rules; one cash balance plus one commitment ceiling delivers both stabilizers `DESIGN.md` §4.3
asks for with half the machinery — you cannot spend cash you lack, and you cannot accumulate wage
commitments beyond your structure.

---

## 4. The event-log seam

The two `event.rs` principles apply unchanged: record resolved values; record outcomes the fold
consumes without re-running engines.

**Bids are not events.** A window produces a rich `WindowOutcome { transfers, rejected_bids,
valuations, unfilled_needs }`, of which **only the completions become events**; the rest is a
**Trace**. This is structurally identical to `MatchOutcome.stream` (`MATCH_MODEL.md` §7) and to the
Phase-5 event/trace split (`DESIGN.md` §6) — and the rejected bids are precisely the material the
journalist agent will want ("*City's third bid rejected*"), so the Trace is not discarded, merely kept
out of the fold.

```rust
TransferCompleted { date, player, from: Option<ClubId>, to: ClubId, fee: Money, contract: Contract }
PlayerReleased    { date, player, club }
ContractRenewed   { date, player, club, contract }
YouthIntake       { date, club, players: Vec<Player> }
PlayerRetired     { date, player }
FinanceTick       { date, deltas: Vec<(ClubId, Money)> }
```

`YouthIntake` records the **generated players** themselves, not a seed — the same choice `GameStarted`
makes about the world, for the same reason: improving youth generation must never rewrite a recorded
career.

**`FinanceTick` is to money what `DevelopmentTick` is to attributes.** Monthly, riding the *same*
30-day period boundary crossing (`period_index`, `DEV_TICK_PERIOD_DAYS`), carrying resolved per-club
deltas that the fold integer-adds. Wages debited, revenue accrued, no re-derivation, no float. The
symmetry is not decorative — it means `commands::step` grows one more tick emitter beside
`dev_ticks_between`, not a new subsystem.

**Fold semantics** (`state::apply`, pure — no RNG, no math beyond integer add):

| Event | Fold action |
|---|---|
| `TransferCompleted` | remove `player` from `from`'s `players` (if any); push to `to`'s; set `Player.contract`; debit `to`, credit `from` |
| `PlayerReleased` | remove from roster; `contract = None` |
| `ContractRenewed` | replace `Player.contract` |
| `YouthIntake` | insert players into `World.players`; push ids onto the club's roster |
| `PlayerRetired` | remove from roster; `contract = None`; mark retired (see §8.2) |
| `FinanceTick` | integer-add each delta to the club's `balance` |

Rosters are `Vec<PlayerId>`; keep them **sorted** after mutation so the fold's output is
order-independent and `GameState` equality stays meaningful across replay paths.

---

## 5. Market resolution — **simultaneous, deferred-acceptance rounds**

**This resolves `DESIGN.md` §10's standing question.** The lean was simultaneous; the argument for it
is stronger than "lean" suggested.

**Why not sequential.** Under sequential resolution the first club to act takes the best available
player unopposed. That manufactures rich-get-richer **as an artifact of the scheduling** — so the
pathology harness (§11) would be measuring its own iteration order rather than the economics, which
defeats the purpose of building it. And it is fatal for the project's distinctive goal: in Phase 5 an
LLM agent's measured decision quality would depend on its slot in the queue, contaminating agent
evaluation at the root.

**The loop.** Deferred acceptance (Gale–Shapley-flavoured) — the right prior art, and it terminates
provably.

1. **Freeze** the world snapshot and the valuation cache (§2.7). Each club computes needs (§6) and a
   ranked shortlist with a reservation price per target.
2. **Bid.** Each club submits **one** bid, for its top unfilled need. One, not *k* — it forces
   prioritisation, and it keeps the contention rule simple.
3. **Resolve contention** per target. The selling club ranks offers by its own utility (fee, then
   buyer reputation, then `ClubId`). The player then **consents or refuses** on wage and buyer
   reputation against his own threshold; a refusal falls through to the next-best bidder — the
   deferred-acceptance step.
4. **Carry losers forward**, shortlists minus players already taken.
5. **Repeat** to fixpoint or `MAX_ROUNDS = 12`. Termination: each round either completes at least one
   transfer or produces no bids.

**Determinism.** Clubs iterate in `ClubId` order over `BTreeMap`. All randomness comes from
`derive_stream(seed, TRANSFER_STREAM_NS | window_index)` and is **drawn unconditionally in a fixed
order**, mirroring `tick_changes`'s deliberate choice to keep stream position value-independent.
Ties break on `ClubId` / `PlayerId`, never on iteration accident.

Suggested namespaces, following the existing `"MATC"` / `"DEVE"` convention:
`TRANSFER_STREAM_NS = 0x5452_414E_0000_0000` (`"TRAN"`),
`FINANCE_STREAM_NS = 0x4649_4E41_0000_0000` (`"FINA"`),
`YOUTH_STREAM_NS = 0x594F_5554_0000_0000` (`"YOUT"`).

---

## 6. Club decision policy

```
need(club, role) = w_depth · depth_gap + w_quality · quality_gap + w_age · succession_risk
```

- **`depth_gap`** — `SQUAD_TEMPLATE` headcount vs. current, by `natural_role`.
- **`quality_gap`** — the club's best CA in that role against **its own reputation-implied target
  level**, *not* the league mean. This matters: measured against the league mean, all twenty clubs
  chase the same three superstars and the bottom half bids uselessly every window, producing a market
  that looks broken for a reason that is purely an evaluation-baseline error.
- **`succession_risk`** — the current starter's **projected** CA in 2–3 years falling below target.
  Third consumer of §2.3's projection, and it produces the recognisably football behaviour of clubs
  replacing aging starters *before* they collapse.

**Utility of a signing** = `need(role) · (value − asking_price)`, filtered by cash and wage headroom.
Surplus, not price: a bargain in a position of no need is still not worth the squad slot.

**Selling is in v1.** Without it the market is one-directional — budgets deplete, squads only grow,
nothing clears, and the harness measures a ratchet. A club lists a player when he is surplus to depth,
expiring-within-a-year and not worth renewing, or when a standing offer exceeds `value` by a margin.

**Stabilizers** (`DESIGN.md` §4.3's "squad-size limits, financial constraints, players wanting
minutes"): hard squad bounds **[18, 30]**; per-role minima (**≥ 2 GK** — a club with no keeper is a
crash, not a strategy); cash and wage-budget constraints as hard filters; and player consent (§5 step
3), which is the "wanting minutes" lever in its first, coarse form.

### 6.1 The policy trait — define now, extract later

`ai_pick_lineup` is already described in-code as "the Phase-1 stub of the layer-3 club decision AI."
Phase 4 is where that stub becomes real, and Phase 5 must **substitute an LLM at the same seam** for
`DESIGN.md` §5's LLM-vs-utility-baseline ablation ("a config change, not a rewrite").

**Decision: define the trait now in `fforge-core::club_ai`; do not extract a crate yet.**

```rust
pub trait ClubPolicy {
    fn transfer_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision>;
}
```

Shaped in the spirit of the Gym contract — a **plain serializable struct in, a constrained enum out,
no world internals read, no state mutated** — without building the Gym/PettingZoo wrapper, which
`DESIGN.md` §9 places in Phase 5. Phase 5 then *wraps* rather than rewrites.

The crate extraction waits for a second implementation to exist. That is `DESIGN.md` §2's own
"reusability is an extraction, not a prediction," applied to itself rather than exempted from itself.
*Counterargument on record:* the five layers are explicit in `DESIGN.md` §3 and a real crate boundary
enforces what a module boundary merely documents. Revisit at Phase 5's first agent.

---

## 7. Window mechanics

**Windows are defined relative to the season, not by absolute day-of-year.** The flat 365-day calendar
(`GameDate`) and a 38-matchday `double_round_robin` schedule mean the season's span in days depends on
matchday spacing; anchoring windows to day-of-year constants would silently break if that spacing ever
changes.

- **Summer window:** opens at `SeasonEnded`, closes `N` days after `SeasonStarted`.
- **Winter window:** a ~30-day span around the schedule midpoint.

**No new command.** The market resolves when a window boundary is crossed, on exactly the mechanism
`dev_ticks_between` already uses — `Command::AdvanceMatchday` and `Command::StartNextSeason` cross the
dates, and `commands::step` emits the resulting events. The transfer market is a *tick*, like
development and finance, not a new interaction mode.

---

## 8. The player pool needs both ends

Development alone leaves the pool static. Once §6 can release players, the pool needs an inflow; once
the sim runs a decade, it needs an outflow. **Both belong in v1** — without them, §11's decade-long
metrics measure a draining or geriatric pool rather than market dynamics, and every reading is
confounded.

### 8.1 Youth intake

An annual intake at the summer window: a small cohort per club, generated by reusing
`worldgen::gen_player` with a youth age band (16–18) and quality anchored on the club's `reputation`
and `coaching_milli`. This is the **second genuine consumer of `coaching_milli`** — the academy lever
finally does two things instead of one.

Recorded as `YouthIntake { date, club, players }` (resolved players, §4).

### 8.2 Retirement

A player retires at the summer window when age ≥ 34 and either his best-role CA has fallen below a
league-relevance floor or he has gone a full season unsigned. `PlayerRetired { date, player }`.

Retired players stay in `World.players` (the log references them; removing them would break replay of
historical `MatchPlayed` XIs) but leave every roster and are excluded from the development tick and
the market.

### 8.3 The clubless-player edge case

A released, unsigned player is still iterated by `development::tick_changes`. Two inputs break:
`club_matches_since_tick` has no entry for him (→ `minutes_absent`, which is correct and needs no
change), and **`coaching` has no club to read** (→ currently a lookup that does not exist). Resolve by
using a neutral `coaching = 1.0` for clubless players. Small, but it is a genuine panic waiting in the
fold if left implicit.

---

## 9. Knob table — a plausibility-picked starting point

In the sense of `ELO_SCALE_S` (`MATCH_MODEL.md` §10 item 6): documented modelling choices, **not
fitted results**. The §11 harness re-fits them, exactly as `b_beat` and `env_phys` were re-fit.

| Knob | Start | Meaning |
|---|:-:|---|
| `ca_ref` | 60 | CA at which value = `V0` |
| `V0` | 1 500 000 | Value anchor at `ca_ref` |
| `beta` | ln2 / 6 ≈ 0.1155 | +6 CA points doubles value |
| `horizon_years` (H) | 8 | Projection horizon |
| `discount` (δ) | 0.88 | Annual discount on future ability |
| `contract_full_years` (T) | 3 | Years at which contract discount vanishes |
| `contract_max_discount` (c) | 0.60 | Discount at zero years remaining |
| `scarcity_bounds` | [0.85, 1.20] | Bounded, deliberately |
| `squad_min` / `squad_max` | 18 / 30 | Hard bounds |
| `max_rounds` | 12 | Clearing-loop cap |
| `wage_share_of_value` | ~0.18/yr | Wage demanded as a share of value |
| `revenue_per_reputation` | — | Annual revenue ∝ reputation; fit against wage bills |

The two that most need the harness: **`beta`** (too flat → homogenised squads; too steep → one club
buys the league) and **`revenue_per_reputation`** (sets whether the market clears at all).

---

## 10. Human transfer decisions — deferred, with the seam left open

**Decision: for v1 every club, including the human's, is AI-run in the transfer market.**

Phase 4's deliverable per `DESIGN.md` §9 is "club decision AI, the shared valuation function, windows;
stress-test for pathologies" — the *market machinery*. Human agency over that machinery is a
management-UI concern, and `DESIGN.md` §9 places UI/UX in Phase 6. Folding it in now would widen Phase
4 by a command, a validation surface, a CLI interaction loop, and a set of "what can the human legally
do mid-window" rules that are better designed once the market's own dynamics are known.

**This is a deliberate scoping call, recorded so it is not mistaken for an oversight.** It has a real
cost: for the duration of Phase 4 the game is, in the transfer window, a spectator sport.

**The seam is left open, not closed.** The follow-on is small precisely because §5 and §6 are built
first:

```rust
Command::SubmitTransferDecision(TransferDecision)   // → Event::TransferDecisionSubmitted
```

It slots into the identical propose-then-validate gate as `Command::SubmitLineup` — a human proposal,
validated in `commands::step` (squad bounds, cash, wage headroom, target availability), recorded as a
resolved decision, and entering §5's clearing loop as one more bidder among twenty. `ClubPolicy`
(§6.1) is the substitution point: the human's club swaps its utility policy for a "read the recorded
decision" policy, which is the *same* substitution Phase 5 performs for LLM agents. Building it for
the human first would have been building it twice.

**Tracked as a Phase-6 task** (`PHASE4_TASKS.md`, "Deferred / follow-on"), promotable to late Phase 4
if the spectator-window feel proves intolerable before Phase 5 arrives.

---

## 11. Validation targets & the harness

`fforge-core::market` + `bin/market` + a `market_is_in_a_believable_ballpark` regression guard — the
sibling of `match_engine::calibrate::StreamTelemetry` / `bin/calibrate` /
`favourite_discrimination_regression_guard` and of `career_arc` / `bin/career_arc` /
`career_arcs_are_in_a_believable_ballpark`. A **passive consumer** of the event stream and the window
Traces; it never writes to the world (`DESIGN.md` §5).

Pooled over **many seeds × ~15 seasons**. This is the third phase in which pooling has proved
necessary — a single synthetic league swung goals-per-match by ±0.4 (`MATCH_MODEL.md` §10 item 4) and
per-seed career-arc spread forced the same discipline in `career_arc`. **Treat multi-seed pooling as a
project-wide harness invariant**, not a per-phase rediscovery. Competitive-balance metrics need it
most: a single league's Gini trajectory is nearly meaningless.

| Metric | Believable band | Catches |
|---|---|---|
| Transfers per club per window | ~2–5 | dead or hyperactive market |
| Fee p90 / median | high | convexity actually holding |
| Points-Gini across a decade | **stable, not monotonically rising** | rich-get-richer runaway |
| Season-to-season rank churn | non-zero | a frozen hierarchy |
| Top-3 clubs' share of league top-20 players | elevated but bounded, non-rising | talent monopolization |
| Median fee, yr15 / yr1 | < ~2× | fee inflation |
| Clubs insolvent / hoarding cash | neither unbounded | broken financial loop |
| League mean age; squad sizes; role coverage | stable; all in [18,30]; ≥2 GK everywhere | intake/retirement imbalance |

Wide bands, gross-regression tripwires — **not fit gates**, in the explicit spirit of both sibling
guards. A curve that has come loose from the design should fail; a knob nudged by 5% should not.

Read §2.6 before interpreting the concentration rows: under v1's perfect information they are an
**upper bound**, not a prediction.

---

## 12. Open sub-questions

Deliberately unresolved, to settle during implementation or the §11 calibration pass.

1. **Player identity in the match event stream (Phase-2b addendum).** `MatchEvent` carries no
   `PlayerId`; the resolver samples an actor and discards it. This blocks form as a valuation input
   (§2.5) and — more seriously — blocks the **journalist agent**, which cannot write "*Rossi scored at
   73'*" from a stream that does not know Rossi was there. `MATCH_MODEL.md` §9 designs the stream for
   narratability and this is the single place it under-delivers. The fix is small (`actor: PlayerId`,
   plus an optional `opponent: PlayerId`, on the beats that have one) and it is **cheapest now**,
   before the stream gains further consumers. Sequenced as **P4.0**.
2. **Wage negotiation.** Wages are currently a deterministic function of value and reputation. Whether
   a genuine negotiation round earns its keep — or is better as a Phase-5 *agent* behaviour on top of
   a deterministic floor — is open.
3. **Free-agent pool dynamics.** Unsigned players persist and continue to develop (§8.3). Whether the
   pool bloats over a decade is a §11 reading, not a prediction.
4. **Loans.** Genuinely useful (young players getting minutes elsewhere is the development system's
   natural partner) and genuinely a second market. Deferred entirely; revisit once §11 shows the
   primary market is stable.
5. **Reputation dynamics.** `Club.reputation` is static at worldgen in v1. Making it respond to league
   finishes closes a real feedback loop — and is a plausible rich-get-richer *amplifier*, so it should
   land only after §11 has a baseline without it.
6. **Asking price vs. valuation.** v1 sets a selling club's ask as a markup on `value`. Whether clubs
   should hold idiosyncratic private valuations (the honest multi-agent model) is deferred until
   fog-of-war (§2.6) makes private valuations meaningful rather than arbitrary.
