//! Attribute → match-action contest maps (`ATTRIBUTE_SCHEMA.md` §6), the
//! shared logistic resolver, fatigue scaling, and the team-quality support
//! term (`MATCH_MODEL.md` §4). Weight vectors are verbatim from the
//! notebook's `CONTEST` dict.

use super::knobs::Knobs;
use fforge_domain::{Attribute, Attributes};

/// A contest's weighted-mean attribute score in 0..100 — mirrors the
/// notebook's `wa()`. `weights` need not be pre-normalized.
pub fn score(attrs: &Attributes, weights: &[(Attribute, f64)]) -> f64 {
    let total: f64 = weights.iter().map(|(_, w)| w).sum();
    weights
        .iter()
        .map(|&(a, w)| attrs.get(a) as f64 * w)
        .sum::<f64>()
        / total
}

pub const PASS_ATK: &[(Attribute, f64)] = &[
    (Attribute::Passing, 4.0),
    (Attribute::Vision, 1.5),
    (Attribute::Decisions, 1.5),
    (Attribute::BallControl, 2.0),
    (Attribute::Composure, 1.0),
];
pub const PASS_DEF: &[(Attribute, f64)] = &[
    (Attribute::DefPositioning, 2.5),
    (Attribute::Marking, 2.0),
    (Attribute::Decisions, 1.5),
    (Attribute::Speed, 1.0),
    (Attribute::Aggression, 1.0),
    (Attribute::WorkRate, 2.0),
];
pub const TAKEON_ATK: &[(Attribute, f64)] = &[
    (Attribute::Dribbling, 4.0),
    (Attribute::BallControl, 2.0),
    (Attribute::Agility, 1.5),
    (Attribute::Speed, 1.5),
    (Attribute::Composure, 1.0),
];
pub const TAKEON_DEF: &[(Attribute, f64)] = &[
    (Attribute::Tackling, 3.0),
    (Attribute::Marking, 2.0),
    (Attribute::DefPositioning, 1.5),
    (Attribute::Speed, 1.5),
    (Attribute::Agility, 1.0),
    (Attribute::Strength, 1.0),
];
pub const CROSS_ATK: &[(Attribute, f64)] = &[
    (Attribute::Crossing, 6.0),
    (Attribute::Vision, 2.5),
    (Attribute::Composure, 1.5),
];
pub const CROSS_DEF: &[(Attribute, f64)] = &[
    (Attribute::DefPositioning, 5.0),
    (Attribute::Marking, 3.0),
    (Attribute::Speed, 2.0),
];
pub const FINISH_ATK: &[(Attribute, f64)] = &[
    (Attribute::Finishing, 4.0),
    (Attribute::Composure, 2.0),
    (Attribute::BallControl, 1.0),
    (Attribute::OffTheBall, 1.5),
];
pub const HEADER_ATK: &[(Attribute, f64)] = &[
    (Attribute::Heading, 4.0),
    (Attribute::Jumping, 2.5),
    (Attribute::Strength, 1.5),
    (Attribute::Composure, 1.5),
    (Attribute::OffTheBall, 1.0),
];
pub const BLOCK_DEF: &[(Attribute, f64)] = &[
    (Attribute::DefPositioning, 3.0),
    (Attribute::Aggression, 1.0),
];
pub const AERIAL_DEF: &[(Attribute, f64)] = &[
    (Attribute::Heading, 3.0),
    (Attribute::Jumping, 2.0),
    (Attribute::Marking, 2.0),
    (Attribute::Strength, 1.5),
    (Attribute::DefPositioning, 1.5),
];
pub const GK_SHOT: &[(Attribute, f64)] = &[
    (Attribute::Reflexes, 4.0),
    (Attribute::Handling, 2.5),
    (Attribute::DefPositioning, 1.5),
    (Attribute::Agility, 1.5),
    (Attribute::CommandOfArea, 1.0),
];
pub const GK_AERIAL: &[(Attribute, f64)] = &[
    (Attribute::CommandOfArea, 4.0),
    (Attribute::Handling, 2.0),
    (Attribute::Reflexes, 2.0),
    (Attribute::DefPositioning, 1.5),
];

pub fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Effective-attribute multiplier from fatigue (`MATCH_MODEL.md` §4): grows
/// with match minute, larger for low-Stamina, high-Work-Rate players — they
/// fade late. `press_mult` is `TACTICS_MODEL.md` §3's Pressing exertion cost
/// (identity `1.0`): a side's own tactical press intensity, applied to every
/// player on that side regardless of whether they're the actor or the
/// defender in a given contest.
pub fn fatigue_mult(attrs: &Attributes, minute: f64, k: &Knobs, press_mult: f64) -> f64 {
    let stamina = attrs.get(Attribute::Stamina) as f64 / 100.0;
    let work_rate = attrs.get(Attribute::WorkRate) as f64 / 100.0;
    let drop = k.fatigue_base
        * press_mult
        * (minute / 90.0)
        * (1.0 - stamina)
        * (1.0 + k.fatigue_wr * work_rate);
    (1.0 - drop).clamp(0.7, 1.0)
}

/// The one logistic-of-difference shape shared by every open-play contest
/// (`MATCH_MODEL.md` §4): `p = sigmoid(k*(atk-def)/s + bias + home_bias·[home attacking])`.
pub fn contest_p(atk: f64, def: f64, bias: f64, k: &Knobs, home_attacking: bool) -> f64 {
    let hb = if home_attacking { k.home_bias } else { 0.0 };
    sigmoid(k.k * (atk - def) / k.s + bias + hb)
}

/// Blend an actor's contest score with the team mean — the cheap
/// "interaction effects" support term (`MATCH_MODEL.md` §4). Small by
/// default: the actor dominates, team quality nudges.
pub fn blend(actor_score: f64, team_mean: f64, k: &Knobs) -> f64 {
    (1.0 - k.support) * actor_score + k.support * team_mean
}

#[cfg(test)]
mod tests {
    use super::*;
    use fforge_domain::{Attributes, NUM_ATTRIBUTES};

    #[test]
    fn fatigue_bounds_hold_at_extremes() {
        let k = Knobs::default();
        let iron_man = Attributes::new([100; NUM_ATTRIBUTES]); // max stamina, max work rate
        let liability = Attributes::new([0; NUM_ATTRIBUTES]); // min stamina, min work rate (no drop either)
        // Full stamina: no drop regardless of minute.
        assert_eq!(fatigue_mult(&iron_man, 90.0, &k, 1.0), 1.0);
        // Zero stamina but also zero work rate at kickoff: no drop yet (minute=0).
        assert_eq!(fatigue_mult(&liability, 0.0, &k, 1.0), 1.0);
        // Multiplier never leaves the sanctioned band.
        for minute in [0.0, 30.0, 45.0, 90.0] {
            let m = fatigue_mult(&liability, minute, &k, 1.0);
            assert!(
                (0.7..=1.0).contains(&m),
                "fatigue multiplier {m} out of band at minute {minute}"
            );
        }
        // A higher press_mult (Pressing High, TACTICS_MODEL.md §3) deepens
        // the drop at the same minute; the identity value must reproduce the
        // 3-arg-equivalent baseline exactly (bit-for-bit, §4).
        let boosted = fatigue_mult(&liability, 45.0, &k, 1.30);
        let baseline = fatigue_mult(&liability, 45.0, &k, 1.0);
        assert!(boosted <= baseline);
    }

    #[test]
    fn contest_p_stays_in_unit_interval_at_extreme_gaps() {
        let k = Knobs::default();
        for (atk, def) in [(0.0, 100.0), (100.0, 0.0), (50.0, 50.0)] {
            for home in [true, false] {
                let p = contest_p(atk, def, k.b_pass, &k, home);
                assert!(
                    (0.0..=1.0).contains(&p),
                    "p={p} out of [0,1] for atk={atk} def={def} home={home}"
                );
            }
        }
    }

    #[test]
    fn home_bias_only_favors_the_home_attacking_side() {
        let k = Knobs::default();
        let p_home = contest_p(50.0, 50.0, k.b_pass, &k, true);
        let p_away = contest_p(50.0, 50.0, k.b_pass, &k, false);
        assert!(
            p_home > p_away,
            "home_bias must strictly favor the home side's attacking contests"
        );
    }

    #[test]
    fn blend_is_identity_at_the_endpoints() {
        let k = Knobs {
            support: 0.0,
            ..Knobs::default()
        };
        assert_eq!(blend(80.0, 20.0, &k), 80.0);
        let k = Knobs {
            support: 1.0,
            ..Knobs::default()
        };
        assert_eq!(blend(80.0, 20.0, &k), 20.0);
    }
}
