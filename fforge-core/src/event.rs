//! The event log — the game state *is* this append-only stream.
//!
//! Two principles from DESIGN.md §6 shape what gets recorded:
//!
//! 1. **Record resolved values, not raw inputs.** `GameStarted` carries the
//!    *generated world snapshot*, not just the seed: if only the seed were
//!    stored and the world re-derived on load, any improvement to worldgen
//!    would silently corrupt every old save — the same failure mode as
//!    re-parsing raw LLM text. Worldgen is an edge producer whose *output* is
//!    the recorded input.
//! 2. **Record outcomes the fold consumes without re-running engines.**
//!    `MatchPlayed` carries the result; replay folds over it and never
//!    re-simulates, so upgrading the match engine (Phase 2) can never rewrite
//!    history. Live play produces these events via `step`; replay just eats
//!    them. This is exactly how recorded agent `Decision`s will enter in
//!    Phase 5 — human lineups (`LineupSubmitted`) already follow the pattern.

use fforge_domain::{ClubId, Fixture, FixtureId, GameDate, Lineup, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Opening event: seed, generated world, schedule, and which club the
    /// human manages. Everything the fold needs, self-contained.
    GameStarted {
        seed: u64,
        start_date: GameDate,
        player_club: ClubId,
        world: World,
        schedule: Vec<Fixture>,
    },
    /// The human manager's resolved, validated team-sheet decision for the
    /// upcoming matchday.
    LineupSubmitted { matchday: u8, lineup: Lineup },
    /// A simulated result. (The rich minute-by-minute match event stream is
    /// the Phase 2 artifact; this scoreline is the walking-skeleton stub.)
    MatchPlayed {
        fixture: FixtureId,
        matchday: u8,
        home_goals: u8,
        away_goals: u8,
    },
    /// The calendar advanced past a matchday.
    MatchdayAdvanced { matchday: u8, new_date: GameDate },
    /// Season complete.
    SeasonEnded { champion: ClubId },
}