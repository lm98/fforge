//! `step(state, command) -> events` — the deterministic transition producer.
//!
//! This is the propose-then-validate gate in miniature: a `Command` is a
//! *proposal* (from the human today; from LLM agents in Phase 5), validation
//! happens here, and only resolved, validated values become events. `step`
//! never mutates state — callers apply the returned events through the fold.

use crate::development::{self, period_date, period_index, DevKnobs};
use crate::event::Event;
use crate::finance::{finance_deltas, FinanceKnobs};
use crate::match_engine::{ai_pick_lineup, play_match};
use crate::rng::derive_stream;
use crate::schedule::double_round_robin;
use crate::state::{league_table, GameState};
use fforge_domain::{ClubId, GameDate, Lineup, PlayerId, World, FORMATIONS, XI};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Submit the human club's team sheet for the upcoming matchday.
    SubmitLineup(Lineup),
    /// Simulate every fixture of the current matchday and advance the calendar.
    AdvanceMatchday,
    /// Begin a fresh season on the (developed) world once the current one is
    /// over — the multi-season continuity development needs (`DEVELOPMENT_MODEL.md`
    /// §5). Runs the offseason development ticks, then resets the calendar.
    StartNextSeason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    SeasonOver,
    SeasonNotOver,
    UnknownFormation(u8),
    DuplicatePlayers,
    NotInSquad(PlayerId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::SeasonOver => write!(f, "the season is over"),
            CommandError::SeasonNotOver => write!(f, "the season is not over yet"),
            CommandError::UnknownFormation(i) => write!(f, "unknown formation index {i}"),
            CommandError::DuplicatePlayers => write!(f, "a player appears twice in the lineup"),
            CommandError::NotInSquad(p) => write!(f, "player {p} is not in your squad"),
        }
    }
}

/// Tag namespace for per-fixture RNG streams (see rng::derive_stream). Public
/// so the calibration harness (`fforge-core/src/bin/calibrate.rs`) can derive
/// the exact same per-fixture stream `advance_matchday` uses without
/// duplicating the constant.
pub const FIXTURE_STREAM_NS: u64 = 0x4D41_5443_0000_0000; // "MATC"

/// Season kickoff is late-summer, day 220 (matching `worldgen`).
const SEASON_START_DOY: u16 = 220;

pub fn step(state: &GameState, command: Command) -> Result<Vec<Event>, CommandError> {
    match command {
        Command::StartNextSeason => {
            if !state.season_over() {
                return Err(CommandError::SeasonNotOver);
            }
            Ok(start_next_season(state))
        }
        other => {
            if state.season_over() {
                return Err(CommandError::SeasonOver);
            }
            match other {
                Command::SubmitLineup(lineup) => {
                    validate_lineup(state, &lineup)?;
                    Ok(vec![Event::LineupSubmitted {
                        matchday: state.current_matchday,
                        lineup,
                    }])
                }
                Command::AdvanceMatchday => Ok(advance_matchday(state)),
                Command::StartNextSeason => unreachable!("handled above"),
            }
        }
    }
}

fn validate_lineup(state: &GameState, lineup: &Lineup) -> Result<(), CommandError> {
    if lineup.formation as usize >= FORMATIONS.len() {
        return Err(CommandError::UnknownFormation(lineup.formation));
    }
    let mut seen = BTreeSet::new();
    for &pid in &lineup.players {
        if !seen.insert(pid) {
            return Err(CommandError::DuplicatePlayers);
        }
    }
    let squad = &state.world.club(state.player_club).players;
    for &pid in &lineup.players {
        if !squad.contains(&pid) {
            return Err(CommandError::NotInSquad(pid));
        }
    }
    debug_assert_eq!(lineup.players.len(), XI);
    Ok(())
}

/// The human club's effective lineup for this matchday: the submitted one,
/// else the last one used, else a deterministic auto-pick. Never fails —
/// forgetting to set a team costs quality, not a crash.
fn effective_player_lineup(state: &GameState) -> Lineup {
    if let Some(lineup) = &state.pending_lineup {
        return lineup.clone();
    }
    if let Some(lineup) = &state.last_lineup {
        return lineup.clone();
    }
    ai_pick_lineup(&state.world, state.player_club)
}

/// The player's own fixture for the upcoming matchday, simulated exactly as
/// `advance_matchday` is about to simulate it (same lineup selection, same
/// seed-derived RNG stream) — a pure query, computed from `state` and
/// discarded by the caller, that never mutates anything or produces an
/// `Event`. Because it re-derives from the same inputs `advance_matchday`
/// consumes, its score can never disagree with what `Command::AdvanceMatchday`
/// actually records. Live-viewing consumers (fforge-game's main game loop)
/// call this *before* executing `AdvanceMatchday` to render the humble text
/// match view (`DESIGN.md` §9) for the human's own match. `None` if the
/// player's club has a bye this matchday.
pub fn player_match_preview(
    state: &GameState,
) -> Option<crate::match_engine::MatchOutcome> {
    let md = state.current_matchday;
    let fixture = state
        .fixtures_of_matchday(md)
        .find(|f| f.home == state.player_club || f.away == state.player_club)?;
    let home_lineup = if fixture.home == state.player_club {
        effective_player_lineup(state)
    } else {
        ai_pick_lineup(&state.world, fixture.home)
    };
    let away_lineup = if fixture.away == state.player_club {
        effective_player_lineup(state)
    } else {
        ai_pick_lineup(&state.world, fixture.away)
    };
    let mut rng = derive_stream(state.seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
    Some(play_match(&state.world, &home_lineup, &away_lineup, &mut rng))
}

fn advance_matchday(state: &GameState) -> Vec<Event> {
    let md = state.current_matchday;
    let mut events = Vec::new();
    let mut new_results = state.results.clone();
    // The playing-time window accumulated so far, plus this matchday's matches —
    // exactly what `state.appearances_since_tick` will be right before any tick
    // this advance fires (DEVELOPMENT_MODEL.md §3).
    let mut window_apps = state.appearances_since_tick.clone();
    let mut window_club_matches = state.club_matches_since_tick.clone();

    for fixture in state.fixtures_of_matchday(md) {
        let home_lineup = if fixture.home == state.player_club {
            effective_player_lineup(state)
        } else {
            ai_pick_lineup(&state.world, fixture.home)
        };
        let away_lineup = if fixture.away == state.player_club {
            effective_player_lineup(state)
        } else {
            ai_pick_lineup(&state.world, fixture.away)
        };
        let mut rng = derive_stream(state.seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
        // The minute-by-minute stream is a Trace, not a fold input
        // (MATCH_MODEL.md §7) — only the score is recorded; it rides
        // alongside for live-viewing consumers (fforge-game's friendly
        // viewer) but is never persisted through the event log.
        let outcome = play_match(&state.world, &home_lineup, &away_lineup, &mut rng);
        let (hg, ag) = (outcome.home_goals, outcome.away_goals);
        new_results.insert(fixture.id, (hg, ag));
        let home_xi = home_lineup.players.to_vec();
        let away_xi = away_lineup.players.to_vec();
        for &pid in home_xi.iter().chain(&away_xi) {
            *window_apps.entry(pid).or_default() += 1;
        }
        *window_club_matches.entry(fixture.home).or_default() += 1;
        *window_club_matches.entry(fixture.away).or_default() += 1;
        events.push(Event::MatchPlayed {
            fixture: fixture.id,
            matchday: md,
            home_goals: hg,
            away_goals: ag,
            home_xi,
            away_xi,
        });
    }

    let new_date = state.date.add_days(7);
    events.push(Event::MatchdayAdvanced {
        matchday: md,
        new_date,
    });

    // Development: fire a tick for each 30-day boundary the advance crosses
    // (at most one, since a matchday step is 7 days). The window's appearances
    // include this matchday's matches, resolved above.
    events.extend(dev_ticks_between(
        state,
        state.date,
        new_date,
        &window_apps,
        &window_club_matches,
    ));

    if md == state.last_matchday {
        let table = league_table(&state.world, &state.schedule, &new_results);
        let champion = table.first().expect("non-empty league").club;
        events.push(Event::SeasonEnded { champion });
    }

    events
}

/// Begin the next season on the developed world (`Command::StartNextSeason`):
/// run the offseason development ticks across the summer break, then reset the
/// calendar with a fresh schedule. The world (with its developed attributes)
/// carries over.
fn start_next_season(state: &GameState) -> Vec<Event> {
    let new_start = next_season_start(state.date);
    // Offseason ticks: no matches in the gap, so the appearance window here is
    // just the tail accumulated since the last in-season tick (§3).
    let mut events = dev_ticks_between(
        state,
        state.date,
        new_start,
        &state.appearances_since_tick,
        &state.club_matches_since_tick,
    );
    let schedule = double_round_robin(&state.world.competition.clubs);
    events.push(Event::SeasonStarted {
        start_date: new_start,
        schedule,
    });
    events
}

/// Emit a `DevelopmentTick` (and, riding the same boundary, a `FinanceTick`,
/// `TRANSFER_MODEL.md` §4) for every 30-day period in `(old, new]`. A working
/// copy of the world is developed forward so successive ticks (only possible
/// across the multi-month offseason gap) compound correctly, exactly as the
/// fold will replay them. `first_apps`/`first_club_matches` are the
/// playing-time window for the *first* tick; later ticks in the same span see
/// an empty window (the fold resets it on each tick).
fn dev_ticks_between(
    state: &GameState,
    old_date: GameDate,
    new_date: GameDate,
    first_apps: &BTreeMap<PlayerId, u32>,
    first_club_matches: &BTreeMap<ClubId, u32>,
) -> Vec<Event> {
    let old_idx = period_index(old_date);
    let new_idx = period_index(new_date);
    if new_idx <= old_idx {
        return Vec::new();
    }
    let knobs = DevKnobs::default();
    let finance_knobs = FinanceKnobs::default();
    let mut work_world: World = state.world.clone();
    let empty_apps: BTreeMap<PlayerId, u32> = BTreeMap::new();
    let empty_club_matches: BTreeMap<ClubId, u32> = BTreeMap::new();

    let mut events = Vec::new();
    for (i, period) in ((old_idx + 1)..=new_idx).enumerate() {
        let tick_date = period_date(period);
        let (apps, club_matches) = if i == 0 {
            (first_apps, first_club_matches)
        } else {
            (&empty_apps, &empty_club_matches)
        };
        let changes = development::tick_changes(
            &work_world,
            state.seed,
            period,
            tick_date,
            apps,
            club_matches,
            &knobs,
        );
        // Compound onto the working copy so the next tick reads developed
        // attributes — the same order the fold applies them in.
        for step in &changes {
            development::apply_attr_step(&mut work_world, step);
        }
        events.push(Event::DevelopmentTick {
            date: tick_date,
            changes,
        });
        // FinanceTick rides the same boundary crossing: resolved
        // revenue-minus-wages deltas off the same working snapshot, so a
        // multi-period offseason gap prices each tick against a consistent
        // world rather than re-deriving from the pre-offseason one.
        events.push(Event::FinanceTick {
            date: tick_date,
            deltas: finance_deltas(&work_world, &finance_knobs),
        });
    }
    events
}

/// The next season-start date strictly after `date`: day 220 of this sim-year,
/// or next year's if already past it.
fn next_season_start(date: GameDate) -> GameDate {
    let candidate = GameDate::from_year_day(date.year(), SEASON_START_DOY);
    if candidate.days > date.days {
        candidate
    } else {
        GameDate::from_year_day(date.year() + 1, SEASON_START_DOY)
    }
}