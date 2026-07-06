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
    /// Morale propagation / captaincy — a system modifier, not a match action.
    pub leadership: Rating,
}