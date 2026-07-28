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

use crate::render::sem::Palette;
use crate::screens::{
    availability, finances, fixtures, header, inbox, season_end, squad, stats, table,
};
use fforge_core::news::NewsObserver;
use fforge_core::{Command, EventObserver, SeasonTelemetry, Session, WorldGenConfig, new_game};
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

/// A session's news observer, rebuilt over the same fixed-seed run. Kept
/// separate from `fixture` so the screens that don't need it don't pay for it.
fn news_fixture(matchdays: usize) -> NewsObserver {
    let log = new_game(SEED, &WorldGenConfig::default(), MY_CLUB);
    let mut news = NewsObserver::new();
    let mut session = {
        let obs: &mut [&mut dyn EventObserver] = &mut [&mut news];
        Session::from_events(log, obs)
    };
    for _ in 0..matchdays {
        session
            .execute(Command::AdvanceMatchday, &mut [&mut news])
            .expect("advance within the season");
        // Matches `game_loop`: state-condition news is pumped once per command.
        news.check_conditions(&session.state);
    }
    news
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
    assert_snapshot("squad", &squad::render(&session, Palette::PLAIN));
}

/// The finances screen wants a session far enough in that at least one
/// monthly `FinanceTick` has fired, or the trend half never renders.
#[test]
fn finances_screen_snapshot() {
    let (session, _) = fixture(20);
    assert_snapshot("finances", &finances::render(&session, Palette::PLAIN));
}

/// ...and the pre-first-tick branch is its own case.
#[test]
fn finances_screen_snapshot_before_any_tick() {
    let (session, _) = fixture(0);
    assert_snapshot(
        "finances_no_ticks",
        &finances::render(&session, Palette::PLAIN),
    );
}

/// The inbox after a few matchdays: both news categories present
/// (event-derived results, state-condition checks), ordered by salience.
#[test]
fn inbox_screen_snapshot() {
    let (session, _) = fixture(5);
    let news = news_fixture(5);
    assert_snapshot("inbox", &inbox::render(&session, &news, 4, Palette::PLAIN));
}

/// The empty branch — before anything has happened at all.
#[test]
fn inbox_screen_snapshot_when_empty() {
    let (session, _) = fixture(0);
    let news = news_fixture(0);
    assert_snapshot(
        "inbox_empty",
        &inbox::render(&session, &news, 0, Palette::PLAIN),
    );
}

/// Availability after enough matchdays for real injuries and cards to have
/// accumulated — the empty-status branch is not the interesting one.
#[test]
fn availability_screen_snapshot() {
    let (session, _) = fixture(12);
    assert_snapshot(
        "availability",
        &availability::render(&session, Palette::PLAIN),
    );
}

#[test]
fn table_screen_snapshot() {
    let (session, _) = fixture(5);
    assert_snapshot("table", &table::render(&session, Palette::PLAIN));
}

#[test]
fn fixtures_screen_snapshot() {
    let (session, _) = fixture(5);
    assert_snapshot("fixtures", &fixtures::render(&session, Palette::PLAIN));
}

/// Matchday 1 has no previous matchday, so the results half is absent — a
/// distinct branch worth pinning.
#[test]
fn fixtures_screen_snapshot_first_matchday() {
    let (session, _) = fixture(0);
    assert_snapshot(
        "fixtures_matchday_1",
        &fixtures::render(&session, Palette::PLAIN),
    );
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
    assert_snapshot("header", &header::render(&session, 3, Palette::PLAIN));
}

#[test]
fn season_end_snapshot() {
    let (session, telemetry) = finished_season();
    assert_snapshot(
        "season_end",
        &season_end::render(&session, &telemetry, Palette::PLAIN),
    );
}

/// Every screen, rendered both ways, for the whole-suite invariants below.
///
/// **The fixture depth is load-bearing.** `the_screens_with_an_axis_actually_colour`
/// needs a session where each screen's axis has something to *say*: a squad
/// with nobody injured, suspended, or tired is correctly all `Sem::Ok` on the
/// availability screen, and `Ok` costs no ink by design. Twelve matchdays is
/// deep enough for real cards and layoffs to have accumulated.
fn every_screen(p: Palette) -> Vec<(&'static str, String)> {
    let (session, telemetry) = fixture(12);
    let news = news_fixture(12);
    let (finished, finished_telemetry) = finished_season();
    vec![
        ("squad", squad::render(&session, p)),
        ("availability", availability::render(&session, p)),
        ("inbox", inbox::render(&session, &news, 4, p)),
        ("finances", finances::render(&session, p)),
        ("table", table::render(&session, p)),
        ("fixtures", fixtures::render(&session, p)),
        ("stats", stats::render(&telemetry)),
        ("header", header::render(&session, 3, p)),
        (
            "season_end",
            season_end::render(&finished, &finished_telemetry, p),
        ),
    ]
}

/// **The test that protects every piped consumer, CI included** (R16).
///
/// With colour disabled — the state under `NO_COLOR`, `--no-color`, or a
/// non-tty stdout — no screen may emit an ANSI escape.
#[test]
fn no_ansi_escapes_when_colour_is_disabled() {
    for (name, output) in every_screen(Palette::PLAIN) {
        assert!(
            !output.contains('\u{1b}'),
            "screen `{name}` emitted an ANSI escape with colour disabled"
        );
    }
}

/// The other half of R15's bargain: **colour must be purely additive.** Strip
/// the escapes from a coloured render and you must be back at the plain one,
/// byte for byte — no extra glyph, no different column, no re-ordering that
/// only the coloured path gets. That is what makes the plain snapshots a
/// complete record of what a screen says.
#[test]
fn colour_changes_nothing_but_colour() {
    let plain = every_screen(Palette::PLAIN);
    let coloured = every_screen(Palette::COLOURED);
    for ((name, plain), (_, coloured)) in plain.into_iter().zip(coloured) {
        assert_eq!(
            strip_ansi(&coloured),
            plain,
            "screen `{name}` renders different *content* with colour on"
        );
    }
}

/// The screens R15 assigns an axis to must actually use it — a screen that
/// silently stopped colouring would otherwise pass every other test here.
#[test]
fn the_screens_with_an_axis_actually_colour() {
    for (name, output) in every_screen(Palette::COLOURED) {
        // `stats` is the documented exception: raw readings, no axis.
        if name == "stats" {
            continue;
        }
        assert!(
            output.contains('\u{1b}'),
            "screen `{name}` has a colour axis but emitted no colour"
        );
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
