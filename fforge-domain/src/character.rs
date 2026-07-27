//! Character / hidden attributes (ATTRIBUTE_SCHEMA.md §2): development,
//! variance, and team-system drivers — **never** contributing to CA.

use crate::attributes::Rating;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Character {
    /// PA: hidden peak best-role CA (§4). Development ceiling; Phase 3 growth
    /// mechanics gate on it.
    pub potential: Rating,
    /// Development rate + big-match modifier; persona seed (Phase 5).
    pub determination: Rating,
    /// Training gain, aging/injury resistance; persona seed (Phase 5).
    pub professionalism: Rating,
    /// Match-to-match variance — how reliably a player hits their CA (hidden).
    pub consistency: Rating,
    /// Weighting on injury events (hidden).
    pub injury_proneness: Rating,
    /// Between-match condition recovery rate (hidden). Split out at Phase 2e
    /// once recovery modeling gave it a genuine second consumer distinct from
    /// Professionalism (`MATCH_MODEL.md` §13, R8; `ATTRIBUTE_SCHEMA.md` §3).
    /// `#[serde(default)]` so pre-2e logs still deserialize.
    #[serde(default = "default_natural_fitness")]
    pub natural_fitness: Rating,
    /// Morale propagation / captaincy — a system modifier, not a match action.
    pub leadership: Rating,
}

/// Serde default for `natural_fitness` on logs recorded before Phase 2e: the
/// midpoint of the worldgen range, not a value that biases old saves' recovery
/// either way.
fn default_natural_fitness() -> Rating {
    50
}