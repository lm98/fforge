//! fm-core — layer 2: the deterministic simulation core.
//!
//! Hard invariants enforced here (DESIGN.md):
//! - the core is a **pure fold over an append-only event log** (`state`);
//! - **no wall clock** (game-time derives from events — `GameDate` moves only
//!   via `MatchdayAdvanced`), **no unseeded randomness** (`rng` streams derive
//!   from the seed recorded in `GameStarted`), **no inline LLM calls** (none
//!   exist yet; when they do, their outputs arrive as recorded events);
//! - the evaluation/telemetry spine is a **passive event-stream consumer**
//!   (`observer`).
//!
//! `match_engine` runs the Phase 2a event-based possession engine
//! (`MATCH_MODEL.md`); only its score folds into `GameState` — the
//! minute-by-minute trace rides alongside, never inside, the fold.

pub mod commands;
pub mod event;
pub mod match_engine;
pub mod observer;
pub mod rng;
pub mod schedule;
pub mod session;
pub mod state;
pub mod worldgen;

pub use commands::{player_match_preview, Command, CommandError};
pub use event::Event;
pub use observer::{EventObserver, SeasonTelemetry};
pub use session::{load_log, save_log, Session};
pub use state::{league_table, GameState, TableRow};
pub use worldgen::{generate, WorldGenConfig};

/// Convenience: assemble the opening event for a new game.
pub fn new_game(seed: u64, cfg: &WorldGenConfig, player_club: fforge_domain::ClubId) -> Vec<Event> {
    let (world, schedule, start_date) = generate(seed, cfg);
    vec![Event::GameStarted {
        seed,
        start_date,
        player_club,
        world,
        schedule,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fforge_domain::ClubId;

    fn run_full_season(seed: u64) -> Session {
        let log = new_game(seed, &WorldGenConfig::default(), ClubId(0));
        let mut session = Session::from_events(log, &mut []);
        while !session.state.season_over() {
            session
                .execute(Command::AdvanceMatchday, &mut [])
                .expect("advance");
        }
        session
    }

    #[test]
    fn same_seed_same_season() {
        let a = run_full_season(20260706);
        let b = run_full_season(20260706);
        assert_eq!(a.log, b.log, "identical seeds must yield identical logs");
        assert_eq!(a.state, b.state);
    }

    #[test]
    fn different_seed_different_season() {
        let a = run_full_season(1);
        let b = run_full_season(2);
        assert_ne!(a.log, b.log);
    }

    #[test]
    fn replay_reconstructs_state_exactly() {
        let session = run_full_season(99);
        let replayed = GameState::replay(&session.log);
        assert_eq!(session.state, replayed);
    }

    #[test]
    fn save_load_round_trip() {
        let session = run_full_season(7);
        let dir = std::env::temp_dir().join("fmsim-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.fml");
        save_log(&path, &session.log).unwrap();
        let loaded = load_log(&path).unwrap();
        assert_eq!(session.log, loaded);
        assert_eq!(GameState::replay(&loaded), session.state);
    }

    #[test]
    fn season_shape_is_sane() {
        let session = run_full_season(123);
        // 20 clubs → 38 matchdays, 380 results, every club played 38.
        assert_eq!(session.state.results.len(), 380);
        let table = league_table(
            &session.state.world,
            &session.state.schedule,
            &session.state.results,
        );
        assert_eq!(table.len(), 20);
        assert!(table.iter().all(|r| r.played == 38));
        assert_eq!(session.state.champion, Some(table[0].club));
        // Points must be internally consistent.
        let total_pts: u32 = table.iter().map(|r| r.points()).sum();
        let (w, d): (u32, u32) = table.iter().fold((0, 0), |(w, d), r| (w + r.won, d + r.drawn));
        assert_eq!(total_pts, w * 3 + d);
    }

    #[test]
    fn aggregates_are_in_a_believable_ballpark() {
        // Phase-2a engine (MATCH_MODEL.md), knobs fitted in the Python
        // prototype against its own synthetic squad generator — not this
        // crate's worldgen. Real worldgen's attribute distribution differs
        // slightly (this crate models age/PA/youth discount; the notebook's
        // squad generator doesn't), so the pooled reading here (~1.7-2.0
        // goals/match) sits a bit under the notebook's fitted ~2.6 target.
        // Closing that gap is exactly what the deferred Rust calibration
        // harness (MATCH_MODEL.md §10) re-tunes; this stays a wide sanity
        // band that only needs to catch gross regressions/bugs.
        let mut telemetry = SeasonTelemetry::default();
        for seed in 0..10u64 {
            let session = run_full_season(seed);
            for e in &session.log {
                telemetry.on_event(e);
            }
        }
        let gpm = telemetry.goals_per_match();
        assert!(
            (1.2..=4.0).contains(&gpm),
            "goals/match {gpm} outside sanity band"
        );
        assert!(
            telemetry.home_win_rate() > telemetry.away_wins as f64 / telemetry.matches as f64,
            "home advantage must be visible"
        );
    }
}