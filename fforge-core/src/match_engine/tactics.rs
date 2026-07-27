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
    /// Tempo → added into the pass-specific bias (`Knobs::b_pass`), keyed by
    /// the passing side's own zone (T7 addendum §5/Fix B — structural, not a
    /// magnitude change: a deep giveaway costs more under §3's turnover
    /// mirroring than a giveaway in midfield, since a turnover won in this
    /// side's own `Def` mirrors to the opponent's `AttC`, while one won in
    /// `Mid` mirrors back to `Mid`. A uniform delta priced every zone's risk
    /// the same and let Direct discount its risk into the cheap zone).
    pub(super) b_pass_delta_by_zone: [f64; NUM_ZONES],
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
    /// **Conceded space.** When this side defends, the *opponent's*
    /// `p_mid_advance` gets this multiplier. Two tenants: Pressing `High`'s
    /// beaten-press term (the space behind a committed press once an opponent
    /// escapes it) and, since T7-R2, Mentality's own commitment term — men
    /// upfield are men not between the ball and the goal. They stack, which
    /// is the intended reading: a high-pressing `Attacking` side concedes on
    /// both counts.
    pub(super) opp_mid_advance_mult: f64,
    /// **Conceded penetration.** When this side defends, the *opponent's*
    /// `p_attc_penetrate` gets this multiplier — Pressing `Deep`'s
    /// compact-block term (no space behind a settled block) and, since
    /// T7-R2, Mentality's, on the same logic as the field above.
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
            b_pass_delta_by_zone: [0.0; NUM_ZONES],
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
            // T7-R2 (§9 item 7): the space a committed side concedes. Before
            // this, Mentality's gains were two advance-class multipliers and
            // its only cost was the logit-class `def_bias` above — ~4× weaker
            // (§3's lever-class note), so `Attacking` was not a risk setting
            // at all, just a better one (it beat `Balanced` 0.540/0.460).
            // §5's stated mechanism for the opponent's compensation was
            // turnover mirroring, but mirroring sends a ball lost high to a
            // *deep* opponent restart — which protects the attacking side
            // rather than punishing it, leaving the risk half of the risk
            // axis unmodelled. This is that half, in the same shape
            // `Pressing::High`'s beaten-press term already uses.
            e.opp_mid_advance_mult *= 1.25;
            e.opp_penetrate_mult *= 1.25;
        }
        Mentality::Defensive => {
            e.advance_mult *= 0.83;
            e.penetrate_mult *= 0.83;
            e.atk_bias -= 0.08;
            for b in &mut e.def_bias_by_zone {
                *b += 0.08;
            }
            // The mirror: a settled side denies the space `Attacking` sells,
            // the same shape `Pressing::Deep`'s compact-block term uses.
            // Not the reciprocal of Attacking's 1.25 — each side of the axis
            // is fitted against `Balanced` on its own, since what it is
            // paying for (Defensive surrenders its own penetration, which is
            // the scarce `p_attc_penetrate = 0.08` gateway) is not the
            // mirror-image quantity of what Attacking is buying.
            e.opp_mid_advance_mult *= 0.79;
            e.opp_penetrate_mult *= 0.79;
        }
        Mentality::Balanced => {}
    }

    match t.tempo {
        Tempo::Direct => {
            // T7-R fit: 1.30 → 1.13. `advance_mult` is an *advance-class*
            // lever — a multiplier on a raw transition probability — and at
            // `p_mid_advance = 0.20` a ×1.30 buys +6.0pp of forward progress
            // per Mid beat, against the −1.7pp the paired `b_pass_delta`
            // −0.10 costs at `b_pass = 1.35` (p ≈ 0.794, dp = p(1−p)·Δb).
            // The two levers were plausibility-picked independently (§9 item
            // 1) and never reconciled, so Direct bought progress at roughly a
            // quarter of its intended price and won 32/32 profile×world cells
            // of the T7-R probe. Fitted so Tempo is net-neutral against
            // `neutral()` on a control squad, leaving the *shape* effects
            // below (long-shot mix, take-on rate) to carry Direct's identity.
            e.advance_mult *= 1.13;
            e.w_longshot_mult *= 1.5;
            e.w_takeon_mult *= 1.1;
            // T7 addendum §5/Fix B: zone-profiled, not uniform. Direct's
            // ×1.30 advance_mult already means it spends less time in its
            // own (expensive-to-lose) Def zone and more in the cheap Mid
            // zone — a uniform −0.15 didn't price that shift, so Direct was
            // taking a discount rather than the risk §5 credited it with.
            e.b_pass_delta_by_zone[Zone::Def.index()] -= 0.25;
            e.b_pass_delta_by_zone[Zone::Mid.index()] -= 0.10;
            e.b_pass_delta_by_zone[Zone::AttC.index()] -= 0.10;
            e.b_pass_delta_by_zone[Zone::AttW.index()] -= 0.10;
        }
        Tempo::Patient => {
            // T7-R fit: 0.80 → 0.88, the mirror of Direct's correction and
            // for the same reason — ×0.80 *surrendered* 4.0pp of forward
            // progress per Mid beat to buy back only +1.6pp of pass
            // retention, making Patient strictly worse than doing nothing on
            // every squad shape measured. Not reciprocal to Direct's 1.13 by
            // construction: each side of the axis is fitted against
            // `neutral()` on its own, since the two levers' costs are not
            // symmetric functions of the same probability.
            e.advance_mult *= 0.88;
            e.w_longshot_mult *= 0.6;
            for b in &mut e.b_pass_delta_by_zone {
                *b += 0.10;
            }
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
            // T7 addendum §4/Fix A: concentrated where a turnover mirrors
            // expensively (§3's own mirroring table) instead of flat across
            // Def/Mid — a turnover won in the opponent's Def mirrors to this
            // side's AttC (expensive for them); won in Mid, it mirrors to
            // Mid (nearly free). A flat +0.15 spent equal pressure on
            // contests of unequal value, and — measured against the fixed
            // fatigue cost below, plus Patient's own +0.10 b_pass_delta
            // sitting in the same zones with the opposite sign — netted to
            // a wash (the T7 triangle's flat 0.502 read against Patient).
            // Closes ATTRIBUTE_SCHEMA.md-adjacent TACTICS_MODEL.md §9 item 3
            // ("texture question... same seam, finer key") in favour of
            // differentiating.
            e.def_bias_by_zone[Zone::Def.index()] += 0.25;
            e.def_bias_by_zone[Zone::Mid.index()] += 0.10;
            // Deliberately *not* re-fitted by T7-R: this is the term that
            // makes Pressing squad-conditional at all. `contest::fatigue_mult`
            // scales its drop by `(1 - stamina)`, so a ×1.30 exertion cost is
            // strictly cheaper for a high-Stamina squad while the
            // `def_bias_by_zone` benefit above is attribute-independent — the
            // measured +1.90pt physical-minus-technical press gradient (§5's
            // T7-R finding, ~11σ over 8 worlds) is this term's doing.
            // Flattening it to balance the press would have deleted the very
            // effect §9 item 6 was asking about.
            e.fatigue_mult *= 1.30;
            // T7-R fit: 1.15 → 1.02. The beaten-press term is the *same*
            // advance-class lever as Tempo's, mis-scaled the same way: ×1.15
            // handed the opponent +3.0pp of Mid advance per beat, which at
            // the slope measured for Direct (~0.078 of expected-points share
            // per unit of advance multiplier) cost the pressing side ~1.2pts
            // on its own — more than the whole `def_bias_by_zone` benefit
            // T7's Fix A was doubling in an attempt to find. That is why Fix
            // A moved nothing: it was topping up a logit-class benefit
            // against an advance-class cost four times its size. Shrunk, not
            // removed — the space behind a committed press is a real
            // mechanism (§5), it was simply priced on the wrong scale.
            e.opp_mid_advance_mult *= 1.02;
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
            assert_eq!(e.b_pass_delta_by_zone, [0.0; NUM_ZONES]);
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
        // Asserted against the two single-instruction resolutions rather than
        // against literal magnitudes, so the T7-R re-fit (and any later one)
        // moves the numbers without touching this invariant — the property
        // under test is *stacking*, not the values being stacked.
        let attacking = Tactics {
            mentality: Mentality::Attacking,
            ..Tactics::neutral()
        };
        let direct = Tactics {
            tempo: Tempo::Direct,
            ..Tactics::neutral()
        };
        let both = Tactics {
            mentality: Mentality::Attacking,
            tempo: Tempo::Direct,
            ..Tactics::neutral()
        };
        let [a, _] = resolve_tactics(attacking, Tactics::neutral());
        let [d, _] = resolve_tactics(direct, Tactics::neutral());
        let [ad, _] = resolve_tactics(both, Tactics::neutral());
        assert!((ad.advance_mult - a.advance_mult * d.advance_mult).abs() < 1e-12);
        // And both levers really are engaged — a stacking test against two
        // identity multipliers would pass vacuously.
        assert!(a.advance_mult > 1.0 && d.advance_mult > 1.0);
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
