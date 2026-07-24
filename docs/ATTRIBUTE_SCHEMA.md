# Attribute Schema — Phase 0.1

The keystone artifact. Everything downstream reads this: the match engine consumes attributes as
action inputs, the development engine grows them along age curves, and valuation + CA aggregation
+ transfer AI all reuse the role-weighting table defined here. Designed **consumption-first** —
every attribute earns its place by feeding a match action or modifying a named system (§6); none
is decoration.

Scope: the attribute set, CA/PA semantics, the role→attribute weighting, and the attribute→action
coverage map. The resolution *math* (how a pass or shot is computed) is Phase 2, pinned in
`MATCH_MODEL.md`; the growth *math* (age-curve parameters) is Phase 3, pinned in
`DEVELOPMENT_MODEL.md`. This file fixes the *shape* those phases build on; where a §-level decision
below has since been resolved there, it is marked **[resolved]** with the pointer.

---

## 1. Ground rules (ratified)

- **Scale:** every rated value is an integer `0–100`. Not floats (exact serialization/hashing,
  no float edge-cases in the deterministic fold), not 1–20 (0–100 gives calibration headroom).
  Any display scale (1–5 stars, 1–20) is a presentation-layer transform on top.
- **CA is derived, never stored.** `CA(player, role)` = role-weighted mean over attributes (§4).
  Attributes / CA / PA stay consistent *by construction* — no store-then-resync bug.
- **PA is stored, hidden, on the same 0–100 scale** = the player's peak attainable best-role CA.
- **Two functional classes of rated field:**
  - **Performance attributes** — contribute to CA and drive match actions. 25 of them (§2).
  - **Character / hidden attributes** — drive development rate, match-to-match variance, injury
    events, and morale/captaincy systems. **Never contribute to CA.** 6 of them (§2), incl. PA —
    7 since Phase 2e added Natural Fitness (§3's sanctioned split, `MATCH_MODEL.md` §13).
- **Attribute *visibility* is out of scope here but has a natural home.** What a scout/manager
  *perceives* about a player vs. the true rated value is precisely the `Observation` vs. `info`
  split from the agent interface (fog-of-war = scoped observation; truth = evaluator channel).
  Deferred, but the schema stores *true* values and observation masks them later — not the reverse.

---

## 2. The attribute set

Each performance attribute carries a **development category** (`Phys`/`Tech`/`Ment`) fixing its
age-curve family (§7) and an **applicability** (outfield / GK / both).

### Performance — Technical (`Tech` category)

| Attribute | Applies | Drives (short) |
|---|---|---|
| Finishing | Outfield | Converting chances; shots from distance (with a range penalty) |
| Passing | Both* | Pass completion, weight, range |
| Ball Control | Both* | Receiving under pressure, first touch, retention in tight space |
| Dribbling | Outfield | Beating a man, carrying the ball |
| Tackling | Outfield | Winning the ball in a challenge (cleanly) |
| Marking | Outfield | Denying space, tracking a runner |
| Heading | Outfield | Aerial duels, attacking and defensive |
| Crossing | Outfield | Delivery quality from wide areas |

\*GKs draw on Passing/Ball Control weakly; their primary ball-playing is **Distribution** (GK group).

### Performance — Mental (`Ment` category)

| Attribute | Applies | Drives (short) |
|---|---|---|
| Vision | Both | Seeing and selecting the incisive pass |
| Decisions | Both | Choosing the right action; broad modulator across contests |
| Def. Positioning | Both | Defensive shape, interceptions, GK angles |
| Off-the-ball | Outfield | Attacking movement, getting into scoring positions |
| Composure | Both | Performing under pressure (finishing, passing when pressed, not fouling) |
| Concentration | Both | Avoiding lapses; error rate, esp. when fatigued |
| Work Rate | Both | Pressing, tracking back, distance covered |
| Aggression | Both | Duel intensity, pressing, and foul propensity |

### Performance — Physical (`Phys` category)

| Attribute | Applies | Drives (short) |
|---|---|---|
| Speed | Both | Closing down, chasing, counters, dribble/track (Pace+Acceleration merged) |
| Stamina | Both | Fatigue rate over the match; modulates everything late |
| Strength | Both | Physical duels, holding off, resisting challenges |
| Agility | Both | Quick turns, dribble/tackle recovery, GK diving reach (Balance merged) |
| Jumping | Both | Aerial reach; GK claiming high balls |

### Performance — Goalkeeping (`Ment`/`Phys`-shaped curves; §7)

| Attribute | Applies | Drives (short) |
|---|---|---|
| Reflexes | GK | Reaction shot-stopping |
| Handling | GK | Holding vs. spilling; catching |
| Command of Area | GK | Claiming crosses, organizing, coming off the line |
| Distribution | GK | Starting attacks (kicking/throwing) |

GKs *also* draw on shared attributes — Def. Positioning, Decisions, Agility, Jumping, Composure,
Concentration (see the GK column in §5).

### Character / hidden (never in CA)

| Field | Hidden? | Drives |
|---|---|---|
| **PA** (Potential) | Yes | Development ceiling = peak attainable best-role CA (§4) |
| Determination | Partly | Development rate; big-match modifier; **persona seed** (Phase 5) |
| Professionalism | Partly | Training gain; aging/injury resistance; **persona seed** (Phase 5) |
| Consistency | Yes | Match-to-match variance (how reliably a player hits their CA) |
| Injury-proneness | Yes | Weighting on injury events |
| Natural Fitness | Yes | Between-match condition recovery — *added at Phase 2e* (§3's flagged split fired: recovery modeling gave it a genuine consumer, `MATCH_MODEL.md` §13) |
| Leadership | Partly | Morale propagation / captaincy — a **system modifier**, not a match action |

**Why Determination/Professionalism are worth defining now** despite the LLM layer being Phase 5:
they're development-rate drivers the game needs *with or without* agents, and they double as the
seed for agent character sheets later — so defining them costs nothing extra, while richer persona
data stays a Phase-5 extension point rather than something over-built now.

---

## 3. Merged & deferred (the granularity lever)

Trimmed from FM-style depth by merging correlated attributes. **Every merge is cheap to reverse
upward and painful to reverse downward** — hence the lean-and-add default. Candidates to split
later if calibration or design demands it:

- **Speed** = Pace + Acceleration. Split if burst-vs-top-speed roles need distinguishing.
- **Ball Control** = Technique + First Touch.
- **Agility** = Agility + Balance.
- **Decisions** absorbs **Anticipation** (reading play) — split if defensive reading needs its own knob.
- **Aggression** absorbs **Bravery** (committing to duels/blocks) and drives fouls; a separate
  hidden **discipline** factor may be needed if card rates won't calibrate from Aggression alone.
  **[resolved: Aggression alone in v1 — `MATCH_MODEL.md` §15.]** The foul/card contest starts
  with no hidden factor, and §15 states the split tripwire in advance (a too-flat per-player
  card tail, or duel-balance distortion when widening it) so the decision is checkable, not
  silently defaulted.
- **Finishing** absorbs **Long Shots** (range penalty) and **Penalty Taking**.

**Deferred entirely** (add as a later, optional layer, not needed to prove the schema):

- **Set-piece specialism** (free kicks, penalties, corners) — currently folded into
  Finishing/Passing/Crossing with set-piece context. A `set_piece` specialist sub-rating is the
  natural v-next addition.
- **Natural Fitness** (recovery between matches, physical-aging resistance) — folded into the
  development/fatigue system, parameterized by Professionalism/Injury-proneness for now. Split out
  in Phase 3 if recovery modeling needs it. **[resolved: not split — `DEVELOPMENT_MODEL.md` §4.]**
  Phase 3's monthly loop never touches between-match recovery, so the attribute's only Phase-3 job
  (physical-aging resistance) is already covered by Professionalism; adding it now would violate the
  lean-and-add rule. Revisit at **Phase 2e**, when fatigue/recovery is modeled and it earns a genuine
  second consumer. **[resolved at 2e: split out — `MATCH_MODEL.md` §13.]** The revisit tripwire
  fired: between-match recovery is now modeled, no existing attribute can carry it without
  double-dipping (Stamina already owns in-match fade; Professionalism is a training/aging trait),
  so Natural Fitness enters as a hidden Character field (§2) whose only v1 consumer is the recovery
  law — the aging-resistance term deliberately stays with Professionalism (no Phase-3 re-fit).
- **Flair** — dropped as a mechanical attribute; a candidate *persona* trait (Phase 5).
- **Role variants** (§5) — refinements on the archetypes, layered later.

---

## 4. CA / PA semantics

**Current Ability** is a pure function of attributes and a role, computed on demand:

```
CA(player, role) = round( Σ_i  w[role][i] · attr[i]  /  Σ_i w[role][i] )   ∈ 0..100
```

where `w[role][i]` is the importance weight from §5. Consequences, both intended:

- **CA is position-relative.** The same player is (say) an 82 as a Full-Back and a 68 as a Winger.
  A player's *headline* CA is CA in their assigned (or best) role. This is more correct than a flat
  "overall," and it's the inverse of FM's store-CA-then-fit-attributes approach.
- **No sync bug is possible** — attributes are the single source of truth; CA is a view.

**Potential Ability** is the hidden ceiling on development:

- Stored per player, hidden, `0–100`, interpreted as **peak attainable best-role CA**.
- Development (Phase 3) raises attributes so that best-role CA trends from current toward PA, with
  **diminishing returns near the ceiling**. Wonderkid = large PA−CA gap; flops and late bloomers
  come from PA being an *estimate* plus development noise (Phase 3).
- **Exactly how PA gates per-attribute growth is a Phase-3 decision.** The schema commitment here
  is only: PA is stored, hidden, same scale as CA, and caps collective growth. *Lean:* gate on a
  best-role-peak-CA measure (keeps PA/CA directly comparable). *Alternative:* a position-agnostic
  "attribute budget." **[resolved: best-role-peak-CA gate — `DEVELOPMENT_MODEL.md` §2.2.]** The lean
  was taken: growth targets are scaled so a fully-realized player's peak best-role CA equals PA
  exactly, reusing this section's role weights, so "PA = peak best-role CA" holds as one comparable
  scale. The attribute-budget alternative was rejected for severing PA from role. (`fforge-core`
  realizes it with a role-shaped per-attribute ceiling so per-attribute growth keeps role shape —
  `DEVELOPMENT_MODEL.md` §2.2's implementation note.)

---

## 5. Role → attribute weighting (a design-once artifact)

One of the design-once / consumed-by-many artifacts (alongside the valuation function
**[resolved: `TRANSFER_MODEL.md` §2]** and the match event stream). It feeds **CA aggregation (§4),
match team-quality contribution, valuation, and the transfer AI's needs assessment** — one table,
four consumers.

> **Sibling artifact (Phase 2):** the match model introduces a **role→zone _presence_ table**
> (`MATCH_MODEL.md` §6) — a second `role × _` table, easily confused with this one. Keep them
> distinct: **this** table rates *attribute importance per role* (how good a player is at a role);
> **that** one rates *spatial presence per role* (where on the pitch a role is on the ball or
> defending). Different consumers, tuned independently.

Importance scale per role: **5** = defining · **4** = very important · **3** = important ·
**2** = useful · **1** = minor · **0** = not relevant *to rating this role* (≠ "never used in a
match" — it means it doesn't shape how good the player is *at this role*).

Eight archetypal roles span the pitch: **GK, CB** (central defender), **FB** (full-back),
**DM** (defensive mid), **CM** (box-to-box), **AM** (attacking mid / playmaker), **W** (winger),
**ST** (centre-forward). These are *starting* weights — design-time estimates to be adjusted by
Phase 2/3 calibration.

**Technical**

| Attribute | GK | CB | FB | DM | CM | AM | W | ST |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Finishing | 0 | 1 | 1 | 1 | 2 | 3 | 3 | 5 |
| Passing | 1 | 2 | 3 | 4 | 4 | 5 | 3 | 2 |
| Ball Control | 1 | 2 | 3 | 3 | 4 | 5 | 4 | 4 |
| Dribbling | 0 | 1 | 2 | 2 | 3 | 4 | 5 | 3 |
| Tackling | 0 | 5 | 4 | 5 | 3 | 1 | 1 | 1 |
| Marking | 0 | 5 | 4 | 4 | 3 | 1 | 1 | 1 |
| Heading | 0 | 4 | 2 | 2 | 2 | 2 | 2 | 4 |
| Crossing | 0 | 1 | 4 | 1 | 2 | 3 | 5 | 2 |

**Mental (performance)**

| Attribute | GK | CB | FB | DM | CM | AM | W | ST |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Vision | 1 | 1 | 2 | 3 | 4 | 5 | 3 | 3 |
| Decisions | 4 | 4 | 3 | 4 | 4 | 4 | 3 | 3 |
| Def. Positioning | 4 | 5 | 4 | 4 | 3 | 1 | 1 | 1 |
| Off-the-ball | 0 | 1 | 2 | 2 | 3 | 4 | 4 | 5 |
| Composure | 3 | 3 | 3 | 3 | 3 | 4 | 3 | 5 |
| Concentration | 4 | 4 | 3 | 3 | 3 | 2 | 2 | 2 |
| Work Rate | 1 | 2 | 4 | 4 | 4 | 3 | 4 | 3 |
| Aggression | 1 | 4 | 3 | 4 | 3 | 1 | 2 | 2 |

**Physical**

| Attribute | GK | CB | FB | DM | CM | AM | W | ST |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Speed | 1 | 3 | 4 | 2 | 3 | 3 | 5 | 4 |
| Stamina | 1 | 2 | 4 | 4 | 5 | 3 | 4 | 3 |
| Strength | 2 | 4 | 2 | 3 | 3 | 2 | 2 | 4 |
| Agility | 4 | 2 | 3 | 2 | 3 | 4 | 4 | 3 |
| Jumping | 3 | 4 | 1 | 2 | 2 | 1 | 1 | 4 |

**Goalkeeping** (outfield roles = 0 for all four)

| Attribute | GK |
|---|:-:|
| Reflexes | 5 |
| Handling | 5 |
| Command of Area | 4 |
| Distribution | 4 |

**Role variants** are refinements on these archetypes, added later, not needed now: e.g.
ball-playing CB (bump Passing/Vision/Composure), poacher vs. target-man ST (Finishing/Off-the-ball
vs. Strength/Heading/link-play), deep-lying vs. advanced playmaker. The archetypes prove the
schema; variants are Phase 2 tactics territory.

---

## 6. Attribute → match-action coverage

Validates that every performance attribute feeds ≥1 match contest, and gives Phase 2 its input
map. Each contest lists contributing attributes by side; the **weighting/resolution math is
Phase 2**.

| # | Contest | Attacking side | Defending side |
|---|---|---|---|
| 1 | **Pass** (retain/progress) | Passing, Vision, Decisions, Composure, Ball Control | Def. Positioning, Marking, Decisions, Speed, Aggression, Work Rate |
| 2 | **Take-on / dribble** | Dribbling, Ball Control, Agility, Speed, Composure | Tackling, Marking, Def. Positioning, Speed, Agility, Strength |
| 3 | **Tackle / challenge** | Ball Control, Strength, Agility, Composure | Tackling, Marking, Aggression, Strength, Decisions, Composure |
| 4 | **Interception / read** | — | Def. Positioning, Decisions, Marking, Speed, Concentration |
| 5 | **Cross → box** | Crossing, Vision · then Heading, Jumping, Strength, Off-the-ball, Composure | Heading, Jumping, Marking, Def. Positioning, Strength |
| 6 | **Shot / conversion** | Finishing, Composure, Ball Control, Off-the-ball | GK: Reflexes, Handling, Def. Positioning, Agility, Command · Block: Def. Positioning, Aggression |
| 7 | **Aerial duel** (long ball) | Heading, Jumping, Strength | Heading, Jumping, Strength |
| 8 | **Foul & card** | — | ↑ Aggression · ↓ Composure, Decisions |
| 9 | **Injury** | — | *Injury-proneness (hidden)*; context (challenge intensity, Strength) |
| 10 | **Press / transition** (team) | Work Rate, Stamina, Speed, Aggression, Def. Positioning | (mirror) |
| 11 | **Fatigue** (over 90') | Stamina modulates all; Work Rate raises exertion | (mirror) |
| 12 | **GK distribution** | Distribution, Decisions, Composure | — |
| 13 | **Error / lapse** | Concentration (↓ by fatigue), Composure (under pressure); per-match floor set by *Consistency (hidden)* | (mirror) |

**No-orphan check.** All 25 performance attributes appear above. Character/hidden accounted for:
Consistency → #13, Injury-proneness → #9, Leadership → morale/captaincy system (not a contest),
Determination/Professionalism → development (not a match), PA → development ceiling. ✔

---

## 7. Development categories → age-curve shapes

The category tag on each performance attribute (§2) selects its age-curve family. **Curve
*parameters* are Phase 3** — **[resolved: filled in `DEVELOPMENT_MODEL.md` §2.1]**, one
`EnvParams` (maturation logistic minus aging logistic) per `DevCategory`, and since **re-fit against
real `worldgen`** by the career-arc harness (`DEVELOPMENT_MODEL.md` §6, the `b_beat`-style re-tune).
The qualitative commitment this section fixes, which the fitted numbers honour:

- **Physical** (Speed, Stamina, Strength, Agility, Jumping): rise early, **peak ~24–27**, then
  decline — steepest in Speed/Agility, gentler in Strength/Jumping.
- **Technical** (all Tech attributes): develop through the 20s, plateau, **very slow late decline**
  (Passing/Finishing hold well into the 30s).
- **Mental** (all performance Ment attributes): slow to develop, **keep rising into the early 30s**,
  minimal decline.
- **Goalkeeping**: technical/mental-shaped — GKs **peak later and age gracefully** (career length
  is a genuine GK trait; reflected in flatter curves).

This is *why* each attribute must be categorized in the schema rather than in Phase 3: the curve
family is a property of the attribute, fixed here; only the numbers wait.

---

## 8. Rust type sketch

Concrete anchor for P0.2. Not final — decisions it encodes: fixed 0–100 integers, dense
array-backed attributes (exact + hashable + cheap to fold), CA derived, character split out.

```rust
/// Fixed 0–100 integer scale for every rated value. Invariant: value ∈ 0..=100.
pub type Rating = u8;

/// Selects an attribute's age-curve family (§7).
pub enum DevCategory { Physical, Technical, Mental }

/// Performance attributes: contribute to CA (§4) and drive match actions (§6).
#[repr(u8)]
pub enum Attribute {
    // Technical
    Finishing, Passing, BallControl, Dribbling, Tackling, Marking, Heading, Crossing,
    // Mental (performance)
    Vision, Decisions, DefPositioning, OffTheBall, Composure, Concentration, WorkRate, Aggression,
    // Physical
    Speed, Stamina, Strength, Agility, Jumping,
    // Goalkeeping
    Reflexes, Handling, CommandOfArea, Distribution,
}
pub const NUM_ATTRIBUTES: usize = 25;

/// Dense, indexed by `Attribute as usize`. Exact serialization; trivial to fold over.
pub struct Attributes([Rating; NUM_ATTRIBUTES]);

/// Character / hidden: development, variance, and team-system drivers — NEVER in CA.
pub struct Character {
    pub potential: Rating,        // PA: hidden peak best-role CA (§4)
    pub determination: Rating,    // dev rate + big-match; persona seed (Phase 5)
    pub professionalism: Rating,  // training gain, aging/injury resistance; persona seed
    pub consistency: Rating,      // match-to-match variance (hidden)
    pub injury_proneness: Rating, // injury-event weight (hidden)
    pub leadership: Rating,       // morale / captaincy system modifier
}

pub enum Role { Gk, Cb, Fb, Dm, Cm, Am, W, St }

/// The design-once role→attribute importance table (0..=5), §5.
/// Consumed by: CA aggregation, valuation, match team-quality, transfer needs.
pub struct RoleWeights {
    // conceptually [Role][Attribute] -> u8 in 0..=5
}

/// CA is derived, never stored (§4): role-weighted mean over attributes, 0..=100.
pub fn current_ability(attrs: &Attributes, role: Role, w: &RoleWeights) -> Rating {
    // round( Σ wᵢ·aᵢ / Σ wᵢ )
    todo!()
}
```

---

## 9. Open sub-questions for P0.1

Genuinely unresolved within the schema (distinct from later-phase math):

1. **Ball-playing CB as archetype or variant?** Currently a variant (§5). If modern squad-building
   makes it first-class, promote it to its own role column.
2. **Does the Concentration (performance) vs. Consistency (hidden variance) split hold?** They're
   deliberately separate jobs — in-match error rate vs. match-to-match reliability — but if Phase 2
   can't make both knobs earn their keep, one may collapse into the other. **[resolved: the split
   holds — `MATCH_MODEL.md` §17.]** The 2e design gives each its own observable no other knob can
   reach — Consistency drives match-to-match rating volatility and the upset rate (one per-match
   draw scaling the day's effective attributes); Concentration drives fatigue-coupled late defensive
   lapses (late-goal share). Both consumed, collapse closed.
3. **Card rates from Aggression alone, or a separate hidden discipline factor?** (§3) — resolvable
   only once the foul/card contest is calibrated (Phase 2), but flagged here. **[resolved:
   Aggression alone in v1, with a pre-stated split tripwire — `MATCH_MODEL.md` §15.]** Lean-and-add
   taken; the hidden discipline factor is added only if calibration shows the per-player card tail
   can't be widened without distorting duel balance (Aggression being a performance input too — the
   conflation §15 names precisely).
4. **Best-role-peak-CA vs. attribute-budget** as the PA growth-gate (§4) — a Phase-3 call, noted so
   it isn't silently defaulted. **[resolved: best-role-peak-CA — `DEVELOPMENT_MODEL.md` §2.2.]**
