//! `GameState` — a pure fold over the event log.
//!
//! `apply` is the fold step: no RNG, no clock, no I/O, no engine calls. All
//! of those live in `commands::step`, which *produces* events; this module
//! only consumes them. `replay(events)` is therefore save-loading, bug
//! reproduction, and (later) counterfactual branch points, all in one.

use crate::development::apply_attr_step;
use crate::event::Event;
use fforge_domain::{ClubId, Fixture, FixtureId, GameDate, Lineup, PlayerId, World};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct GameState {
    pub seed: u64,
    /// The world snapshot — **mutated by development** (`DevelopmentTick` folds
    /// attribute steps into it). Its `GameStarted` form is only the starting
    /// point; current attributes are the fold's running result.
    pub world: World,
    pub player_club: ClubId,
    pub schedule: Vec<Fixture>,
    pub date: GameDate,
    /// 1-based matchday about to be played next. `last_matchday + 1` once over.
    pub current_matchday: u8,
    pub last_matchday: u8,
    pub results: BTreeMap<FixtureId, (u8, u8)>,
    /// The human's submitted lineup for the upcoming matchday, if any.
    pub pending_lineup: Option<Lineup>,
    /// The lineup most recently used, reused as the default next time.
    pub last_lineup: Option<Lineup>,
    pub champion: Option<ClubId>,
    /// Appearances accrued since the last `DevelopmentTick` — the playing-time
    /// window feeding development (`DEVELOPMENT_MODEL.md` §3). Folded from each
    /// `MatchPlayed`'s XIs; reset on each tick.
    pub appearances_since_tick: BTreeMap<PlayerId, u32>,
    /// Matches each club played in the current development window — the
    /// denominator for the appeared/benched/absent share. Reset on each tick.
    pub club_matches_since_tick: BTreeMap<ClubId, u32>,
}

impl GameState {
    /// Rebuild state from the log. Panics on a malformed log (an empty log or
    /// one not starting with `GameStarted`) — that is a corrupted save, not a
    /// recoverable game situation.
    pub fn replay(events: &[Event]) -> GameState {
        let mut iter = events.iter();
        let first = iter.next().expect("event log is empty");
        let mut state = match first {
            Event::GameStarted {
                seed,
                start_date,
                player_club,
                world,
                schedule,
            } => {
                let last_matchday = schedule.iter().map(|f| f.matchday).max().unwrap_or(0);
                GameState {
                    seed: *seed,
                    world: world.clone(),
                    player_club: *player_club,
                    schedule: schedule.clone(),
                    date: *start_date,
                    current_matchday: 1,
                    last_matchday,
                    results: BTreeMap::new(),
                    pending_lineup: None,
                    last_lineup: None,
                    champion: None,
                    appearances_since_tick: BTreeMap::new(),
                    club_matches_since_tick: BTreeMap::new(),
                }
            }
            other => panic!("event log must start with GameStarted, found {other:?}"),
        };
        for event in iter {
            state.apply(event);
        }
        state
    }

    /// The fold step. Total over post-`GameStarted` events.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::GameStarted { .. } => {
                panic!("GameStarted may only appear as the first event")
            }
            Event::LineupSubmitted { lineup, .. } => {
                self.pending_lineup = Some(lineup.clone());
            }
            Event::MatchPlayed {
                fixture,
                home_goals,
                away_goals,
                home_xi,
                away_xi,
                ..
            } => {
                self.results.insert(*fixture, (*home_goals, *away_goals));
                // Accrue the playing-time window (DEVELOPMENT_MODEL.md §3).
                for &pid in home_xi.iter().chain(away_xi) {
                    *self.appearances_since_tick.entry(pid).or_default() += 1;
                }
                if let Some(fx) = self.schedule.iter().find(|f| f.id == *fixture) {
                    *self.club_matches_since_tick.entry(fx.home).or_default() += 1;
                    *self.club_matches_since_tick.entry(fx.away).or_default() += 1;
                }
            }
            Event::MatchdayAdvanced { new_date, .. } => {
                if let Some(lineup) = self.pending_lineup.take() {
                    self.last_lineup = Some(lineup);
                }
                self.date = *new_date;
                self.current_matchday += 1;
            }
            Event::DevelopmentTick { changes, .. } => {
                // Pure integer add, clamped — no RNG, no growth math (invariant
                // 2). All of that produced these deltas in `commands::step`.
                for step in changes {
                    apply_attr_step(&mut self.world, step);
                }
                // The window resets: appearances are per-tick.
                self.appearances_since_tick.clear();
                self.club_matches_since_tick.clear();
            }
            Event::SeasonEnded { champion } => {
                self.champion = Some(*champion);
            }
            Event::SeasonStarted {
                start_date,
                schedule,
            } => {
                self.schedule = schedule.clone();
                self.last_matchday = schedule.iter().map(|f| f.matchday).max().unwrap_or(0);
                self.date = *start_date;
                self.current_matchday = 1;
                self.results.clear();
                self.pending_lineup = None;
                self.champion = None;
                // `world` (developed) and `last_lineup` carry over; the
                // appearance window is managed by the offseason ticks that
                // precede this event.
            }
        }
    }

    pub fn season_over(&self) -> bool {
        self.champion.is_some()
    }

    pub fn fixtures_of_matchday(&self, matchday: u8) -> impl Iterator<Item = &Fixture> {
        self.schedule.iter().filter(move |f| f.matchday == matchday)
    }
}

/// One league-table row. The table is **derived, never stored** — same
/// philosophy as CA: results are the single source of truth, the table is a
/// view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub club: ClubId,
    pub played: u32,
    pub won: u32,
    pub drawn: u32,
    pub lost: u32,
    pub goals_for: u32,
    pub goals_against: u32,
}

impl TableRow {
    pub fn points(&self) -> u32 {
        self.won * 3 + self.drawn
    }
    pub fn goal_diff(&self) -> i64 {
        self.goals_for as i64 - self.goals_against as i64
    }
}

/// Standings from an arbitrary results map (callers may merge not-yet-applied
/// events in). Sort: points, goal difference, goals for, then club name —
/// fully deterministic.
pub fn league_table(
    world: &World,
    schedule: &[Fixture],
    results: &BTreeMap<FixtureId, (u8, u8)>,
) -> Vec<TableRow> {
    let mut rows: BTreeMap<ClubId, TableRow> = world
        .competition
        .clubs
        .iter()
        .map(|&c| {
            (
                c,
                TableRow {
                    club: c,
                    played: 0,
                    won: 0,
                    drawn: 0,
                    lost: 0,
                    goals_for: 0,
                    goals_against: 0,
                },
            )
        })
        .collect();

    for fixture in schedule {
        let Some(&(hg, ag)) = results.get(&fixture.id) else {
            continue;
        };
        {
            let home = rows.get_mut(&fixture.home).expect("home club in table");
            home.played += 1;
            home.goals_for += hg as u32;
            home.goals_against += ag as u32;
            match hg.cmp(&ag) {
                std::cmp::Ordering::Greater => home.won += 1,
                std::cmp::Ordering::Equal => home.drawn += 1,
                std::cmp::Ordering::Less => home.lost += 1,
            }
        }
        {
            let away = rows.get_mut(&fixture.away).expect("away club in table");
            away.played += 1;
            away.goals_for += ag as u32;
            away.goals_against += hg as u32;
            match ag.cmp(&hg) {
                std::cmp::Ordering::Greater => away.won += 1,
                std::cmp::Ordering::Equal => away.drawn += 1,
                std::cmp::Ordering::Less => away.lost += 1,
            }
        }
    }

    let mut table: Vec<TableRow> = rows.into_values().collect();
    table.sort_by(|a, b| {
        b.points()
            .cmp(&a.points())
            .then(b.goal_diff().cmp(&a.goal_diff()))
            .then(b.goals_for.cmp(&a.goals_for))
            .then(world.club(a.club).name.cmp(&world.club(b.club).name))
    });
    table
}
