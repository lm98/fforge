//! `GameState` — a pure fold over the event log.
//!
//! `apply` is the fold step: no RNG, no clock, no I/O, no engine calls. All
//! of those live in `commands::step`, which *produces* events; this module
//! only consumes them. `replay(events)` is therefore save-loading, bug
//! reproduction, and (later) counterfactual branch points, all in one.

use crate::development::apply_attr_step;
use crate::event::Event;
use fforge_domain::{
    ClubId, Contract, Fixture, FixtureId, GameDate, Lineup, Money, Player, PlayerId, World,
};
use std::collections::BTreeMap;

/// Move `player` from `from` (if any) to `to`, exchange `fee` between their
/// balances, and install `contract` — the resolved effect of a completed
/// transfer (`TRANSFER_MODEL.md` §4). Shared by the `TransferCompleted` fold
/// arm below and `market::resolve_window`'s per-round working-world update,
/// so there is exactly one place this mutation is encoded, never two that
/// could drift apart.
pub(crate) fn apply_transfer_completed(
    world: &mut World,
    player: PlayerId,
    from: Option<ClubId>,
    to: ClubId,
    fee: Money,
    contract: Contract,
) {
    if let Some(from_club) = from
        && let Some(club) = world.clubs.get_mut(&from_club)
    {
        club.players.retain(|&p| p != player);
        club.finances.balance = Money(club.finances.balance.0 + fee.0);
    }
    if let Some(club) = world.clubs.get_mut(&to) {
        if !club.players.contains(&player) {
            club.players.push(player);
            club.players.sort();
        }
        club.finances.balance = Money(club.finances.balance.0 - fee.0);
    }
    if let Some(p) = world.players.get_mut(&player) {
        p.contract = Some(contract);
    }
}

/// Integer-add each resolved per-club delta to `Club.finances.balance`
/// (`TRANSFER_MODEL.md` §4). Shared by the `FinanceTick` fold arm and
/// `commands::dev_ticks_between`'s working-world compounding, so a transfer
/// window resolving in the same advance sees this tick's cash flow already
/// applied.
pub(crate) fn apply_finance_deltas(world: &mut World, deltas: &[(ClubId, Money)]) {
    for &(club, delta) in deltas {
        if let Some(c) = world.clubs.get_mut(&club) {
            c.finances.balance = Money(c.finances.balance.0 + delta.0);
        }
    }
}

/// Insert `players` into `club`'s roster and `World.players`
/// (`TRANSFER_MODEL.md` §8.1, §4) — the resolved effect of a `YouthIntake`.
/// Shared by the `YouthIntake` fold arm and `commands::transfer_window_events`'s
/// working-world update (the same one-encoding pattern as
/// `apply_transfer_completed`).
pub(crate) fn apply_youth_intake(world: &mut World, club: ClubId, players: &[Player]) {
    if let Some(c) = world.clubs.get_mut(&club) {
        for p in players {
            if !c.players.contains(&p.id) {
                c.players.push(p.id);
            }
        }
        c.players.sort();
    }
    for p in players {
        world.players.insert(p.id, p.clone());
    }
}

/// Remove `player` from every roster and mark him retired
/// (`TRANSFER_MODEL.md` §8.2, §4) — the resolved effect of a `PlayerRetired`.
/// Shared by the `PlayerRetired` fold arm and `commands::transfer_window_events`'s
/// working-world update.
pub(crate) fn apply_player_retired(world: &mut World, player: PlayerId) {
    if let Some(cid) = world.club_of(player)
        && let Some(c) = world.clubs.get_mut(&cid)
    {
        c.players.retain(|p| p != &player);
    }
    if let Some(p) = world.players.get_mut(&player) {
        p.contract = None;
        p.retired = true;
    }
}

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
    /// The date each currently contract-less player last lost his contract
    /// (`Event::PlayerReleased`) — `pool::retirements`' "gone a full season
    /// unsigned" reading (`TRANSFER_MODEL.md` §8.2). Cleared when the player
    /// signs again (`TransferCompleted`) or retires (`PlayerRetired`).
    pub unsigned_since: BTreeMap<PlayerId, GameDate>,
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
                    unsigned_since: BTreeMap::new(),
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
            // The following six fold arms are the `TRANSFER_MODEL.md` §4
            // event-log seam: pure integer operations only — no RNG, no
            // math beyond addition, no engine calls. Rosters are kept
            // sorted after every mutation so replay-path equality holds.
            Event::TransferCompleted {
                player,
                from,
                to,
                fee,
                contract,
                ..
            } => {
                apply_transfer_completed(&mut self.world, *player, *from, *to, *fee, *contract);
                self.unsigned_since.remove(player);
            }
            Event::PlayerReleased {
                player, club, date,
            } => {
                if let Some(c) = self.world.clubs.get_mut(club) {
                    c.players.retain(|p| p != player);
                }
                if let Some(p) = self.world.players.get_mut(player) {
                    p.contract = None;
                }
                self.unsigned_since.insert(*player, *date);
            }
            Event::ContractRenewed {
                player, contract, ..
            } => {
                if let Some(p) = self.world.players.get_mut(player) {
                    p.contract = Some(*contract);
                }
            }
            Event::YouthIntake { club, players, .. } => {
                apply_youth_intake(&mut self.world, *club, players);
            }
            Event::PlayerRetired { player, .. } => {
                apply_player_retired(&mut self.world, *player);
                self.unsigned_since.remove(player);
            }
            Event::FinanceTick { deltas, .. } => {
                apply_finance_deltas(&mut self.world, deltas);
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

#[cfg(test)]
mod transfer_event_tests {
    //! `TRANSFER_MODEL.md` §4's event-log seam: the six new fold arms above.
    //! Events and fold only (this task's scope fence) — no decision logic, no
    //! clearing loop, no valuation calls, so every event here is hand-built
    //! rather than produced by a command.

    use super::*;
    use crate::event::Event;
    use crate::session::{load_log, save_log};
    use crate::worldgen::{generate, WorldGenConfig};
    use fforge_domain::{Contract, Money};

    /// A one-event log (`GameStarted` on a freshly generated world) plus the
    /// state it folds to — the common starting point for every test below.
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

    #[test]
    fn transfer_completed_moves_the_player_and_balances_exactly_once() {
        let (mut log, state) = base_log(1);
        let from_club = ClubId(0);
        let to_club = ClubId(1);
        let player = state.world.club(from_club).players[0];
        let fee = Money(4_500_000);
        let new_contract = Contract {
            wage: Money(900_000),
            expires: GameDate::from_year_day(2031, 100),
        };
        let before_from_balance = state.world.club(from_club).finances.balance;
        let before_to_balance = state.world.club(to_club).finances.balance;

        log.push(Event::TransferCompleted {
            date: state.date,
            player,
            from: Some(from_club),
            to: to_club,
            fee,
            contract: new_contract,
        });

        let replayed = GameState::replay(&log);
        let from = replayed.world.club(from_club);
        let to = replayed.world.club(to_club);

        assert!(
            !from.players.contains(&player),
            "player must leave the selling club"
        );
        assert_eq!(
            to.players.iter().filter(|&&p| p == player).count(),
            1,
            "player must join the buying club exactly once"
        );
        assert_eq!(
            from.finances.balance.0,
            before_from_balance.0 + fee.0,
            "selling club must be credited exactly the fee"
        );
        assert_eq!(
            to.finances.balance.0,
            before_to_balance.0 - fee.0,
            "buying club must be debited exactly the fee"
        );
        assert_eq!(replayed.world.player(player).contract, Some(new_contract));
        assert_eq!(replayed.world.club_of(player), Some(to_club));
        assert!(
            from.players.windows(2).all(|w| w[0] < w[1]),
            "seller roster must stay sorted"
        );
        assert!(
            to.players.windows(2).all(|w| w[0] < w[1]),
            "buyer roster must stay sorted"
        );

        // Idempotent under replay: replaying the same log from scratch, twice,
        // reproduces byte-identical state — the transfer folds exactly once
        // per replay, never accumulated across replays.
        let replayed_again = GameState::replay(&log);
        assert_eq!(replayed, replayed_again);
    }

    #[test]
    fn free_agent_signing_has_no_selling_club_to_credit() {
        let (mut log, state) = base_log(2);
        let club = ClubId(0);
        let player = state.world.club(club).players[0];
        log.push(Event::PlayerReleased {
            date: state.date,
            player,
            club,
        });
        let contract = Contract {
            wage: Money(250_000),
            expires: GameDate::from_year_day(2028, 1),
        };
        log.push(Event::TransferCompleted {
            date: state.date,
            player,
            from: None,
            to: club,
            fee: Money(0),
            contract,
        });

        let replayed = GameState::replay(&log);
        assert_eq!(
            replayed
                .world
                .club(club)
                .players
                .iter()
                .filter(|&&p| p == player)
                .count(),
            1
        );
        assert_eq!(replayed.world.player(player).contract, Some(contract));
    }

    #[test]
    fn replay_is_deterministic_across_the_new_events() {
        let (mut log, state) = base_log(11);
        let from_club = ClubId(0);
        let to_club = ClubId(1);
        let player = state.world.club(from_club).players[2];
        log.push(Event::TransferCompleted {
            date: state.date,
            player,
            from: Some(from_club),
            to: to_club,
            fee: Money(1_200_000),
            contract: Contract {
                wage: Money(300_000),
                expires: GameDate::from_year_day(2030, 50),
            },
        });
        let renewed_player = state.world.club(to_club).players[0];
        log.push(Event::ContractRenewed {
            date: state.date,
            player: renewed_player,
            club: to_club,
            contract: Contract {
                wage: Money(500_000),
                expires: GameDate::from_year_day(2032, 10),
            },
        });
        log.push(Event::FinanceTick {
            date: state.date,
            deltas: vec![(ClubId(0), Money(50_000)), (ClubId(1), Money(-20_000))],
        });

        let a = GameState::replay(&log);
        let b = GameState::replay(&log);
        assert_eq!(
            a, b,
            "replay must be deterministic across the new event kinds"
        );
    }

    #[test]
    fn rosters_stay_sorted_and_within_bounds_after_pool_events() {
        let (mut log, state) = base_log(3);
        let club = ClubId(2);
        let squad_before = state.world.club(club).players.len();

        // Youth intake: two recruits cloned from an existing squad member as a
        // stand-in for `worldgen::gen_player`'s youth cohort (out of this
        // task's scope — only the event/fold mechanics are under test here).
        let template = state.world.club_players(club).next().unwrap().clone();
        let mut recruit_a = template.clone();
        recruit_a.id = PlayerId(100_000);
        recruit_a.contract = None;
        let mut recruit_b = template;
        recruit_b.id = PlayerId(100_001);
        recruit_b.contract = None;
        log.push(Event::YouthIntake {
            date: state.date,
            club,
            players: vec![recruit_a.clone(), recruit_b.clone()],
        });

        // A retirement and a release, each removing one existing player.
        let retiring = state.world.club(club).players[0];
        let released = state.world.club(club).players[1];
        log.push(Event::PlayerRetired {
            date: state.date,
            player: retiring,
        });
        log.push(Event::PlayerReleased {
            date: state.date,
            player: released,
            club,
        });

        let replayed = GameState::replay(&log);
        let roster = &replayed.world.club(club).players;

        assert!(
            roster.windows(2).all(|w| w[0] < w[1]),
            "roster must stay sorted: {roster:?}"
        );
        assert!(
            !roster.contains(&retiring),
            "retired player must leave the roster"
        );
        assert!(
            !roster.contains(&released),
            "released player must leave the roster"
        );
        assert!(roster.contains(&recruit_a.id) && roster.contains(&recruit_b.id));
        assert_eq!(
            roster.len(),
            squad_before - 2 + 2,
            "net roster size must reflect the two exits and two intakes"
        );
        assert!(
            (10..=40).contains(&roster.len()),
            "squad size should stay within a sane bound, got {}",
            roster.len()
        );

        assert!(replayed.world.player(retiring).contract.is_none());
        assert!(
            replayed.world.player(retiring).retired,
            "PlayerRetired must mark the player retired"
        );
        assert!(replayed.world.player(released).contract.is_none());
        assert!(
            !replayed.world.player(released).retired,
            "a release is not a retirement"
        );
        assert_eq!(replayed.world.player(recruit_a.id).contract, None);
    }

    #[test]
    fn save_load_round_trips_the_new_events() {
        let (mut log, state) = base_log(21);
        let club = ClubId(4);
        let roster = state.world.club(club).players.clone();
        log.push(Event::PlayerReleased {
            date: state.date,
            player: roster[0],
            club,
        });
        log.push(Event::ContractRenewed {
            date: state.date,
            player: roster[1],
            club,
            contract: Contract {
                wage: Money(750_000),
                expires: GameDate::from_year_day(2029, 300),
            },
        });
        let mut recruit = state.world.club_players(club).nth(2).unwrap().clone();
        recruit.id = PlayerId(200_000);
        log.push(Event::YouthIntake {
            date: state.date,
            club,
            players: vec![recruit.clone()],
        });
        log.push(Event::PlayerRetired {
            date: state.date,
            player: roster[3],
        });
        log.push(Event::FinanceTick {
            date: state.date,
            deltas: vec![(club, Money(12_345))],
        });
        log.push(Event::TransferCompleted {
            date: state.date,
            player: recruit.id,
            from: Some(club),
            to: ClubId(5),
            fee: Money(2_000_000),
            contract: Contract {
                wage: Money(600_000),
                expires: GameDate::from_year_day(2031, 1),
            },
        });

        let dir = std::env::temp_dir().join("fforge-test-transfer-events");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.fml");
        save_log(&path, &log).unwrap();
        let loaded = load_log(&path).unwrap();

        assert_eq!(log, loaded, "log must round-trip through JSON exactly");
        assert_eq!(GameState::replay(&log), GameState::replay(&loaded));
    }
}
