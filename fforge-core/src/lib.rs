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
pub mod development;
pub mod event;
pub mod match_engine;
pub mod observer;
pub mod rng;
pub mod schedule;
pub mod session;
pub mod state;
pub mod worldgen;

pub use commands::{Command, CommandError, FIXTURE_STREAM_NS, player_match_preview};
pub use development::{DEV_STREAM_NS, DevKnobs};
pub use event::{AttrStep, Event};
pub use observer::{EventObserver, SeasonTelemetry};
pub use session::{Session, load_log, save_log};
pub use state::{GameState, TableRow, league_table};
pub use worldgen::{WorldGenConfig, generate};

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

    use fforge_domain::{
        Attribute, GameDate, Lineup, PlayerId, ROLE_WEIGHTS, World, XI, best_role,
    };

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

    /// Play `seasons` full seasons, rolling over via `StartNextSeason` between
    /// them — so development (which fires monthly, offseason included) is active
    /// across a multi-year career, the Phase-3 continuity.
    fn run_seasons(seed: u64, seasons: usize) -> Session {
        let log = new_game(seed, &WorldGenConfig::default(), ClubId(0));
        let mut session = Session::from_events(log, &mut []);
        for s in 0..seasons {
            while !session.state.season_over() {
                session
                    .execute(Command::AdvanceMatchday, &mut [])
                    .expect("advance");
            }
            if s + 1 < seasons {
                session
                    .execute(Command::StartNextSeason, &mut [])
                    .expect("start next season");
            }
        }
        session
    }

    /// The world snapshot recorded in the opening `GameStarted` — the players'
    /// attributes *before* any development.
    fn initial_world(session: &Session) -> World {
        match &session.log[0] {
            Event::GameStarted { world, .. } => world.clone(),
            other => panic!("first event must be GameStarted, got {other:?}"),
        }
    }

    fn dev_ticks(session: &Session) -> Vec<&Event> {
        session
            .log
            .iter()
            .filter(|e| matches!(e, Event::DevelopmentTick { .. }))
            .collect()
    }

    const START_DATE: GameDate = GameDate {
        days: 2026 * 365 + 220,
    };

    #[test]
    fn development_is_active_across_seasons() {
        let session = run_seasons(2026, 3);
        // Monthly cadence over ~3 years: ~11-12 ticks/year, offseason included.
        let ticks = dev_ticks(&session);
        assert!(
            (30..=40).contains(&ticks.len()),
            "expected ~monthly dev ticks over 3 seasons, got {}",
            ticks.len()
        );
        let total_steps: usize = session
            .log
            .iter()
            .map(|e| match e {
                Event::DevelopmentTick { changes, .. } => changes.len(),
                _ => 0,
            })
            .sum();
        assert!(
            total_steps > 1000,
            "development barely moved: {total_steps} steps"
        );

        // Attributes actually changed vs the recorded starting snapshot.
        let init = initial_world(&session);
        let changed = init
            .players
            .keys()
            .filter(|&&pid| {
                init.player(pid).attributes != session.state.world.player(pid).attributes
            })
            .count();
        assert!(
            changed > init.players.len() / 2,
            "only {changed} players developed"
        );
    }

    #[test]
    fn development_ages_veterans_and_respects_pa() {
        let session = run_seasons(7, 3);
        let init = initial_world(&session);
        let world = &session.state.world;

        // Veterans (31+ at start) lose pace — physicals decline (§2.1).
        let (mut old_speed_delta, mut old_n) = (0i32, 0i32);
        // Aggregate best-role CA must not breach the PA ceiling (§2.2).
        let (mut sum_ca, mut sum_pa) = (0f64, 0f64);
        for (&pid, p0) in &init.players {
            let p1 = world.player(pid);
            for a in p1.attributes.as_array() {
                assert!(*a <= 100, "attribute clamp breached");
            }
            if p0.age(START_DATE) >= 31 {
                old_speed_delta += p1.attributes.get(Attribute::Speed) as i32
                    - p0.attributes.get(Attribute::Speed) as i32;
                old_n += 1;
            }
            sum_ca += best_role(&p1.attributes, &ROLE_WEIGHTS).1 as f64;
            sum_pa += p1.character.potential as f64;
        }
        assert!(old_n > 0);
        assert!(
            (old_speed_delta as f64 / old_n as f64) < -1.0,
            "veteran pace should decline, mean dSpeed {}",
            old_speed_delta as f64 / old_n as f64
        );
        let attainment = sum_ca / sum_pa;
        assert!(
            (0.80..=1.0).contains(&attainment),
            "population attainment {attainment} outside plausible band (PA respected in aggregate)"
        );
    }

    #[test]
    fn playing_time_drives_development() {
        // §3: minutes matter. Same seed, same world — but in one run the human
        // club plays its eleven *youngest* players every week; in the other they
        // are benched by the auto-picker. The played youngsters must develop more.
        let club = ClubId(0);
        let youth: Vec<PlayerId> = {
            let log = new_game(4242, &WorldGenConfig::default(), club);
            let init = match &log[0] {
                Event::GameStarted { world, .. } => world.clone(),
                other => panic!("expected GameStarted, got {other:?}"),
            };
            let mut ps = init.club(club).players.clone();
            // youngest first = largest birth-day count; tie-break by id.
            ps.sort_by_key(|&pid| (std::cmp::Reverse(init.player(pid).birth.days), pid));
            ps.truncate(XI);
            ps
        };

        let benched = run_seasons(4242, 3);
        let played = {
            let log = new_game(4242, &WorldGenConfig::default(), club);
            let mut session = Session::from_events(log, &mut []);
            let mut xi = [PlayerId(0); XI];
            xi.copy_from_slice(&youth);
            let lineup = Lineup {
                formation: 0,
                players: xi,
            };
            for s in 0..3 {
                session
                    .execute(Command::SubmitLineup(lineup.clone()), &mut [])
                    .expect("submit youth XI");
                while !session.state.season_over() {
                    session
                        .execute(Command::AdvanceMatchday, &mut [])
                        .expect("advance");
                }
                if s + 1 < 3 {
                    session
                        .execute(Command::StartNextSeason, &mut [])
                        .expect("next season");
                }
            }
            session
        };

        let gain = |session: &Session| -> i32 {
            let init = initial_world(session);
            youth
                .iter()
                .map(|&pid| {
                    best_role(&session.state.world.player(pid).attributes, &ROLE_WEIGHTS).1 as i32
                        - best_role(&init.player(pid).attributes, &ROLE_WEIGHTS).1 as i32
                })
                .sum()
        };
        let (g_played, g_benched) = (gain(&played), gain(&benched));
        assert!(
            g_played > g_benched,
            "playing youth ({g_played}) must develop them more than benching ({g_benched})"
        );
    }

    #[test]
    fn same_seed_identical_attribute_histories() {
        // The whole point of the seeded, record-outcomes design: two runs of the
        // same seed produce byte-identical development — same tick sequence, same
        // deltas, same final attributes — across multiple seasons.
        let a = run_seasons(2027, 3);
        let b = run_seasons(2027, 3);
        assert_eq!(dev_ticks(&a), dev_ticks(&b), "dev tick histories diverged");
        assert_eq!(a.log, b.log, "logs diverged");
        for (&pid, pa) in &a.state.world.players {
            assert_eq!(
                pa.attributes,
                b.state.world.player(pid).attributes,
                "attribute history diverged for {pid}"
            );
        }
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
        // Multi-season so DevelopmentTick + SeasonStarted folds are exercised:
        // the developed world must reconstruct identically from the log alone.
        let session = run_seasons(99, 3);
        assert!(!dev_ticks(&session).is_empty(), "no development to replay");
        let replayed = GameState::replay(&session.log);
        assert_eq!(session.state, replayed);
    }

    #[test]
    fn save_load_round_trip() {
        // Multi-season: the persisted log carries the whole developed career and
        // round-trips through JSON exactly (AttrStep deltas, appearances, etc.).
        let session = run_seasons(7, 3);
        assert!(!dev_ticks(&session).is_empty(), "no development to persist");
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
        let (w, d): (u32, u32) = table
            .iter()
            .fold((0, 0), |(w, d), r| (w + r.won, d + r.drawn));
        assert_eq!(total_pts, w * 3 + d);
    }

    #[test]
    fn aggregates_are_in_a_believable_ballpark() {
        // Phase-2a engine (MATCH_MODEL.md). The calibration harness
        // (`match_engine::calibrate::StreamTelemetry`, `bin/calibrate.rs`,
        // and `resolve.rs`'s `notebook_parity` test) diagnosed the original
        // ~1.7-2.0 goals/match this suite used to see: parity against
        // notebook-equivalent inputs held (loop is a faithful port, not a
        // port bug), coupling `p_wide` to each formation's actual
        // wide-presence share (`resolve::formation_p_wide`, `MATCH_MODEL.md`
        // §10 item 1) fixed the formation-shape mismatch but moved pooled
        // gpm by <0.01, and the dominant gap turned out to be conversion
        // (~7% vs the notebook's ~10%) — a `b_beat` re-tune against real
        // `worldgen`'s attribute distribution (`knobs.rs`'s doc comment)
        // closed it: this suite now reads ~2.5-2.6 goals/match, in line
        // with the notebook's own ~2.6 target. This stays a wide sanity
        // band that only needs to catch gross regressions/bugs going
        // forward; the harness is the place to re-chase the real number if
        // Phase 2e (tactics, cards, subs) moves it again.
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
