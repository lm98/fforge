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
//!    `DevelopmentTick` (Phase 3, `DEVELOPMENT_MODEL.md` §5) is the newest
//!    application of the same rule: it records the resolved sparse attribute
//!    deltas, *not* the seed, so the growth model can evolve without rewriting a
//!    recorded career — the fold only integer-adds the deltas.

use crate::club_ai::TransferDecision;
use crate::match_engine::{CardOutcome, InjuryOutcome};
use fforge_domain::{
    Attribute, ClubId, Contract, Fixture, FixtureId, GameDate, Lineup, Money, Player, PlayerId,
    World,
};
use serde::{Deserialize, Serialize};

/// One resolved attribute step in a `DevelopmentTick` (`DEVELOPMENT_MODEL.md`
/// §5): a `delta` (usually ±1, occasionally more for fast youth growth) applied
/// to one player's one attribute. The fold adds it, clamped to 0..=100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrStep {
    pub player: PlayerId,
    pub attr: Attribute,
    pub delta: i8,
}

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
    /// A simulated result. Carries the two participating XIs (`home_xi` /
    /// `away_xi`) as the *resolved outcome* the fold consumes: appearances feed
    /// the Phase-3 playing-time development input (`DEVELOPMENT_MODEL.md` §3),
    /// recorded here rather than re-derived at tick time — a past matchday's
    /// effective lineup depends on transient `pending_lineup` state that is not
    /// otherwise reconstructable, so recording it is the replay-safe source.
    /// (The rich minute-by-minute match event stream stays a Trace, not a fold
    /// input, `MATCH_MODEL.md` §7.)
    ///
    /// The Phase-2e boundary extension (`MATCH_MODEL.md` §12) adds the
    /// resolved per-player consequences that outlive the match: `injuries`
    /// (the days out, never a severity to re-roll), `cards` (the card itself,
    /// never a foul to re-resolve), `ratings` (tenths; recorded because
    /// the stream they derive from is not persisted for bulk AI matches),
    /// and `minutes` (true minutes played, substitutions included — T4 of
    /// the batch handoff, §16/R7's playing-time input). Suspensions are
    /// deliberately *absent*: a ban is derived in the fold from accumulated
    /// cards — recording both would create two sources of truth that can
    /// disagree, the sync bug the CA-is-derived rule exists to make
    /// impossible. All four default to empty so pre-2e logs (and logs
    /// written before the corresponding models land) replay unchanged.
    MatchPlayed {
        fixture: FixtureId,
        matchday: u8,
        home_goals: u8,
        away_goals: u8,
        home_xi: Vec<PlayerId>,
        away_xi: Vec<PlayerId>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        injuries: Vec<InjuryOutcome>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cards: Vec<CardOutcome>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ratings: Vec<(PlayerId, u8)>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        minutes: Vec<(PlayerId, u8)>,
    },
    /// The calendar advanced past a matchday.
    MatchdayAdvanced { matchday: u8, new_date: GameDate },
    /// A monthly player-development tick (`DEVELOPMENT_MODEL.md` §5). Carries the
    /// resolved sparse attribute changes; the fold integer-adds them (clamped)
    /// and never re-runs growth math. Bounded (sparse, monthly, integer-
    /// quantized); replay reconstructs identical attribute histories by folding
    /// `changes`. Emitted by the calendar advance when a 30-day period boundary
    /// is crossed.
    DevelopmentTick {
        date: GameDate,
        changes: Vec<AttrStep>,
    },
    /// Season complete.
    SeasonEnded { champion: ClubId },
    /// A fresh season begins on the (developed) world (`DEVELOPMENT_MODEL.md`
    /// §5 multi-season continuity): a new fixture schedule and start date. The
    /// world snapshot carries over — this only resets the season's calendar,
    /// results, and champion.
    SeasonStarted {
        start_date: GameDate,
        schedule: Vec<Fixture>,
    },
    /// A completed transfer (`TRANSFER_MODEL.md` §4): the resolved fee and the
    /// buyer's freshly agreed contract — never a bid, which stays in the
    /// window's Trace (§4, §5). `from: None` is a free-agent signing, so there
    /// is no selling club to credit.
    TransferCompleted {
        date: GameDate,
        player: PlayerId,
        from: Option<ClubId>,
        to: ClubId,
        fee: Money,
        contract: Contract,
    },
    /// A club releases a player outright: he leaves the roster and the
    /// contract ends, with no fee changing hands.
    PlayerReleased {
        date: GameDate,
        player: PlayerId,
        club: ClubId,
    },
    /// An existing player's contract is replaced with newly resolved terms.
    ContractRenewed {
        date: GameDate,
        player: PlayerId,
        club: ClubId,
        contract: Contract,
    },
    /// The summer youth intake (`TRANSFER_MODEL.md` §8.1). Carries the
    /// **generated players** themselves, not a seed — the same choice
    /// `GameStarted` makes about the world snapshot, for the same reason:
    /// improving youth generation must never rewrite a recorded career.
    YouthIntake {
        date: GameDate,
        club: ClubId,
        players: Vec<Player>,
    },
    /// A player retires (`TRANSFER_MODEL.md` §8.2): he leaves every roster,
    /// his contract ends, and he is marked retired. He stays in `World.players`
    /// — the log still references him in historical `MatchPlayed` XIs.
    PlayerRetired { date: GameDate, player: PlayerId },
    /// The monthly finance tick (`TRANSFER_MODEL.md` §4) — money's
    /// `DevelopmentTick`. Carries resolved per-club revenue-minus-wages
    /// deltas the fold integer-adds to `Club.finances.balance`; fires on the
    /// same 30-day period-boundary crossing `DevelopmentTick` does.
    FinanceTick {
        date: GameDate,
        deltas: Vec<(ClubId, Money)>,
    },
    /// The human manager's resolved, validated transfer plan for the
    /// upcoming window (`TRANSFER_MODEL.md` §10's pre-commitment model,
    /// promoted from "the seam is left open"): the exact `TransferDecision`s
    /// `club_ai::RecordedPolicy` will replay, unchanged, in every round of
    /// that window's clearing loop — the same "record the resolved proposal,
    /// never the raw command" rule `LineupSubmitted` already follows.
    /// Overwrites any previously pending plan; consumed and cleared by the
    /// next `TransferWindowClosed`.
    TransferDecisionSubmitted {
        date: GameDate,
        club: ClubId,
        decisions: Vec<TransferDecision>,
    },
    /// A transfer window's clearing loop has resolved (`TRANSFER_MODEL.md`
    /// §5, §7) — emitted unconditionally for every window boundary crossed,
    /// even one that clears zero transfers, so the fold has a reliable point
    /// to expire a pre-committed human plan rather than letting it silently
    /// carry into the *next* window it was never meant for.
    TransferWindowClosed { date: GameDate, window_index: u64 },
}
