//! `step(state, command) -> events` — the deterministic transition producer.
//!
//! This is the propose-then-validate gate in miniature: a `Command` is a
//! *proposal* (from the human today; from LLM agents in Phase 5), validation
//! happens here, and only resolved, validated values become events. `step`
//! never mutates state — callers apply the returned events through the fold.

use crate::club_ai::{TransferDecision, UtilityKnobs};
use crate::development::{self, period_date, period_index, DevKnobs};
use crate::event::Event;
use crate::finance::{finance_deltas, FinanceKnobs};
use crate::market::{self, MarketKnobs};
use crate::match_engine::{ai_pick_lineup, play_match};
use crate::pool::{self, PoolKnobs};
use crate::rng::derive_stream;
use crate::schedule::double_round_robin;
use crate::state::{
    apply_finance_deltas, apply_player_retired, apply_transfer_completed, apply_youth_intake,
    league_table, GameState,
};
use crate::valuation::ValueKnobs;
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
    /// Submit the human club's transfer plan for the window currently open
    /// (`TRANSFER_MODEL.md` §10's pre-commitment model): validated here for
    /// shape only (targets exist, aren't already owned, prices aren't
    /// negative, sell-list entries are the club's own players), then
    /// replayed verbatim by `club_ai::RecordedPolicy` in every round of that
    /// window's clearing loop once it resolves. Affordability, squad
    /// bounds, and every other resolve-time stabilizer are checked later,
    /// inside the clearing loop itself, against whatever the world actually
    /// looks like when the window closes — not here.
    SubmitTransferDecision(Vec<TransferDecision>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    SeasonOver,
    SeasonNotOver,
    UnknownFormation(u8),
    DuplicatePlayers,
    NotInSquad(PlayerId),
    /// A transfer decision names a player who does not exist in the world.
    UnknownPlayer(PlayerId),
    /// A `Bid` names a player already on the submitting club's own books.
    AlreadyOwned(PlayerId),
    /// A `Bid`'s reservation price is negative.
    NegativePrice(PlayerId),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::SeasonOver => write!(f, "the season is over"),
            CommandError::SeasonNotOver => write!(f, "the season is not over yet"),
            CommandError::UnknownFormation(i) => write!(f, "unknown formation index {i}"),
            CommandError::DuplicatePlayers => write!(f, "a player appears twice in the lineup"),
            CommandError::NotInSquad(p) => write!(f, "player {p} is not in your squad"),
            CommandError::UnknownPlayer(p) => write!(f, "player {p} does not exist"),
            CommandError::AlreadyOwned(p) => write!(f, "player {p} is already on your books"),
            CommandError::NegativePrice(p) => {
                write!(f, "player {p}'s reservation price cannot be negative")
            }
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
                Command::SubmitTransferDecision(decisions) => {
                    validate_transfer_decisions(state, &decisions)?;
                    Ok(vec![Event::TransferDecisionSubmitted {
                        date: state.date,
                        club: state.player_club,
                        decisions,
                    }])
                }
                Command::StartNextSeason => unreachable!("handled above"),
            }
        }
    }
}

/// Submit-time shape validation for `Command::SubmitTransferDecision`
/// (`TRANSFER_MODEL.md` §10): every named player must exist; a `Bid`'s
/// target must not already be the submitting club's own player and its
/// price must not be negative; a `List`'s target must actually be the
/// submitting club's player. This is shape only — whether the plan is
/// actually affordable, or the target still available, is resolve-time
/// validation inside `market::resolve_window`'s clearing loop, checked
/// against the world as it is when the window closes, not as it was at
/// submission time.
fn validate_transfer_decisions(
    state: &GameState,
    decisions: &[TransferDecision],
) -> Result<(), CommandError> {
    let squad = &state.world.club(state.player_club).players;
    for d in decisions {
        match *d {
            TransferDecision::Bid { player, price, .. } => {
                if !state.world.players.contains_key(&player) {
                    return Err(CommandError::UnknownPlayer(player));
                }
                if squad.contains(&player) {
                    return Err(CommandError::AlreadyOwned(player));
                }
                if price.0 < 0 {
                    return Err(CommandError::NegativePrice(player));
                }
            }
            TransferDecision::List { player } => {
                if !state.world.players.contains_key(&player) {
                    return Err(CommandError::UnknownPlayer(player));
                }
                if !squad.contains(&player) {
                    return Err(CommandError::NotInSquad(player));
                }
            }
        }
    }
    Ok(())
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
    // this advance fires (DEVELOPMENT_MODEL.md §3). Minutes-valued since T4.
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
        // §2.8/T4: the window now accumulates *minutes*, not appearance
        // counts — `window_club_matches` stays the denominator's basis
        // (available minutes = 90 × matches).
        for &(pid, mins) in &outcome.minutes {
            *window_apps.entry(pid).or_default() += mins as u32;
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
            // The 2e boundary fields (MATCH_MODEL.md §12), passed through as
            // the engine resolved them — empty until the §14/§15/§18 models
            // land, recorded (not re-derived) once they do.
            injuries: outcome.injuries,
            cards: outcome.cards,
            ratings: outcome.ratings,
            minutes: outcome.minutes,
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
    let (dev_events, work_world) = dev_ticks_between(
        state,
        state.date,
        new_date,
        &window_apps,
        &window_club_matches,
    );
    events.extend(dev_events);

    // Transfer windows (`TRANSFER_MODEL.md` §7): resolved on the same
    // boundary-crossing mechanism, against the tick-compounded world above.
    events.extend(transfer_window_events(state, &work_world, state.date, new_date));

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
    // just the tail accumulated since the last in-season tick (§3). No window
    // boundary falls inside the offseason gap itself (§7: the summer window
    // closes *after* `SeasonStarted`), so no transfer resolution belongs here.
    let (mut events, _work_world) = dev_ticks_between(
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
/// an empty window (the fold resets it on each tick). Returns the compounded
/// working world alongside the events, so a caller resolving a transfer
/// window in the same advance (`transfer_window_events`) prices against this
/// tick's developed attributes and finance deltas, not the pre-tick world.
fn dev_ticks_between(
    state: &GameState,
    old_date: GameDate,
    new_date: GameDate,
    first_apps: &BTreeMap<PlayerId, u32>,
    first_club_matches: &BTreeMap<ClubId, u32>,
) -> (Vec<Event>, World) {
    let mut work_world: World = state.world.clone();
    let old_idx = period_index(old_date);
    let new_idx = period_index(new_date);
    if new_idx <= old_idx {
        return (Vec::new(), work_world);
    }
    let knobs = DevKnobs::default();
    let finance_knobs = FinanceKnobs::default();
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
        let deltas = finance_deltas(&work_world, &finance_knobs);
        apply_finance_deltas(&mut work_world, &deltas);
        events.push(Event::FinanceTick {
            date: tick_date,
            deltas,
        });
    }
    (events, work_world)
}

/// Emit a `TransferCompleted` for every transfer a window's clearing loop
/// completes, for each window boundary (§7) inside `(old_date, new_date]` —
/// and, for the summer window only, the pool's own events first (§8: youth
/// intake, then retirement). No new command: the market and the pool are
/// both *ticks*, exactly like development and finance, resolved here inside
/// `commands::step` when `AdvanceMatchday` crosses a window's close date.
/// `world` is the tick-compounded snapshot (`dev_ticks_between`'s return) so
/// the window prices against this advance's developed attributes and finance
/// deltas. `window_index` is even for the summer window, odd for winter
/// (`season_start.year() * 2` / `+ 1`) — that parity is what gates the pool
/// events to summer only.
fn transfer_window_events(state: &GameState, world: &World, old_date: GameDate, new_date: GameDate) -> Vec<Event> {
    let season_start = season_start_date(state);
    let candidates = [
        (
            season_start.year() as u64 * 2,
            market::summer_window_close(season_start),
        ),
        (
            season_start.year() as u64 * 2 + 1,
            market::winter_window_close(season_start, state.last_matchday),
        ),
    ];
    let mut crossed: Vec<(u64, GameDate)> = candidates
        .into_iter()
        .filter(|&(_, close)| old_date < close && close <= new_date)
        .collect();
    crossed.sort_by_key(|&(_, close)| close);
    if crossed.is_empty() {
        return Vec::new();
    }

    let dev_knobs = DevKnobs::default();
    let value_knobs = ValueKnobs::default();
    let utility_knobs = UtilityKnobs::default();
    let market_knobs = MarketKnobs::default();
    let pool_knobs = PoolKnobs::default();

    let mut work_world = world.clone();
    let mut events = Vec::new();
    for (window_index, close_date) in crossed {
        // Summer window only (§8): resolved against the same snapshot the
        // market prices off next, so new prospects are already on their
        // club's books and retirees are already excluded from valuation and
        // squad depth before the clearing loop runs.
        if window_index.is_multiple_of(2) {
            let pool_events = pool::summer_pool_events(
                &work_world,
                close_date,
                state.seed,
                window_index,
                &state.unsigned_since,
                &dev_knobs,
                &pool_knobs,
                utility_knobs.squad_max,
            );
            for e in &pool_events {
                match e {
                    Event::YouthIntake { club, players, .. } => {
                        apply_youth_intake(&mut work_world, *club, players);
                    }
                    Event::PlayerRetired { player, .. } => {
                        apply_player_retired(&mut work_world, *player);
                    }
                    _ => unreachable!("summer_pool_events only produces YouthIntake/PlayerRetired"),
                }
            }
            events.extend(pool_events);
        }

        let outcome = market::resolve_window(
            &work_world,
            close_date,
            state.seed,
            window_index,
            &dev_knobs,
            &value_knobs,
            &utility_knobs,
            &market_knobs,
            Some(state.player_club),
            &state.pending_transfer_decisions,
        );
        for t in &outcome.transfers {
            apply_transfer_completed(&mut work_world, t.player, t.from, t.to, t.fee, t.contract);
            events.push(Event::TransferCompleted {
                date: close_date,
                player: t.player,
                from: t.from,
                to: t.to,
                fee: t.fee,
                contract: t.contract,
            });
        }
        // Emitted unconditionally, even for a window that clears zero
        // transfers (§10's pre-commitment model): the fold's only reliable
        // signal to expire a human plan that was good for *this* window,
        // not the next one it was never submitted for.
        events.push(Event::TransferWindowClosed {
            date: close_date,
            window_index,
        });
    }
    events
}

/// The current season's start date, derived — never stored — from
/// `state.date` and `state.current_matchday`: each matchday step advances
/// the calendar exactly 7 days from the season's kickoff
/// (`commands::advance_matchday`), so `date - (current_matchday - 1) * 7`
/// recovers it without a dedicated field.
fn season_start_date(state: &GameState) -> GameDate {
    state.date.add_days(-((state.current_matchday as i64 - 1) * 7))
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

#[cfg(test)]
mod transfer_decision_tests {
    //! `TRANSFER_MODEL.md` §10's pre-commitment model:
    //! `Command::SubmitTransferDecision`'s submit-time shape validation and
    //! its threading through the real `AdvanceMatchday` pipeline.

    use super::*;
    use crate::worldgen::{generate, WorldGenConfig};
    use fforge_domain::{Money, Role};

    fn base_log(seed: u64) -> (Vec<Event>, GameState) {
        let (world, schedule, start_date) = generate(seed, &WorldGenConfig::default());
        let event = Event::GameStarted {
            seed,
            start_date,
            player_club: ClubId(0),
            world,
            schedule,
        };
        let state = GameState::replay(std::slice::from_ref(&event));
        (vec![event], state)
    }

    /// `step` the command, fold its events into `state`, and append them to
    /// `log` — the same append-fold-notify sequence `Session::execute`
    /// performs, reproduced by hand since `commands` cannot depend on
    /// `session` (the dependency runs the other way).
    fn drive(state: &mut GameState, log: &mut Vec<Event>, command: Command) {
        let events = step(state, command).expect("valid command");
        for e in &events {
            state.apply(e);
        }
        log.extend(events);
    }

    #[test]
    fn rejects_a_bid_on_an_unknown_player() {
        let (_, state) = base_log(1);
        let decisions = vec![TransferDecision::List {
            player: PlayerId(999_999),
        }];
        assert_eq!(
            step(&state, Command::SubmitTransferDecision(decisions)),
            Err(CommandError::UnknownPlayer(PlayerId(999_999)))
        );
    }

    #[test]
    fn rejects_bidding_on_your_own_player() {
        let (_, state) = base_log(2);
        let own = state.world.club(state.player_club).players[0];
        let decisions = vec![TransferDecision::Bid {
            player: own,
            from: None,
            role: Role::St,
            price: Money(1),
        }];
        assert_eq!(
            step(&state, Command::SubmitTransferDecision(decisions)),
            Err(CommandError::AlreadyOwned(own))
        );
    }

    #[test]
    fn rejects_a_negative_reservation_price() {
        let (_, state) = base_log(3);
        let other_club = state
            .world
            .competition
            .clubs
            .iter()
            .copied()
            .find(|&c| c != state.player_club)
            .expect("more than one club");
        let target = state.world.club(other_club).players[0];
        let decisions = vec![TransferDecision::Bid {
            player: target,
            from: Some(other_club),
            role: Role::St,
            price: Money(-1),
        }];
        assert_eq!(
            step(&state, Command::SubmitTransferDecision(decisions)),
            Err(CommandError::NegativePrice(target))
        );
    }

    #[test]
    fn rejects_listing_a_player_not_in_your_squad() {
        let (_, state) = base_log(4);
        let other_club = state
            .world
            .competition
            .clubs
            .iter()
            .copied()
            .find(|&c| c != state.player_club)
            .expect("more than one club");
        let not_mine = state.world.club(other_club).players[0];
        let decisions = vec![TransferDecision::List { player: not_mine }];
        assert_eq!(
            step(&state, Command::SubmitTransferDecision(decisions)),
            Err(CommandError::NotInSquad(not_mine))
        );
    }

    #[test]
    fn a_valid_plan_is_recorded_pending_and_cleared_once_its_window_closes() {
        let (mut log, mut state) = base_log(5);
        let mine = state.world.club(state.player_club).players[0];
        let decisions = vec![TransferDecision::List { player: mine }];

        let submitted_at = state.date;
        drive(
            &mut state,
            &mut log,
            Command::SubmitTransferDecision(decisions.clone()),
        );
        assert_eq!(
            log.last(),
            Some(&Event::TransferDecisionSubmitted {
                date: submitted_at,
                club: state.player_club,
                decisions: decisions.clone(),
            })
        );
        assert_eq!(state.pending_transfer_decisions, decisions);

        loop {
            let before = log.len();
            drive(&mut state, &mut log, Command::AdvanceMatchday);
            let closed = log[before..]
                .iter()
                .any(|e| matches!(e, Event::TransferWindowClosed { .. }));
            if closed {
                break;
            }
            assert!(
                !state.season_over(),
                "season ended before any window closed"
            );
        }
        assert!(
            state.pending_transfer_decisions.is_empty(),
            "a window closing must clear the pending plan"
        );
    }

    #[test]
    fn replay_reconstructs_identical_state_across_a_submitted_plan_and_window_close() {
        let (mut log, mut state) = base_log(6);
        let mine = state.world.club(state.player_club).players[0];
        drive(
            &mut state,
            &mut log,
            Command::SubmitTransferDecision(vec![TransferDecision::List { player: mine }]),
        );
        loop {
            let before = log.len();
            drive(&mut state, &mut log, Command::AdvanceMatchday);
            let closed = log[before..]
                .iter()
                .any(|e| matches!(e, Event::TransferWindowClosed { .. }));
            if closed || state.season_over() {
                break;
            }
        }
        let replayed = GameState::replay(&log);
        assert_eq!(
            state, replayed,
            "replay must reconstruct identical state across §10's new events"
        );
        // Determinism: replaying the same log twice must agree, too.
        assert_eq!(GameState::replay(&log), GameState::replay(&log));
    }

    #[test]
    fn human_club_that_submits_nothing_completes_no_transfers_across_a_real_season() {
        let (mut log, mut state) = base_log(8);
        while !state.season_over() {
            drive(&mut state, &mut log, Command::AdvanceMatchday);
        }
        let touched_human = log.iter().any(|e| {
            matches!(
                e,
                Event::TransferCompleted { from, to, .. }
                    if *to == state.player_club || *from == Some(state.player_club)
            )
        });
        assert!(
            !touched_human,
            "a human club that never submits a transfer plan must complete no transfers of its own"
        );
    }
}
