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
   can't make both knobs earn their keep, one may collapse into the other.
3. **Card rates from Aggression alone, or a separate hidden discipline factor?** (§3) — resolvable
   only once the foul/card contest is calibrated (Phase 2), but flagged here.
4. **Best-role-peak-CA vs. attribute-budget** as the PA growth-gate (§4) — a Phase-3 call, noted so
   it isn't silently defaulted.