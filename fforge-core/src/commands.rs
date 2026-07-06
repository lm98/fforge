//! `step(state, command) -> events` — the deterministic transition producer.
//!
//! This is the propose-then-validate gate in miniature: a `Command` is a
//! *proposal* (from the human today; from LLM agents in Phase 5), validation
//! happens here, and only resolved, validated values become events. `step`
//! never mutates state — callers apply the returned events through the fold.

use crate::event::Event;
use crate::match_engine::{ai_pick_lineup, lineup_strength, simulate_match};
use crate::rng::derive_stream;
use crate::state::{league_table, GameState};
use fforge_domain::{Lineup, PlayerId, FORMATIONS, XI};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Submit the human club's team sheet for the upcoming matchday.
    SubmitLineup(Lineup),
    /// Simulate every fixture of the current matchday and advance the calendar.
    AdvanceMatchday,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    SeasonOver,
    UnknownFormation(u8),
    DuplicatePlayers,
    NotInSquad(PlayerId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::SeasonOver => write!(f, "the season is over"),
            CommandError::UnknownFormation(i) => write!(f, "unknown formation index {i}"),
            CommandError::DuplicatePlayers => write!(f, "a player appears twice in the lineup"),
            CommandError::NotInSquad(p) => write!(f, "player {p} is not in your squad"),
        }
    }
}

/// Tag namespace for per-fixture RNG streams (see rng::derive_stream).
const FIXTURE_STREAM_NS: u64 = 0x4D41_5443_0000_0000; // "MATC"

pub fn step(state: &GameState, command: Command) -> Result<Vec<Event>, CommandError> {
    if state.season_over() {
        return Err(CommandError::SeasonOver);
    }
    match command {
        Command::SubmitLineup(lineup) => {
            validate_lineup(state, &lineup)?;
            Ok(vec![Event::LineupSubmitted {
                matchday: state.current_matchday,
                lineup,
            }])
        }
        Command::AdvanceMatchday => Ok(advance_matchday(state)),
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

fn advance_matchday(state: &GameState) -> Vec<Event> {
    let md = state.current_matchday;
    let mut events = Vec::new();
    let mut new_results = state.results.clone();

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
        let hs = lineup_strength(&state.world, &home_lineup);
        let as_ = lineup_strength(&state.world, &away_lineup);
        let mut rng = derive_stream(state.seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
        let (hg, ag) = simulate_match(hs, as_, &mut rng);
        new_results.insert(fixture.id, (hg, ag));
        events.push(Event::MatchPlayed {
            fixture: fixture.id,
            matchday: md,
            home_goals: hg,
            away_goals: ag,
        });
    }

    events.push(Event::MatchdayAdvanced {
        matchday: md,
        new_date: state.date.add_days(7),
    });

    if md == state.last_matchday {
        let table = league_table(&state.world, &state.schedule, &new_results);
        let champion = table.first().expect("non-empty league").club;
        events.push(Event::SeasonEnded { champion });
    }

    events
}