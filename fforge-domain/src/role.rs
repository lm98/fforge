//! The eight archetypal roles and the role→attribute weighting table
//! (ATTRIBUTE_SCHEMA.md §5) — the design-once artifact consumed by CA
//! aggregation, match team-quality, valuation, and the transfer AI.
//!
//! The numbers below are a **verbatim transcription** of the §5 tables.
//! Starting weights: design-time estimates, to be adjusted by Phase 2/3
//! calibration — adjust them in ATTRIBUTE_SCHEMA.md first, then here.

use crate::attributes::{Attribute, NUM_ATTRIBUTES};
use serde::{Deserialize, Serialize};

/// The eight archetypal roles spanning the pitch (§5). Role *variants*
/// (ball-playing CB, poacher vs target-man...) are Phase 2 territory.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Role {
    Gk,
    Cb,
    Fb,
    Dm,
    Cm,
    Am,
    W,
    St,
}

pub const NUM_ROLES: usize = 8;

impl Role {
    pub const ALL: [Role; NUM_ROLES] = [
        Role::Gk,
        Role::Cb,
        Role::Fb,
        Role::Dm,
        Role::Cm,
        Role::Am,
        Role::W,
        Role::St,
    ];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Gk => "Goalkeeper",
            Role::Cb => "Centre-Back",
            Role::Fb => "Full-Back",
            Role::Dm => "Defensive Mid",
            Role::Cm => "Central Mid",
            Role::Am => "Attacking Mid",
            Role::W => "Winger",
            Role::St => "Striker",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Role::Gk => "GK",
            Role::Cb => "CB",
            Role::Fb => "FB",
            Role::Dm => "DM",
            Role::Cm => "CM",
            Role::Am => "AM",
            Role::W => "W ",
            Role::St => "ST",
        }
    }
}

/// Importance weights 0..=5 (§5). Row = attribute (canonical order),
/// column = role in `Role::ALL` order: GK, CB, FB, DM, CM, AM, W, ST.
#[rustfmt::skip]
const WEIGHTS: [[u8; NUM_ROLES]; NUM_ATTRIBUTES] = [
    // Technical            GK CB FB DM CM AM  W ST
    /* Finishing       */ [ 0, 1, 1, 1, 2, 3, 3, 5],
    /* Passing         */ [ 1, 2, 3, 4, 4, 5, 3, 2],
    /* Ball Control    */ [ 1, 2, 3, 3, 4, 5, 4, 4],
    /* Dribbling       */ [ 0, 1, 2, 2, 3, 4, 5, 3],
    /* Tackling        */ [ 0, 5, 4, 5, 3, 1, 1, 1],
    /* Marking         */ [ 0, 5, 4, 4, 3, 1, 1, 1],
    /* Heading         */ [ 0, 4, 2, 2, 2, 2, 2, 4],
    /* Crossing        */ [ 0, 1, 4, 1, 2, 3, 5, 2],
    // Mental (performance)
    /* Vision          */ [ 1, 1, 2, 3, 4, 5, 3, 3],
    /* Decisions       */ [ 4, 4, 3, 4, 4, 4, 3, 3],
    /* Def. Positioning*/ [ 4, 5, 4, 4, 3, 1, 1, 1],
    /* Off-the-ball    */ [ 0, 1, 2, 2, 3, 4, 4, 5],
    /* Composure       */ [ 3, 3, 3, 3, 3, 4, 3, 5],
    /* Concentration   */ [ 4, 4, 3, 3, 3, 2, 2, 2],
    /* Work Rate       */ [ 1, 2, 4, 4, 4, 3, 4, 3],
    /* Aggression      */ [ 1, 4, 3, 4, 3, 1, 2, 2],
    // Physical
    /* Speed           */ [ 1, 3, 4, 2, 3, 3, 5, 4],
    /* Stamina         */ [ 1, 2, 4, 4, 5, 3, 4, 3],
    /* Strength        */ [ 2, 4, 2, 3, 3, 2, 2, 4],
    /* Agility         */ [ 4, 2, 3, 2, 3, 4, 4, 3],
    /* Jumping         */ [ 3, 4, 1, 2, 2, 1, 1, 4],
    // Goalkeeping (outfield roles = 0)
    /* Reflexes        */ [ 5, 0, 0, 0, 0, 0, 0, 0],
    /* Handling        */ [ 5, 0, 0, 0, 0, 0, 0, 0],
    /* Command of Area */ [ 4, 0, 0, 0, 0, 0, 0, 0],
    /* Distribution    */ [ 4, 0, 0, 0, 0, 0, 0, 0],
];

/// The design-once role→attribute importance table (§5, §8).
/// A zero-sized handle over the static table for now; becomes a loadable
/// value if calibration wants to tune weights at runtime (Phase 2/3).
#[derive(Debug, Clone, Copy, Default)]
pub struct RoleWeights;

pub const ROLE_WEIGHTS: RoleWeights = RoleWeights;

impl RoleWeights {
    #[inline]
    pub fn weight(&self, role: Role, attr: Attribute) -> u8 {
        WEIGHTS[attr.index()][role.index()]
    }
}