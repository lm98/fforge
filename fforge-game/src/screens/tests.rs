//! Snapshot tests for every screen (R16).
//!
//! `fforge-game` had no output tests at all before this; every later change —
//! colour, new columns, new screens — could silently break alignment or emit
//! an escape sequence into piped output with nothing to catch it.
//!
//! The snapshots are plain committed `.txt` files under `fforge-game/snapshots/`
//! compared with `assert_eq!` — no new dependency. Regenerate them all with:
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test -p fforge-game
//! ```
//!
//! and **read the diff before committing it** — an unexplained snapshot churn
//! is the whole thing these tests exist to catch.
//!
//! Everything here runs against one fixed seed, so the world, the fixtures,
//! and every result are reproducible: that is the same determinism guarantee
//! the core's own test suite leans on.

use crate::screens::{fixtures, header, season_end, squad, stats, table};
use fforge_core::{Command, SeasonTelemetry, Session, WorldGenConfig, new_game};
use fforge_domain::ClubId;
use std::path::PathBuf;

/// One fixed seed for every snapshot, so a fixture is a pure function of the
/// number of matchdays played.
const SEED: u64 = 0xF00D_BEEF;

/// The human's club. `ClubId(0)` is stable across worldgen for a given seed.
const MY_CLUB: ClubId = ClubId(0);

/// A session with `matchdays` matchdays played. `AdvanceMatchday` is
/// deterministic given the seed, so this is reproducible.
fn fixture(matchdays: usize) -> (Session, SeasonTelemetry) {
    let log = new_game(SEED, &WorldGenConfig::default(), MY_CLUB);
    let mut telemetry = SeasonTelemetry::default();
    let mut session = Session::from_events(log, &mut [&mut telemetry]);
    for _ in 0..matchdays {
        session
            .execute(Command::AdvanceMatchday, &mut [&mut telemetry])
            .expect("advance within the season");
    }
    (session, telemetry)
}

/// A session played to the final whistle of the season.
fn finished_season() -> (Session, SeasonTelemetry) {
    let log = new_game(SEED, &WorldGenConfig::default(), MY_CLUB);
    let mut telemetry = SeasonTelemetry::default();
    let mut session = Session::from_events(log, &mut [&mut telemetry]);
    while !session.state.season_over() {
        session
            .execute(Command::AdvanceMatchday, &mut [&mut telemetry])
            .expect("advance until the season ends");
    }
    (session, telemetry)
}

fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join(format!("{name}.txt"))
}

/// Compare `actual` against the committed snapshot, or rewrite it when
/// `UPDATE_SNAPSHOTS` is set.
fn assert_snapshot(name: &str, actual: &str) {
    let path = snapshot_path(name);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("snapshots dir"))
            .expect("create snapshot dir");
        std::fs::write(&path, actual).expect("write snapshot");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing snapshot {}: {e}\nRe-run with UPDATE_SNAPSHOTS=1 to create it.",
            path.display()
        )
    });
    assert_eq!(
        actual,
        expected,
        "screen `{name}` no longer matches its snapshot ({}).\n\
         If the change is intended, re-run with UPDATE_SNAPSHOTS=1 and review the diff.",
        path.display()
    );
}

#[test]
fn squad_screen_snapshot() {
    let (session, _) = fixture(0);
    assert_snapshot("squad", &squad::render(&session));
}

#[test]
fn table_screen_snapshot() {
    let (session, _) = fixture(5);
    assert_snapshot("table", &table::render(&session));
}

#[test]
fn fixtures_screen_snapshot() {
    let (session, _) = fixture(5);
    assert_snapshot("fixtures", &fixtures::render(&session));
}

/// Matchday 1 has no previous matchday, so the results half is absent — a
/// distinct branch worth pinning.
#[test]
fn fixtures_screen_snapshot_first_matchday() {
    let (session, _) = fixture(0);
    assert_snapshot("fixtures_matchday_1", &fixtures::render(&session));
}

#[test]
fn stats_screen_snapshot() {
    let (_, telemetry) = fixture(5);
    assert_snapshot("stats", &stats::render(&telemetry));
}

/// The zero-matches branch of the stats screen divides by `matches`, so an
/// empty telemetry is its own case.
#[test]
fn stats_screen_snapshot_before_any_match() {
    let (_, telemetry) = fixture(0);
    assert_snapshot("stats_empty", &stats::render(&telemetry));
}

#[test]
fn header_snapshot() {
    let (session, _) = fixture(5);
    assert_snapshot("header", &header::render(&session));
}

#[test]
fn season_end_snapshot() {
    let (session, telemetry) = finished_season();
    assert_snapshot("season_end", &season_end::render(&session, &telemetry));
}

/// **The test that protects every piped consumer, CI included** (R16).
///
/// With colour disabled — which is every screen's state today, and will stay
/// the state under `NO_COLOR`/`--no-color`/non-tty once U2 lands the `Sem`
/// vocabulary — no screen may emit an ANSI escape.
#[test]
fn no_ansi_escapes_when_colour_is_disabled() {
    let (session, telemetry) = fixture(5);
    let (finished, finished_telemetry) = finished_season();
    let rendered = [
        ("squad", squad::render(&session)),
        ("table", table::render(&session)),
        ("fixtures", fixtures::render(&session)),
        ("stats", stats::render(&telemetry)),
        ("header", header::render(&session)),
        (
            "season_end",
            season_end::render(&finished, &finished_telemetry),
        ),
    ];
    for (name, output) in rendered {
        assert!(
            !output.contains('\u{1b}'),
            "screen `{name}` emitted an ANSI escape with colour disabled"
        );
    }
}
