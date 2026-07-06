//! CA / PA semantics (ATTRIBUTE_SCHEMA.md §4).
//!
//! CA is **derived, never stored**: a pure function of attributes and a role.
//! Attributes are the single source of truth; CA is a view — no sync bug is
//! possible by construction. PA lives in `Character` (stored, hidden).

use crate::attributes::{Attribute, Attributes, Rating};
use crate::role::{Role, RoleWeights};

/// `CA(player, role) = round( Σᵢ w[role][i]·attr[i] / Σᵢ w[role][i] )` ∈ 0..=100.
/// Integer arithmetic throughout (round-half-up) — exact in the deterministic fold.
pub fn current_ability(attrs: &Attributes, role: Role, w: &RoleWeights) -> Rating {
    let mut num: u32 = 0;
    let mut den: u32 = 0;
    for attr in Attribute::ALL {
        let wi = w.weight(role, attr) as u32;
        num += wi * attrs.get(attr) as u32;
        den += wi;
    }
    debug_assert!(den > 0, "every role weights at least one attribute");
    ((num + den / 2) / den) as Rating
}

/// The role in which this attribute set rates highest, and that rating.
/// A player's *headline* CA is CA in their assigned (or best) role.
/// Ties break in `Role::ALL` order — deterministic.
pub fn best_role(attrs: &Attributes, w: &RoleWeights) -> (Role, Rating) {
    let mut best = (Role::Gk, 0u8);
    for role in Role::ALL {
        let ca = current_ability(attrs, role, w);
        if ca > best.1 {
            best = (role, ca);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attributes::NUM_ATTRIBUTES;
    use crate::role::ROLE_WEIGHTS;

    #[test]
    fn uniform_attributes_give_that_ca_in_every_role() {
        let attrs = Attributes::new([70; NUM_ATTRIBUTES]);
        for role in Role::ALL {
            assert_eq!(current_ability(&attrs, role, &ROLE_WEIGHTS), 70);
        }
    }

    #[test]
    fn ca_is_position_relative() {
        // A pure-defender profile: high defensive attributes, low attacking.
        let mut attrs = Attributes::new([50; NUM_ATTRIBUTES]);
        for a in [
            Attribute::Tackling,
            Attribute::Marking,
            Attribute::DefPositioning,
            Attribute::Heading,
            Attribute::Strength,
        ] {
            attrs.set(a, 90);
        }
        for a in [Attribute::Finishing, Attribute::Dribbling, Attribute::OffTheBall] {
            attrs.set(a, 25);
        }
        let cb = current_ability(&attrs, Role::Cb, &ROLE_WEIGHTS);
        let st = current_ability(&attrs, Role::St, &ROLE_WEIGHTS);
        assert!(cb > st, "defender profile must rate higher at CB ({cb}) than ST ({st})");
        assert_eq!(best_role(&attrs, &ROLE_WEIGHTS).0, Role::Cb);
    }

    #[test]
    fn ca_stays_in_range() {
        let hi = Attributes::new([100; NUM_ATTRIBUTES]);
        let lo = Attributes::new([0; NUM_ATTRIBUTES]);
        for role in Role::ALL {
            assert_eq!(current_ability(&hi, role, &ROLE_WEIGHTS), 100);
            assert_eq!(current_ability(&lo, role, &ROLE_WEIGHTS), 0);
        }
    }
}