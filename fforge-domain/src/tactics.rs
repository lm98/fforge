//! Tactics (`TACTICS_MODEL.md` §2): the four-instruction surface a manager
//! submits alongside the team sheet, resolving into per-side effective knobs
//! (§3) at match time. `neutral()`/`Default` is the identity element every
//! 2e feature must have (`MATCH_MODEL.md` §11): every side playing it
//! reproduces today's Phase-2a engine bit-for-bit (§4's invariant).

use serde::{Deserialize, Serialize};

/// Risk posture: commit men forward for chance volume, at the price of
/// counter exposure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mentality {
    Defensive,
    Balanced,
    Attacking,
}

/// Progression style: many safe actions vs few risky ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tempo {
    Patient,
    Balanced,
    Direct,
}

/// Route mix: how much of the final-third entry goes through the wide zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Width {
    Narrow,
    Balanced,
    Wide,
}

/// Where you contest the opponent's possession: their build-up, or your own
/// block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pressing {
    Deep,
    Balanced,
    High,
}

/// The per-side tactical instruction set (§2). `Default` is `neutral()` —
/// load-bearing for serde back-compat (`Lineup.tactics`'s `#[serde(default)]`,
/// §6) and the §4 neutral-tactics invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tactics {
    pub mentality: Mentality,
    pub tempo: Tempo,
    pub width: Width,
    pub pressing: Pressing,
}

impl Tactics {
    pub const fn neutral() -> Self {
        Tactics {
            mentality: Mentality::Balanced,
            tempo: Tempo::Balanced,
            width: Width::Balanced,
            pressing: Pressing::Balanced,
        }
    }
}

impl Default for Tactics {
    fn default() -> Self {
        Self::neutral()
    }
}
