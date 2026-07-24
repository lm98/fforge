//! Per-side tactics resolution (`TACTICS_MODEL.md` §3): a pure, RNG-free
//! function from a side's own `Tactics` to a per-side effect table,
//! evaluated once per match — the same "resolve once, apply many" shape
//! `team_means` already established for `p_wide`. Consuming zero draws is
//! what makes the §4 neutral-tactics invariant hold *by construction*: every
//! multiplier is `1.0` and every bias is `0.0` at `neutral()`, and IEEE-754
//! makes `p * 1.0` / `x + 0.0` exact, so the draw sequence is untouched.
//!
//! Every effect is one of exactly three deformation types (§3): a multiplier
//! on an existing transition/selection probability, an additive term in the
//! existing logistic bias slot, or a multiplier on the fatigue rate. No new
//! contest types, no new zones, no presence-table edits.

use super::zone::{NUM_ZONES, Zone};
use fforge_domain::{Mentality, Pressing, Tactics, Tempo, Width};

/// Per-side effective view, resolved once per match from this side's own
/// `Tactics` (§3). Pure and RNG-free.
///
/// Divergence from the doc's `action_w_mult: [f64; N]` sketch: named fields
/// (`w_longshot_mult`/`w_takeon_mult`/`w_cross_mult`) instead of a generic
/// array — the doc itself calls its pseudocode a starting point, not a
/// commitment (§8), and named fields read directly against §3's effect
/// table without a second index-to-weight mapping to keep in sync.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SideEffects {
    /// Width → stacks on `formation_p_wide`.
    pub(super) p_wide_mult: f64,
    /// Tempo/Mentality → `p_def_advance`, `p_mid_advance` (this side's own
    /// advance probability).
    pub(super) advance_mult: f64,
    /// Mentality → `p_attc_penetrate`, `p_attc_dribble_box` (this side's own
    /// penetration probability).
    pub(super) penetrate_mult: f64,
    /// Tempo → `w_longshot_attc`.
    pub(super) w_longshot_mult: f64,
    /// Tempo → `w_takeon_{mid,attc,attw}`.
    pub(super) w_takeon_mult: f64,
    /// Width → `w_cross_attw`.
    pub(super) w_cross_mult: f64,
    /// Tempo → added into the pass-specific bias (`Knobs::b_pass`).
    pub(super) b_pass_delta: f64,
    /// Mentality → added into `contest_p`'s bias when this side attacks.
    pub(super) atk_bias: f64,
    /// Pressing/Mentality → added, negated, into the *opponent's* attacking
    /// bias when this side defends, keyed by the possessing (opponent's)
    /// zone. Own-side bookkeeping: a positive entry means *this side's*
    /// defence is better in that zone, so it subtracts from the attacker's
    /// success probability once negated at the point of use.
    pub(super) def_bias_by_zone: [f64; NUM_ZONES],
    /// Pressing → scales `Knobs::fatigue_base` for every player on this
    /// side, actor or defender alike (an exertion cost of the press, not a
    /// contest-specific term).
    pub(super) fatigue_mult: f64,
    /// Pressing `High`'s beaten-press term: when this side defends, the
    /// *opponent's* `p_mid_advance` gets this multiplier — the space behind
    /// a committed press once an opponent escapes it.
    pub(super) opp_mid_advance_mult: f64,
    /// Pressing `Deep`'s compact-block term: when this side defends, the
    /// *opponent's* `p_attc_penetrate` gets this multiplier — no space
    /// behind a settled block.
    pub(super) opp_penetrate_mult: f64,
}

impl SideEffects {
    /// The identity element (§2.1, §4): every multiplier `1.0`, every bias
    /// `0.0`, exactly — not approximately.
    pub(super) const fn identity() -> Self {
        SideEffects {
            p_wide_mult: 1.0,
            advance_mult: 1.0,
            penetrate_mult: 1.0,
            w_longshot_mult: 1.0,
            w_takeon_mult: 1.0,
            w_cross_mult: 1.0,
            b_pass_delta: 0.0,
            atk_bias: 0.0,
            def_bias_by_zone: [0.0; NUM_ZONES],
            fatigue_mult: 1.0,
            opp_mid_advance_mult: 1.0,
            opp_penetrate_mult: 1.0,
        }
    }
}

/// Resolve one side's own `Tactics` into its `SideEffects` (§3's effect
/// table). Where Mentality and Tempo both touch `advance_mult`, the
/// multipliers stack — independent levers on the same probability, the
/// `formation_p_wide` × `p_wide_mult` precedent.
fn resolve_side_effects(t: Tactics) -> SideEffects {
    let mut e = SideEffects::identity();

    match t.mentality {
        Mentality::Attacking => {
            e.advance_mult *= 1.20;
            e.penetrate_mult *= 1.20;
            e.atk_bias += 0.08;
            for b in &mut e.def_bias_by_zone {
                *b -= 0.08;
            }
        }
        Mentality::Defensive => {
            e.advance_mult *= 0.83;
            e.penetrate_mult *= 0.83;
            e.atk_bias -= 0.08;
            for b in &mut e.def_bias_by_zone {
                *b += 0.08;
            }
        }
        Mentality::Balanced => {}
    }

    match t.tempo {
        Tempo::Direct => {
            e.advance_mult *= 1.30;
            e.w_longshot_mult *= 1.5;
            e.w_takeon_mult *= 1.1;
            e.b_pass_delta -= 0.15;
        }
        Tempo::Patient => {
            e.advance_mult *= 0.80;
            e.w_longshot_mult *= 0.6;
            e.b_pass_delta += 0.10;
        }
        Tempo::Balanced => {}
    }

    match t.width {
        Width::Wide => {
            e.p_wide_mult *= 1.35;
            e.w_cross_mult *= 1.2;
        }
        Width::Narrow => {
            e.p_wide_mult *= 0.70;
            e.w_cross_mult *= 0.85;
        }
        Width::Balanced => {}
    }

    match t.pressing {
        Pressing::High => {
            e.def_bias_by_zone[Zone::Def.index()] += 0.15;
            e.def_bias_by_zone[Zone::Mid.index()] += 0.15;
            e.fatigue_mult *= 1.30;
            e.opp_mid_advance_mult *= 1.15;
        }
        Pressing::Deep => {
            e.def_bias_by_zone[Zone::Def.index()] -= 0.10;
            e.def_bias_by_zone[Zone::Mid.index()] -= 0.10;
            e.def_bias_by_zone[Zone::AttC.index()] += 0.10;
            e.def_bias_by_zone[Zone::AttW.index()] += 0.10;
            e.def_bias_by_zone[Zone::Box.index()] += 0.10;
            e.opp_penetrate_mult *= 0.85;
        }
        Pressing::Balanced => {}
    }

    e
}

/// Resolve both sides at once — the `(Tactics, Tactics) -> (SideEffects,
/// SideEffects)` shape §3 names. Each side's effects depend only on its own
/// `Tactics`; the cross-side interaction (§5's structural rock-paper-
/// scissors) emerges from how these one-sided effects play out during
/// resolution, never from consulting the opponent's tactics here — that is
/// the "no matchup table" commitment.
pub(super) fn resolve_tactics(home: Tactics, away: Tactics) -> [SideEffects; 2] {
    [resolve_side_effects(home), resolve_side_effects(away)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tactics_neutral_is_the_exact_identity() {
        // TACTICS_MODEL.md §4: every field of SideEffects for
        // (neutral, neutral) equals its identity constant exactly — `== 1.0`
        // / `== 0.0`, not approximately. Movement here at the neutral
        // setting is a wiring bug, never a re-tune.
        let [home, away] = resolve_tactics(Tactics::neutral(), Tactics::neutral());
        for e in [home, away] {
            assert_eq!(e.p_wide_mult, 1.0);
            assert_eq!(e.advance_mult, 1.0);
            assert_eq!(e.penetrate_mult, 1.0);
            assert_eq!(e.w_longshot_mult, 1.0);
            assert_eq!(e.w_takeon_mult, 1.0);
            assert_eq!(e.w_cross_mult, 1.0);
            assert_eq!(e.b_pass_delta, 0.0);
            assert_eq!(e.atk_bias, 0.0);
            assert_eq!(e.def_bias_by_zone, [0.0; NUM_ZONES]);
            assert_eq!(e.fatigue_mult, 1.0);
            assert_eq!(e.opp_mid_advance_mult, 1.0);
            assert_eq!(e.opp_penetrate_mult, 1.0);
        }
    }

    #[test]
    fn mentality_and_tempo_advance_mult_stack() {
        // §3: independent levers on the same probability multiply together.
        let t = Tactics {
            mentality: Mentality::Attacking,
            tempo: Tempo::Direct,
            ..Tactics::neutral()
        };
        let [e, _] = resolve_tactics(t, Tactics::neutral());
        assert!((e.advance_mult - 1.20 * 1.30).abs() < 1e-12);
    }

    #[test]
    fn mentality_defensive_mirrors_attacking_exactly() {
        let attacking = Tactics {
            mentality: Mentality::Attacking,
            ..Tactics::neutral()
        };
        let defensive = Tactics {
            mentality: Mentality::Defensive,
            ..Tactics::neutral()
        };
        let [atk, _] = resolve_tactics(attacking, Tactics::neutral());
        let [def, _] = resolve_tactics(defensive, Tactics::neutral());
        assert!((atk.advance_mult - 1.20).abs() < 1e-12);
        assert!((def.advance_mult - 0.83).abs() < 1e-12);
        assert_eq!(atk.atk_bias, -def.atk_bias);
        for z in 0..NUM_ZONES {
            assert_eq!(atk.def_bias_by_zone[z], -def.def_bias_by_zone[z]);
        }
    }
}
