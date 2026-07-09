//! The five-zone possession state space (`MATCH_MODEL.md` §2) and the
//! role→zone presence table (§6): who is on the ball / defending where.
//!
//! Distinct from `fforge_domain::ROLE_WEIGHTS` (attribute *importance* for
//! CA) — this rates spatial *presence*, and drives actor/defender sampling
//! in the resolution model (§4). Verbatim from the calibrated
//! `match_model_prototype.ipynb` (`PRES_ATT` / `PRES_DEF`).

use fforge_domain::{NUM_ROLES, Role};

pub const NUM_ZONES: usize = 5;

/// `Def`/`Mid`/`AttC`/`AttW` are dwelling zones; `Box` is not dwelt in — an
/// edge that reaches it resolves a shot immediately (arrival = chance).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Def,
    Mid,
    AttC,
    AttW,
    Box,
}

impl Zone {
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// A short phrase for commentary rendering (the humble text match view).
    pub fn label(self) -> &'static str {
        match self {
            Zone::Def => "deep in their own third",
            Zone::Mid => "in midfield",
            Zone::AttC => "in the final third",
            Zone::AttW => "out wide",
            Zone::Box => "in the box",
        }
    }
}

/// Attacking presence: how often a role is the on-ball actor in a zone.
/// Row = `Role::ALL` order, column = zone (declared order).
#[rustfmt::skip]
const PRES_ATT: [[u8; NUM_ZONES]; NUM_ROLES] = [
    // Def Mid AttC AttW Box
    [5,  0,  0,   0,   0], // Gk — starts build-up
    [4,  1,  0,   0,   0], // Cb
    [3,  3,  1,   3,   0], // Fb — overlaps wide
    [3,  4,  1,   0,   0], // Dm
    [1,  4,  3,   1,   1], // Cm
    [0,  3,  4,   2,   2], // Am — central creation
    [0,  2,  2,   5,   2], // W  — owns the wide zone
    [0,  1,  3,   1,   5], // St — owns the box
];

/// Defensive presence: the primary challenger when the opponent attacks a
/// zone. Row = `Role::ALL` order, column = zone (declared order).
#[rustfmt::skip]
const PRES_DEF: [[u8; NUM_ZONES]; NUM_ROLES] = [
    // Def Mid AttC AttW Box
    [0,  0,  0,   0,   3], // Gk — contests crosses/shots in the box
    [1,  1,  4,   2,   5], // Cb — anchors central + box
    [1,  2,  2,   5,   3], // Fb — primary wide defender
    [2,  4,  3,   1,   1], // Dm
    [2,  4,  2,   1,   0], // Cm
    [2,  2,  1,   1,   0], // Am
    [3,  2,  1,   2,   0], // W  — tracks back a little
    [4,  1,  0,   0,   0], // St — presses deep build-up
];

pub fn attacking_presence(role: Role, zone: Zone) -> u32 {
    PRES_ATT[role.index()][zone.index()] as u32
}

pub fn defending_presence(role: Role, zone: Zone) -> u32 {
    PRES_DEF[role.index()][zone.index()] as u32
}
