//! The Phase-2a event-based possession match engine (`MATCH_MODEL.md`),
//! behind the same `play_match` call site the Phase-1 crude engine used to
//! occupy. State space, resolution model, the wide route, and the knob
//! table are a faithful Rust port of the calibrated Python prototype
//! (`match_model_prototype.ipynb`, referenced from `MATCH_MODEL.md` §1) —
//! nothing here is a re-guess of the shape-finding, only its translation.
//!
//! Phase 2e has begun: tactics (`TACTICS_MODEL.md`) lands as transition-
//! matrix modifiers behind this same call site (no structural change).
//! Still deferred: cards & fouls, injuries, set pieces, substitutions, and
//! the character/hidden attributes.

mod calibrate;
mod contest;
mod knobs;
mod resolve;
mod stream;
mod tactics;
mod zone;

pub use calibrate::{
    DeviationReport, ELO_SCALE_S, FormationStats, GapBinStats, GapDeviation, StreamTelemetry,
    elo_expected,
};
pub use knobs::Knobs;
pub use stream::{MatchEvent, MatchEventKind, ShotKind, ShotOutcome, ShotSource, Side};
pub use zone::Zone;

use crate::rng::Rng;
use fforge_domain::{
    ClubId, FORMATIONS, Lineup, PlayerId, ROLE_WEIGHTS, Role, Tactics, World, XI, current_ability,
};
use serde::{Deserialize, Serialize};

/// A resolved injury (`MATCH_MODEL.md` §12, §14): the *days out*, decided at
/// match time — never a severity category for the fold to re-roll, so the
/// severity model can evolve without rewriting anyone's recorded medical
/// history (the `DevelopmentTick` argument verbatim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjuryOutcome {
    pub player: PlayerId,
    /// Days unavailable, counted from the match date. The fold turns this
    /// into `Player.injured_until`.
    pub days_out: u16,
}

/// The card itself (`MATCH_MODEL.md` §15). A second yellow is recorded as
/// `SecondYellow` — a red by bookkeeping — so no consumer ever has to
/// reconstruct the distinction from minute ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Card {
    Yellow,
    SecondYellow,
    Red,
}

/// A resolved card (`MATCH_MODEL.md` §12, §15): the recorded truth from which
/// suspensions are *derived* in the fold — a ban is never stored and never its
/// own event (the derived-suspension rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardOutcome {
    pub player: PlayerId,
    pub card: Card,
    pub minute: u8,
}

/// Mean CA-in-slot-role over the eleven — a squad-quality scalar independent
/// of any particular match-resolution model. Used for display and by
/// `ai_pick_lineup`'s formation comparison below.
pub fn lineup_strength(world: &World, lineup: &Lineup) -> f64 {
    let def = lineup.formation_def();
    let mut sum = 0.0;
    for (slot, &pid) in lineup.players.iter().enumerate() {
        let player = world.player(pid);
        sum += current_ability(&player.attributes, def.slots[slot], &ROLE_WEIGHTS) as f64;
    }
    sum / XI as f64
}

/// The result of a simulated match: the score that folds into `GameState`
/// (via `Event::MatchPlayed`) plus the minute-by-minute trace. The trace
/// rides alongside the fold, never inside it (`MATCH_MODEL.md` §7) — it is a
/// Trace, not a fold input, and callers are free to discard it. Nothing here
/// is persisted by `commands::advance_matchday`; only the score is.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchOutcome {
    pub home_goals: u8,
    pub away_goals: u8,
    pub stream: Vec<MatchEvent>,
    /// Resolved per-player consequences that outlive the match
    /// (`MATCH_MODEL.md` §12): unlike `stream`, these *do* ride into
    /// `Event::MatchPlayed`. The boundary is grown once, ahead of the models
    /// that fill it — the engine emits all three empty until the §14 injury
    /// model, §15 foul/card contest, and §18 rating derivation land, so
    /// nothing here may touch the RNG draw sequence.
    pub injuries: Vec<InjuryOutcome>,
    pub cards: Vec<CardOutcome>,
    /// Per-player rating in tenths (`68` = 6.8), `MATCH_MODEL.md` §18.
    pub ratings: Vec<(PlayerId, u8)>,
    /// True minutes played, substitutions included (`MATCH_MODEL.md` §12,
    /// §16, R7). Every starting-XI player at 90 until T10/T11/T12 (injuries,
    /// red cards, substitutions) make partial minutes possible — there is no
    /// bench yet, so "0 otherwise" is simply absence from this vec, not a
    /// recorded entry.
    pub minutes: Vec<(PlayerId, u8)>,
}

/// Simulate one match: `(lineups, world, rng)` in, score + trace out. A pure
/// function of its inputs — same seed stream, same outcome, by construction
/// (`MATCH_MODEL.md` §7).
pub fn play_match(world: &World, home: &Lineup, away: &Lineup, rng: &mut Rng) -> MatchOutcome {
    resolve::play_match(world, home, away, rng)
}

/// Deterministic AI team selection: for each formation, greedily fill slots
/// with the best remaining player by CA-in-slot-role (ties → lower player
/// id); keep the formation with the best mean. This is the Phase-1 stub of
/// the layer-3 club decision AI — same seam, richer policy later.
pub fn ai_pick_lineup(world: &World, club: ClubId) -> Lineup {
    let squad: Vec<PlayerId> = world.club(club).players.clone();
    let mut best: Option<(f64, Lineup)> = None;

    for (fi, formation) in FORMATIONS.iter().enumerate() {
        let mut remaining = squad.clone();
        let mut chosen = [PlayerId(0); XI];
        let mut total = 0.0;
        for (slot, &role) in formation.slots.iter().enumerate() {
            let (idx, ca) = pick_best(world, &remaining, role);
            chosen[slot] = remaining.remove(idx);
            total += ca as f64;
        }
        let mean = total / XI as f64;
        let candidate = Lineup {
            formation: fi as u8,
            players: chosen,
            // T7 adds ai_pick_tactics; until then every AI side plays
            // neutral (T6's scope fence — nothing selects non-neutral
            // tactics here).
            tactics: Tactics::neutral(),
        };
        match &best {
            Some((score, _)) if *score >= mean => {}
            _ => best = Some((mean, candidate)),
        }
    }
    best.expect("at least one formation").1
}

fn pick_best(world: &World, pool: &[PlayerId], role: Role) -> (usize, u8) {
    let mut best_idx = 0;
    let mut best_ca = 0u8;
    let mut best_id = PlayerId(u32::MAX);
    for (i, &pid) in pool.iter().enumerate() {
        let ca = current_ability(&world.player(pid).attributes, role, &ROLE_WEIGHTS);
        if ca > best_ca || (ca == best_ca && pid < best_id) {
            best_idx = i;
            best_ca = ca;
            best_id = pid;
        }
    }
    (best_idx, best_ca)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::derive_stream;
    use fforge_domain::World;

    fn tiny_world_and_lineups() -> (World, Lineup, Lineup) {
        super::golden::phase_2a_world_and_lineups()
    }

    #[test]
    fn every_event_names_an_actor_in_the_fielding_sides_xi() {
        // The identity enrichment invariant (MATCH_MODEL.md §9): each beat's
        // `actor` is a player the resolver sampled from the `side`-relative
        // fielding XI, so it must be a member of that XI — and `opponent`,
        // when a contest names one, must belong to the other side's XI. No
        // event may reference a player who was not on the pitch for its side.
        let (world, home, away) = tiny_world_and_lineups();
        let home_xi: std::collections::BTreeSet<_> = home.players.iter().copied().collect();
        let away_xi: std::collections::BTreeSet<_> = away.players.iter().copied().collect();
        for seed in 0..64u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play_match(&world, &home, &away, &mut rng);
            for event in &outcome.stream {
                let (fielding, opposing) = match event.side {
                    Side::Home => (&home_xi, &away_xi),
                    Side::Away => (&away_xi, &home_xi),
                };
                assert!(
                    fielding.contains(&event.actor),
                    "seed {seed}: {:?} at {}' names actor {} who is not in the fielding side's XI",
                    event.kind,
                    event.minute,
                    event.actor
                );
                if let Some(opponent) = event.opponent {
                    assert!(
                        opposing.contains(&opponent),
                        "seed {seed}: {:?} at {}' names opponent {opponent} who is not in the \
                         opposing side's XI",
                        event.kind,
                        event.minute
                    );
                }
            }
        }
    }

    #[test]
    fn boundary_consequences_stay_empty_until_the_2e_models_land() {
        // MATCH_MODEL.md §12/§11 sequencing step 1: the boundary is grown
        // ahead of the models that fill it, so the engine must emit all
        // three vectors empty — anything else here means an unsanctioned
        // model (and its RNG draws) sneaked in ahead of its design gate.
        let (world, home, away) = tiny_world_and_lineups();
        for seed in 0..32u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play_match(&world, &home, &away, &mut rng);
            assert!(
                outcome.injuries.is_empty()
                    && outcome.cards.is_empty()
                    && outcome.ratings.is_empty(),
                "seed {seed}: the 2a engine must populate no 2e consequences"
            );
        }
    }

    #[test]
    fn same_seed_same_outcome() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut r1 = derive_stream(99, 1);
        let mut r2 = derive_stream(99, 1);
        let a = play_match(&world, &home, &away, &mut r1);
        let b = play_match(&world, &home, &away, &mut r2);
        assert_eq!(
            a, b,
            "identical (lineups, world, rng stream) must yield an identical outcome"
        );
    }

    #[test]
    fn different_streams_can_diverge() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut r1 = derive_stream(1, 1);
        let mut r2 = derive_stream(2, 1);
        let a = play_match(&world, &home, &away, &mut r1);
        let b = play_match(&world, &home, &away, &mut r2);
        assert_ne!(
            a.stream, b.stream,
            "different rng streams should not replay identically"
        );
    }

    #[test]
    fn stream_is_never_empty_and_ends_with_a_final_score_consistent_with_shot_events() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut rng = derive_stream(42, 1);
        let outcome = play_match(&world, &home, &away, &mut rng);
        assert!(
            !outcome.stream.is_empty(),
            "a 90-minute match must produce events"
        );
        let goal_events = outcome
            .stream
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    MatchEventKind::Shot {
                        outcome: ShotOutcome::Goal,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            goal_events,
            outcome.home_goals as usize + outcome.away_goals as usize,
            "every goal in the score must have exactly one corresponding Shot{{outcome: Goal}} event"
        );
    }

    #[test]
    fn identical_squads_show_a_structural_home_advantage() {
        // Same club on both sides of the ball — the only asymmetry left is
        // home_bias and each half's kickoff. Pooled over many seeds, home
        // must win more often than away (mirrors the Phase-1 crude-engine
        // home-advantage invariant, now against the real resolution model).
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _schedule, _start) = crate::worldgen::generate(7, &cfg);
        let club = world.competition.clubs[0];
        let lineup = ai_pick_lineup(&world, club);

        let mut home_wins = 0u32;
        let mut away_wins = 0u32;
        for seed in 0..200u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play_match(&world, &lineup, &lineup, &mut rng);
            match outcome.home_goals.cmp(&outcome.away_goals) {
                std::cmp::Ordering::Greater => home_wins += 1,
                std::cmp::Ordering::Less => away_wins += 1,
                std::cmp::Ordering::Equal => {}
            }
        }
        assert!(
            home_wins > away_wins,
            "home_bias must be visible: {home_wins} home wins vs {away_wins} away wins"
        );
    }
}

/// The pinned Phase-2a golden baseline (batch-3 handoff T5): the reference
/// every 2e identity invariant in the batch asserts against (§2.1). Captured
/// as the last commit before any engine change, per the handoff's explicit
/// ordering — it cannot be captured retroactively, since T3 (`natural_fitness`)
/// already changed which world every worldgen seed produces.
///
/// `TACTICS_MODEL.md` §4's `neutral_tactics_reproduce_phase_2a_bit_for_bit`
/// (T6) replays these seeds through the tactics-aware engine at
/// `neutral()`/`neutral()` and asserts equality against this table; any
/// accidental extra draw or perturbed probability at the neutral setting
/// fails it loudly, as a wiring bug rather than a value to update.
#[cfg(test)]
pub(crate) mod golden {
    use super::*;
    use crate::rng::derive_stream;

    /// The exact fixture `TACTICS_MODEL.md` §4 names: a 2-club world at
    /// worldgen seed 7, `ai_pick_lineup` XIs for each club.
    pub(crate) fn phase_2a_world_and_lineups() -> (World, Lineup, Lineup) {
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _schedule, _start) = crate::worldgen::generate(7, &cfg);
        let clubs = world.competition.clubs.clone();
        let home = ai_pick_lineup(&world, clubs[0]);
        let away = ai_pick_lineup(&world, clubs[1]);
        (world, home, away)
    }

    /// `(home_goals, away_goals, stream.len())` for seeds `0..32`, RNG stream
    /// tag `1` (`derive_stream(seed, 1)`), against
    /// `phase_2a_world_and_lineups()`. The two clubs in this seed-7, 2-club
    /// world sit at the extreme ends of worldgen's quality spread (§3's
    /// evenly-spread-then-shuffled anchors collapse to the min/max with only
    /// two clubs), so every match is a lopsided home win — irrelevant here,
    /// since this table exists to catch *any* movement, not to be a
    /// representative match.
    pub(crate) const PHASE_2A_SEEDS_0_32: [(u8, u8, usize); 32] = [
        (16, 0, 869),
        (20, 0, 873),
        (14, 0, 866),
        (13, 0, 867),
        (14, 0, 857),
        (18, 0, 882),
        (18, 0, 870),
        (9, 0, 862),
        (10, 0, 861),
        (20, 0, 877),
        (20, 0, 865),
        (12, 0, 879),
        (18, 0, 865),
        (10, 0, 862),
        (14, 0, 852),
        (15, 0, 870),
        (11, 0, 859),
        (14, 0, 869),
        (18, 0, 860),
        (14, 0, 869),
        (12, 0, 860),
        (12, 0, 872),
        (8, 0, 860),
        (16, 0, 871),
        (20, 0, 858),
        (12, 0, 877),
        (20, 0, 864),
        (15, 0, 868),
        (12, 0, 857),
        (12, 0, 856),
        (23, 0, 881),
        (19, 0, 863),
    ];

    #[test]
    fn phase_2a_golden_baseline_reproduces() {
        // Tracks whatever `ai_pick_lineup` currently produces — neutral
        // tactics today (T6's scope fence), but T7 will make it call
        // `ai_pick_tactics`, at which point this reading is expected to move
        // and gets re-pinned deliberately (§8's rollout discipline), same as
        // `favourite_discrimination_regression_guard`.
        let (world, home, away) = phase_2a_world_and_lineups();
        for (seed, &(hg, ag, len)) in (0u64..32).zip(PHASE_2A_SEEDS_0_32.iter()) {
            let mut rng = derive_stream(seed, 1);
            let outcome = play_match(&world, &home, &away, &mut rng);
            assert_eq!(
                (outcome.home_goals, outcome.away_goals, outcome.stream.len()),
                (hg, ag, len),
                "seed {seed}: Phase-2a golden baseline moved — a wiring bug \
                 in whatever landed since T5, never a re-tune"
            );
        }
    }

    #[test]
    fn neutral_tactics_reproduce_phase_2a_bit_for_bit() {
        // TACTICS_MODEL.md §4's named golden test: explicitly force
        // `Tactics::neutral()` on both sides — independent of whatever
        // `ai_pick_lineup` defaults to (today neutral, but T7 changes that)
        // — so this stays a permanent bit-identity guardrail rather than
        // tracking the AI policy's evolving choice.
        let (world, mut home, mut away) = phase_2a_world_and_lineups();
        home.tactics = fforge_domain::Tactics::neutral();
        away.tactics = fforge_domain::Tactics::neutral();
        for (seed, &(hg, ag, len)) in (0u64..32).zip(PHASE_2A_SEEDS_0_32.iter()) {
            let mut rng = derive_stream(seed, 1);
            let outcome = play_match(&world, &home, &away, &mut rng);
            assert_eq!(
                (outcome.home_goals, outcome.away_goals, outcome.stream.len()),
                (hg, ag, len),
                "seed {seed}: neutral tactics must reproduce the Phase-2a \
                 baseline bit-for-bit (§4) — movement here is a wiring bug, \
                 never a re-tune"
            );
        }
    }
}
