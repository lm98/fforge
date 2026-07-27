//! Substitution plans (`MATCH_MODEL.md` §16, `TACTICS_MODEL.md` §7's
//! pre-commitment pattern): a small, declarative condition→action rule set
//! the manager submits alongside the team sheet, evaluated deterministically
//! inside the engine at fixed decision points — never mid-match I/O.
//! `Lineup::sub_plan`'s empty `Vec` default is the identity: no rules, no
//! substitutions, no tactic changes, bit-identical to the pre-2e engine.

use crate::{Mentality, PlayerId, Pressing, Tempo, Width};
use serde::{Deserialize, Serialize};

/// A side's own scoreline standpoint at the moment a decision point is
/// evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScoreState {
    Trailing,
    Level,
    Leading,
}

/// One clause of a rule's condition; a rule fires only once every clause in
/// its `SubRule::conditions` holds (empty = always). RNG-free by
/// construction — every clause reads already-resolved match state (the
/// clock, the score, condition/injury/dismissal state), never draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubCondition {
    /// The match clock has reached at least this minute.
    MinuteAtLeast(u8),
    /// This side's own scoreline standpoint.
    Score(ScoreState),
    /// The named player's current in-match fatigue reads below this percent
    /// (0-100) — "condition-drained" in the decision-point sense (§2.6):
    /// `contest::fatigue_mult`'s live reading, not the pre-match §13 value,
    /// since that is fixed for the whole match and cannot itself drain
    /// further as play goes on.
    PlayerConditionBelow(PlayerId, u8),
    /// The named player has picked up an in-match injury (§14) — still on
    /// the pitch (injuries don't force a departure by themselves), but
    /// flagged for a covering substitution.
    PlayerInjured(PlayerId),
    /// This side is already down a player (a red card, §15) — one fewer
    /// than eleven on the pitch.
    ManDown,
}

/// One resolvable action a rule may take. `Substitute` is the only kind
/// that counts against the 3-substitution cap; a tactics change is
/// unlimited (re-applying the same instruction at a later decision point is
/// a harmless no-op) and is the in-match tactic-change seam
/// `TACTICS_MODEL.md` §7 named and left for here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAction {
    /// Bring `player_in` on for `player_out` — both named explicitly at
    /// plan-authoring time (never resolved by an in-match search), so
    /// evaluation stays draw-free and the plan really is the whole decision.
    Substitute {
        player_out: PlayerId,
        player_in: PlayerId,
    },
    SetMentality(Mentality),
    SetTempo(Tempo),
    SetWidth(Width),
    SetPressing(Pressing),
}

/// A condition→action rule (§2.7's vocabulary): all `conditions` must hold
/// for `action` to fire. Rules are evaluated in list order at every
/// decision point; a rule whose players are no longer eligible (already
/// subbed off, or the bench name has already entered) is a silent no-op,
/// not a validation error, since a plan is authored before kickoff and
/// match events beyond a manager's control decide which rules ever get the
/// chance to fire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubRule {
    pub conditions: Vec<SubCondition>,
    pub action: SubAction,
}
