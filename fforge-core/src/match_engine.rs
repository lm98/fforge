//! The Phase-2a event-based possession match engine (`MATCH_MODEL.md`),
//! behind the same `play_match` call site the Phase-1 crude engine used to
//! occupy. State space, resolution model, the wide route, and the knob
//! table are a faithful Rust port of the calibrated Python prototype
//! (`match_model_prototype.ipynb`, referenced from `MATCH_MODEL.md` §1) —
//! nothing here is a re-guess of the shape-finding, only its translation.
//!
//! Deferred to Phase 2e (behind this same call site, no structural change):
//! tactics as transition-matrix modifiers, cards & fouls, injuries, set
//! pieces, substitutions, and the character/hidden attributes.

mod calibrate;
mod contest;
mod knobs;
mod resolve;
mod stream;
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
    ClubId, FORMATIONS, Lineup, PlayerId, ROLE_WEIGHTS, Role, World, XI, current_ability,
};

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
