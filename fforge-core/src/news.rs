//! The news/notification Trace (`DESIGN.md` §9's evaluation spine, Phase-5's
//! journalist raw material): a structured, replay-safe inbox derived *from*
//! the event log and live state, never stored *in* either.
//!
//! **Trace, not Event.** A `NewsItem` is a fact *about* the recorded game,
//! never a fact *of* it — nothing here ever folds into `GameState`, and
//! nothing here is itself persisted. This is exactly the reasoning that
//! keeps `MatchOutcome.stream` out of the fold (`MATCH_MODEL.md` §7) and
//! `WindowOutcome`'s rejected bids out of it (`TRANSFER_MODEL.md` §4):
//! deriving the inbox from what is *already* recorded means replay
//! reproduces it for free, and the log never bloats with presentation.
//!
//! **Structured, not pre-rendered — the load-bearing choice.** `NewsKind`
//! carries typed IDs (`PlayerId`, `ClubId`, `Money`, ...), never strings.
//! `TemplateRenderer` (this module, deterministic, zero LLM) and a future
//! Phase-5 journalist renderer are peers reading the *same* structure, not a
//! thing and its patch — `DESIGN.md`'s "templated fallbacks, LLM optional"
//! showing up as architecture, not just a promise.
//!
//! **Two categories of news, one `EventObserver`.** [`NewsObserver`]
//! implements `EventObserver` for event-derived news (transfer completed,
//! match result, youth intake, retirement) — a pure function of the bare
//! `Event` stream, exactly like `SeasonTelemetry`. State-condition news
//! (contract expiring, balance below zero, no cover at a role) does not fit
//! that signature, which sees events and never state — the same wall
//! `market::calibrate::MarketTelemetry` hit and resolved by taking
//! `record_season_end(&world, ...)` from outside the trait. [`NewsObserver::check_conditions`]
//! does the same here, reading `&GameState` directly; nothing widens the
//! Phase-1 `EventObserver` trait itself.
//!
//! **A third, narrower path for `WindowOutcome`'s Trace.** Bids rejected
//! fit neither category: a rejected bid changes nothing in `GameState` (so
//! `check_conditions` can never see it) and is never itself an `Event` (so
//! `on_event` can never see it either — `market.rs` is explicit that
//! `rejected_bids` is "kept, but never fed to the fold"). [`NewsObserver::observe_rejected_bids`]
//! consumes that Trace directly, the same way `fforge-game` re-derives
//! `MatchOutcome`'s commentary stream live (`commands::player_match_preview`)
//! rather than persisting it: nothing about a rejected bid is ever a fact of
//! `World`, so nothing here is lost that a replay was ever meant to
//! reproduce — a cold replay simply never re-populates that slice of the
//! inbox, exactly the asymmetry match commentary already has.
//!
//! **Provenance is structural, not an afterthought.** Every `NewsItem`
//! carries `sources: Vec<EventRef>` — indices into the log the item was
//! derived from. Event-derived items source themselves directly (the event
//! being processed IS the source); state-condition items source the most
//! recent event `NewsObserver` has seen that plausibly caused the condition
//! (a squad-membership or finance-affecting event for that club, a
//! contract-setting event for that player) via small incrementally
//! maintained indices, never widening `check_conditions`'s own signature.
//!
//! **Salience and audience keep the inbox usable.** 380 matches a season
//! plus transfers plus monthly ticks is an unusable firehose unfiltered;
//! every item is tagged with both, and [`NewsObserver::inbox`] is the
//! filtered read path a consumer actually wants.

use crate::event::Event;
use crate::market::{RejectReason, RejectedBid};
use crate::observer::EventObserver;
use crate::state::GameState;
use fforge_domain::{
    ClubId, FixtureId, GameDate, Money, PlayerId, ROLE_WEIGHTS, Role, World, best_role,
};
use std::collections::{BTreeMap, BTreeSet};

/// A contract term expiring at or inside this many days out is newsworthy
/// (category 2's "contract expiring within 30 days").
const CONTRACT_EXPIRY_WARNING_DAYS: i64 = 30;

/// An index into the append-only `Session::log` — a `NewsItem`'s provenance.
/// Never dereferenced by this module; a consumer (B5.3's validation gate, a
/// CLI's "why am I seeing this" drill-down) resolves it against the log it
/// already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventRef(pub usize);

/// Who a `NewsItem` is naturally addressed to. `League` items are relevant
/// to anyone; `Club`/`Player` items are scoped to one participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Audience {
    League,
    Club(ClubId),
    Player(PlayerId),
}

/// The typed payload every `NewsItem` carries — IDs and resolved values,
/// never strings (module docs: "structured, not pre-rendered — the
/// load-bearing choice"). `TemplateRenderer::render` resolves names against
/// a `World`; a future journalist renderer authors prose from these same
/// fields rather than rewriting someone else's.
#[derive(Debug, Clone, PartialEq)]
pub enum NewsKind {
    MatchResult {
        fixture: FixtureId,
        home: ClubId,
        away: ClubId,
        home_goals: u8,
        away_goals: u8,
    },
    TransferCompleted {
        player: PlayerId,
        from: Option<ClubId>,
        to: ClubId,
        fee: Money,
    },
    /// Sourced from `WindowOutcome`'s Trace, not the event log — see the
    /// module docs and [`NewsObserver::observe_rejected_bids`].
    BidRejected {
        player: PlayerId,
        from: Option<ClubId>,
        bidder: ClubId,
        price: Money,
        reason: RejectReason,
    },
    YouthIntake {
        club: ClubId,
        players: Vec<PlayerId>,
    },
    Retirement {
        player: PlayerId,
    },
    ContractExpiring {
        player: PlayerId,
        club: ClubId,
        days_remaining: i64,
    },
    FinancialWarning {
        club: ClubId,
        balance: Money,
    },
    RoleCoverageGap {
        club: ClubId,
        role: Role,
    },
}

/// One notification. `salience` is 0 (irrelevant) to 100 (must-see); v1's
/// scale is deliberately coarse (see `NewsObserver`'s per-kind constants) —
/// good enough for threshold filtering, not a promise of fine-grained
/// ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct NewsItem {
    pub date: GameDate,
    pub kind: NewsKind,
    pub sources: Vec<EventRef>,
    pub salience: u8,
    pub audience: Audience,
}

/// `NewsItem` -> presentation string. `TemplateRenderer` (below) is the v1,
/// deterministic, zero-LLM implementation; a Phase-5 journalist renderer is
/// a peer implementation of this same trait, authoring from the same
/// `NewsKind` structure rather than patching `TemplateRenderer`'s output.
pub trait NewsRenderer {
    fn render(&self, item: &NewsItem, world: &World) -> String;
}

/// The v1 renderer: fixed templates, one per `NewsKind` variant, resolving
/// names against `world`. Deterministic, no LLM, never fails — a malformed
/// `NewsItem` (an ID that does not exist in `world`) is a caller bug, not
/// something this renderer tries to recover from.
pub struct TemplateRenderer;

impl NewsRenderer for TemplateRenderer {
    fn render(&self, item: &NewsItem, world: &World) -> String {
        match &item.kind {
            NewsKind::MatchResult {
                home,
                away,
                home_goals,
                away_goals,
                ..
            } => format!(
                "{} {home_goals} - {away_goals} {}",
                world.club(*home).name,
                world.club(*away).name
            ),
            NewsKind::TransferCompleted {
                player,
                from,
                to,
                fee,
            } => {
                let name = &world.player(*player).name;
                let buyer = &world.club(*to).name;
                match from {
                    Some(seller) => format!(
                        "{name} joins {buyer} from {} for {fee}.",
                        world.club(*seller).name
                    ),
                    None => format!("{name} joins {buyer} on a free transfer."),
                }
            }
            NewsKind::BidRejected {
                player,
                bidder,
                price,
                reason,
                ..
            } => {
                let name = &world.player(*player).name;
                let club = &world.club(*bidder).name;
                let why = match reason {
                    RejectReason::Outranked => "was outbid",
                    RejectReason::PlayerRefused => "was turned down by the player",
                };
                format!("{club}'s {price} bid for {name} {why}.")
            }
            NewsKind::YouthIntake { club, players } => format!(
                "{} unveils {} youth prospect(s).",
                world.club(*club).name,
                players.len()
            ),
            NewsKind::Retirement { player } => {
                format!("{} announces retirement.", world.player(*player).name)
            }
            NewsKind::ContractExpiring {
                player,
                club,
                days_remaining,
            } => format!(
                "{}'s contract at {} expires in {days_remaining} day(s).",
                world.player(*player).name,
                world.club(*club).name
            ),
            NewsKind::FinancialWarning { club, balance } => format!(
                "{}'s balance has fallen to {balance}.",
                world.club(*club).name
            ),
            NewsKind::RoleCoverageGap { club, role } => format!(
                "{} has no recognised {} on the books.",
                world.club(*club).name,
                role.name()
            ),
        }
    }
}

/// Event-derived news (`EventObserver`) plus state-condition news
/// (`check_conditions`) plus the narrow `WindowOutcome`-Trace path
/// (`observe_rejected_bids`) — see the module docs for why there are three
/// entry points and not one.
///
/// Maintains small incremental indices — squad membership, each player's
/// most recent contract-setting event, each club's most recent
/// finance/squad-affecting event, the fixture->clubs map — built purely from
/// events it has already seen, so `check_conditions` (which only ever sees
/// `&GameState`, never the log) can still attach real provenance to a
/// state-condition item. `warned_*` sets make every state-condition check
/// edge-triggered (fires once per newly-true condition, clears when the
/// condition lifts so a *future* recurrence fires again) rather than
/// spamming the same fact every time it is called while the condition
/// merely persists — the mechanism that keeps a season-long inbox bounded.
#[derive(Debug, Default)]
pub struct NewsObserver {
    next_index: usize,
    items: Vec<NewsItem>,
    player_club: Option<ClubId>,
    current_date: Option<GameDate>,
    fixture_clubs: BTreeMap<FixtureId, (ClubId, ClubId)>,
    club_of: BTreeMap<PlayerId, ClubId>,
    contract_source: BTreeMap<PlayerId, EventRef>,
    finance_source: BTreeMap<ClubId, EventRef>,
    squad_source: BTreeMap<ClubId, EventRef>,
    warned_contract: BTreeSet<(PlayerId, GameDate)>,
    warned_finance: BTreeSet<ClubId>,
    warned_role_gap: BTreeSet<(ClubId, Role)>,
}

impl NewsObserver {
    pub fn new() -> Self {
        Self::default()
    }

    /// The full accumulated inbox, oldest first — exactly the order items
    /// were derived in (event order for category 1, call order for category
    /// 2/3), so replay reproducing the same call sequence reproduces this
    /// list byte for byte.
    pub fn items(&self) -> &[NewsItem] {
        &self.items
    }

    /// A filtered, newest-first read: items at or above `min_salience` whose
    /// audience is `audience` exactly or `League` (relevant to everyone).
    /// The presentation-ready view a consumer actually wants — `items()` is
    /// the raw accumulator underneath it.
    pub fn inbox(&self, audience: Audience, min_salience: u8) -> Vec<&NewsItem> {
        let mut out: Vec<&NewsItem> = self
            .items
            .iter()
            .filter(|item| item.salience >= min_salience)
            .filter(|item| item.audience == audience || item.audience == Audience::League)
            .collect();
        out.reverse();
        out
    }

    fn salience(&self, club: ClubId, mine: u8, other: u8) -> u8 {
        if Some(club) == self.player_club {
            mine
        } else {
            other
        }
    }

    /// Bids-rejected news, sourced from `WindowOutcome`'s Trace directly
    /// (see the module docs) rather than the event log. `window_closed` is
    /// the `EventRef` of the `Event::TransferWindowClosed` this batch of
    /// rejections resolved alongside — every item's real, if coarse,
    /// provenance, even though the rejection itself was never separately
    /// recorded. Not called by `on_event`/`check_conditions`; a live caller
    /// holding a fresh `WindowOutcome` invokes this directly, the moment it
    /// has one (mirroring `player_match_preview`'s live-only re-derivation
    /// of `MatchOutcome`).
    pub fn observe_rejected_bids(
        &mut self,
        date: GameDate,
        window_closed: EventRef,
        rejected: &[RejectedBid],
    ) {
        for r in rejected {
            let mine_involved = Some(r.bidder) == self.player_club
                || r.from.is_some_and(|c| Some(c) == self.player_club);
            let audience = if mine_involved {
                Audience::Club(
                    self.player_club
                        .expect("mine_involved implies player_club is set"),
                )
            } else {
                Audience::Club(r.bidder)
            };
            self.items.push(NewsItem {
                date,
                kind: NewsKind::BidRejected {
                    player: r.player,
                    from: r.from,
                    bidder: r.bidder,
                    price: r.price,
                    reason: r.reason,
                },
                sources: vec![window_closed],
                salience: if mine_involved { 55 } else { 10 },
                audience,
            });
        }
    }

    /// State-condition news (category 2): queries over `state` at its
    /// current date, not events. Intended to be called once after every
    /// command is applied — live and replay alike, at identical points in
    /// the state's progression — so the edge-triggered `warned_*` sets stay
    /// in lockstep and the resulting inbox is exactly reproducible. Does not
    /// widen `EventObserver`; this is a plain inherent method taking
    /// `&GameState` directly, the same seam `market::calibrate::MarketTelemetry`
    /// already established for `record_season_end`.
    pub fn check_conditions(&mut self, state: &GameState) {
        for (&cid, club) in &state.world.clubs {
            // Financial warning: balance below zero, edge-triggered so a
            // club that stays negative for weeks is warned once, not every
            // call, and a later dip after recovering warns again.
            if club.finances.balance.0 < 0 {
                if self.warned_finance.insert(cid) {
                    let sources = self.finance_source.get(&cid).copied().into_iter().collect();
                    self.items.push(NewsItem {
                        date: state.date,
                        kind: NewsKind::FinancialWarning {
                            club: cid,
                            balance: club.finances.balance,
                        },
                        sources,
                        salience: self.salience(cid, 75, 20),
                        audience: Audience::Club(cid),
                    });
                }
            } else {
                self.warned_finance.remove(&cid);
            }

            // Role coverage: any role nobody in the squad is best suited to.
            let mut covered: BTreeSet<Role> = BTreeSet::new();
            for &pid in &club.players {
                let (role, _) = best_role(&state.world.player(pid).attributes, &ROLE_WEIGHTS);
                covered.insert(role);
            }
            for &role in Role::ALL.iter() {
                let key = (cid, role);
                if covered.contains(&role) {
                    self.warned_role_gap.remove(&key);
                } else if self.warned_role_gap.insert(key) {
                    let sources = self.squad_source.get(&cid).copied().into_iter().collect();
                    self.items.push(NewsItem {
                        date: state.date,
                        kind: NewsKind::RoleCoverageGap { club: cid, role },
                        sources,
                        salience: self.salience(cid, 60, 15),
                        audience: Audience::Club(cid),
                    });
                }
            }

            // Contracts expiring within CONTRACT_EXPIRY_WARNING_DAYS. Keyed
            // by (player, expires) so a renewal re-arms the warning for its
            // own future expiry rather than staying silenced forever.
            for &pid in &club.players {
                let Some(contract) = state.world.player(pid).contract else {
                    continue;
                };
                let days_remaining = contract.expires.days - state.date.days;
                if !(0..=CONTRACT_EXPIRY_WARNING_DAYS).contains(&days_remaining) {
                    continue;
                }
                let key = (pid, contract.expires);
                if self.warned_contract.insert(key) {
                    let sources = self
                        .contract_source
                        .get(&pid)
                        .copied()
                        .into_iter()
                        .collect();
                    self.items.push(NewsItem {
                        date: state.date,
                        kind: NewsKind::ContractExpiring {
                            player: pid,
                            club: cid,
                            days_remaining,
                        },
                        sources,
                        salience: self.salience(cid, 65, 20),
                        audience: Audience::Club(cid),
                    });
                }
            }
        }
    }
}

impl EventObserver for NewsObserver {
    fn on_event(&mut self, event: &Event) {
        // `self.next_index` is exactly this event's index into the log: both
        // `Session::from_events` (replay) and `Session::execute` (live) call
        // `on_event` once per event, strictly in log order, so a running
        // count kept here needs no index parameter from the trait — the one
        // constraint the task holds firm on ("do not widen `EventObserver`").
        let this_ref = EventRef(self.next_index);
        self.next_index += 1;

        match event {
            Event::GameStarted {
                player_club,
                world,
                schedule,
                start_date,
                ..
            } => {
                self.player_club = Some(*player_club);
                self.current_date = Some(*start_date);
                for fx in schedule {
                    self.fixture_clubs.insert(fx.id, (fx.home, fx.away));
                }
                for club in world.clubs.values() {
                    for &pid in &club.players {
                        self.club_of.insert(pid, club.id);
                    }
                    self.squad_source.insert(club.id, this_ref);
                    self.finance_source.insert(club.id, this_ref);
                }
                for (&pid, p) in &world.players {
                    if p.contract.is_some() {
                        self.contract_source.insert(pid, this_ref);
                    }
                }
            }
            Event::SeasonStarted {
                start_date,
                schedule,
            } => {
                self.current_date = Some(*start_date);
                for fx in schedule {
                    self.fixture_clubs.insert(fx.id, (fx.home, fx.away));
                }
            }
            Event::MatchdayAdvanced { new_date, .. } => {
                self.current_date = Some(*new_date);
            }
            Event::MatchPlayed {
                fixture,
                home_goals,
                away_goals,
                ..
            } => {
                let Some(&(home, away)) = self.fixture_clubs.get(fixture) else {
                    // A malformed/partial log (no schedule seen for this
                    // fixture) — skip rather than fabricate a home/away.
                    return;
                };
                let mine_involved =
                    Some(home) == self.player_club || Some(away) == self.player_club;
                let date = self.current_date.unwrap_or(GameDate { days: 0 });
                let audience = if mine_involved {
                    Audience::Club(
                        self.player_club
                            .expect("mine_involved implies player_club is set"),
                    )
                } else {
                    Audience::League
                };
                self.items.push(NewsItem {
                    date,
                    kind: NewsKind::MatchResult {
                        fixture: *fixture,
                        home,
                        away,
                        home_goals: *home_goals,
                        away_goals: *away_goals,
                    },
                    sources: vec![this_ref],
                    salience: if mine_involved { 70 } else { 15 },
                    audience,
                });
            }
            Event::TransferCompleted {
                date,
                player,
                from,
                to,
                fee,
                ..
            } => {
                self.club_of.insert(*player, *to);
                self.contract_source.insert(*player, this_ref);
                self.finance_source.insert(*to, this_ref);
                self.squad_source.insert(*to, this_ref);
                if let Some(seller) = from {
                    self.finance_source.insert(*seller, this_ref);
                    self.squad_source.insert(*seller, this_ref);
                }
                let mine_involved = Some(*to) == self.player_club
                    || from.is_some_and(|c| Some(c) == self.player_club);
                let audience = if mine_involved {
                    Audience::Club(
                        self.player_club
                            .expect("mine_involved implies player_club is set"),
                    )
                } else {
                    Audience::Club(*to)
                };
                self.items.push(NewsItem {
                    date: *date,
                    kind: NewsKind::TransferCompleted {
                        player: *player,
                        from: *from,
                        to: *to,
                        fee: *fee,
                    },
                    sources: vec![this_ref],
                    salience: if mine_involved { 80 } else { 25 },
                    audience,
                });
            }
            Event::PlayerReleased { player, club, .. } => {
                self.club_of.remove(player);
                self.contract_source.remove(player);
                self.squad_source.insert(*club, this_ref);
            }
            Event::ContractRenewed { player, club, .. } => {
                self.club_of.insert(*player, *club);
                self.contract_source.insert(*player, this_ref);
            }
            Event::YouthIntake {
                date,
                club,
                players,
            } => {
                for p in players {
                    self.club_of.insert(p.id, *club);
                    self.contract_source.insert(p.id, this_ref);
                }
                self.squad_source.insert(*club, this_ref);
                let mine = Some(*club) == self.player_club;
                self.items.push(NewsItem {
                    date: *date,
                    kind: NewsKind::YouthIntake {
                        club: *club,
                        players: players.iter().map(|p| p.id).collect(),
                    },
                    sources: vec![this_ref],
                    salience: if mine { 50 } else { 15 },
                    audience: Audience::Club(*club),
                });
            }
            Event::PlayerRetired { date, player } => {
                let club = self.club_of.remove(player);
                self.contract_source.remove(player);
                let mine = club.is_some() && club == self.player_club;
                self.items.push(NewsItem {
                    date: *date,
                    kind: NewsKind::Retirement { player: *player },
                    sources: vec![this_ref],
                    salience: if mine { 45 } else { 15 },
                    audience: club.map(Audience::Club).unwrap_or(Audience::League),
                });
            }
            Event::FinanceTick { deltas, .. } => {
                for (cid, _) in deltas {
                    self.finance_source.insert(*cid, this_ref);
                }
            }
            // No category-1 news and no provenance index this module needs
            // from these: a plain lineup pick, a development tick, the
            // season-over marker, and §10's pre-commitment bookkeeping are
            // all outside this module's initial `NewsKind` coverage.
            Event::LineupSubmitted { .. }
            | Event::DevelopmentTick { .. }
            | Event::SeasonEnded { .. }
            | Event::TransferDecisionSubmitted { .. }
            | Event::TransferWindowClosed { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Command;
    use crate::session::Session;
    use crate::state::GameState;
    use crate::worldgen::{WorldGenConfig, generate};
    use fforge_domain::{
        Attributes, Character, Club, Competition, CompetitionId, Contract, DevProfile, Finances,
        NUM_ATTRIBUTES, Player,
    };

    /// Drive a real game (worldgen'd league, a full season, then a slice of
    /// the next one) far enough to cross at least two transfer windows and
    /// gather a realistic spread of news, calling `check_conditions` after
    /// every command exactly as the module docs prescribe. Returns the
    /// finished `Session`, the `NewsObserver` that watched it live, the log
    /// length right after the opening `GameStarted` (before any command —
    /// and before any `check_conditions` call, since none happens until the
    /// first command executes), and the command-boundary log lengths
    /// (`session.log.len()` right after each `check_conditions` call) a
    /// replay driver needs to reproduce the same call cadence.
    fn drive_a_real_game(seed: u64) -> (Session, NewsObserver, usize, Vec<usize>) {
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, schedule, start) = generate(seed, &cfg);
        let player_club = world.competition.clubs[0];
        let opening = vec![Event::GameStarted {
            seed,
            start_date: start,
            player_club,
            world,
            schedule,
        }];

        let mut news = NewsObserver::new();
        let mut session = Session::from_events(opening, &mut [&mut news]);
        let first_boundary = session.log.len();
        let mut boundaries = Vec::new();

        for _ in 0..(38 + 8) {
            if session.state.season_over() {
                session
                    .execute(Command::StartNextSeason, &mut [&mut news])
                    .expect("start next season");
            } else {
                session
                    .execute(Command::AdvanceMatchday, &mut [&mut news])
                    .expect("advance");
            }
            news.check_conditions(&session.state);
            boundaries.push(session.log.len());
        }

        (session, news, first_boundary, boundaries)
    }

    #[test]
    fn replaying_a_log_reproduces_an_identical_news_stream() {
        let (session, live, first_boundary, boundaries) = drive_a_real_game(2026);
        assert!(
            !live.items().is_empty(),
            "test scenario produced no news at all — widen it"
        );

        // Fold the exact same log again from scratch, feeding on_event
        // through the opening GameStarted with no check_conditions call yet
        // (the live run made none either, before its first command), then
        // on_event + check_conditions at the identical per-command
        // boundaries the live run used — the property under test: same
        // event stream, same call cadence, same news, byte for byte.
        let log = session.log.clone();
        let mut replayed = NewsObserver::new();
        for event in &log[0..first_boundary] {
            replayed.on_event(event);
        }
        let mut prev = first_boundary;
        for &boundary in &boundaries {
            for event in &log[prev..boundary] {
                replayed.on_event(event);
            }
            let state = GameState::replay(&log[..boundary]);
            replayed.check_conditions(&state);
            prev = boundary;
        }

        // Item-by-item first, so a genuine divergence points at exactly
        // which item and how, not an unreadable whole-vector diff.
        for (i, (a, b)) in live.items().iter().zip(replayed.items().iter()).enumerate() {
            assert_eq!(
                a, b,
                "replay diverged from the live news stream at item {i}"
            );
        }
        assert_eq!(
            live.items().len(),
            replayed.items().len(),
            "replay must reproduce an identical news stream (item count differs)"
        );
    }

    #[test]
    fn every_items_sources_resolve_to_real_log_entries() {
        let (session, live, _first_boundary, _boundaries) = drive_a_real_game(41);
        for item in live.items() {
            for src in &item.sources {
                assert!(
                    src.0 < session.log.len(),
                    "{:?} has an out-of-range source {src:?} (log has {} entries)",
                    item.kind,
                    session.log.len()
                );
            }
        }

        // `observe_rejected_bids`' sources are supplied by the caller, not
        // derived here — check the contract holds for a real window index.
        let mut news = NewsObserver::new();
        news.player_club = Some(ClubId(0));
        let window_closed = EventRef(session.log.len() - 1);
        news.observe_rejected_bids(
            session.state.date,
            window_closed,
            &[RejectedBid {
                round: 0,
                player: PlayerId(0),
                from: None,
                bidder: ClubId(0),
                price: Money(1_000_000),
                reason: RejectReason::PlayerRefused,
            }],
        );
        for item in news.items() {
            for src in &item.sources {
                assert_eq!(*src, window_closed);
                assert!(src.0 < session.log.len());
            }
        }
    }

    #[test]
    fn salience_filtering_yields_a_bounded_inbox_over_a_full_season() {
        let (session, live, _first_boundary, _boundaries) = drive_a_real_game(7);
        let player_club = session.state.player_club;

        let everything = live.items().len();
        let filtered = live.inbox(Audience::Club(player_club), 50);

        assert!(
            everything > 50,
            "a full season plus a slice of the next should generate a lot of raw news, got {everything}"
        );
        assert!(
            filtered.len() < everything,
            "salience filtering must actually narrow the inbox"
        );
        assert!(
            filtered.len() < 100,
            "a high-salience inbox for one club over ~1.2 seasons must stay small, got {}",
            filtered.len()
        );
        // Newest first.
        for pair in filtered.windows(2) {
            assert!(
                pair[0].date.days >= pair[1].date.days,
                "inbox must be newest-first"
            );
        }
    }

    #[test]
    fn check_conditions_is_edge_triggered_not_level_triggered() {
        // A minimal one-club world with a negative balance: calling
        // check_conditions repeatedly against the SAME state must only ever
        // emit the financial warning once, not once per call.
        let cfg = WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (mut world, schedule, start) = generate(3, &cfg);
        let cid = world.competition.clubs[0];
        world.clubs.get_mut(&cid).unwrap().finances.balance = Money(-1);

        let opening = vec![Event::GameStarted {
            seed: 3,
            start_date: start,
            player_club: cid,
            world,
            schedule,
        }];
        let mut news = NewsObserver::new();
        let session = Session::from_events(opening, &mut [&mut news]);

        news.check_conditions(&session.state);
        news.check_conditions(&session.state);
        news.check_conditions(&session.state);

        let warnings = news
            .items()
            .iter()
            .filter(|i| matches!(i.kind, NewsKind::FinancialWarning { .. }))
            .count();
        assert_eq!(
            warnings, 1,
            "a persisting condition must warn once, not every call"
        );
    }

    fn mini_world_for_render() -> (World, PlayerId, PlayerId, ClubId, ClubId) {
        let cfg = WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _schedule, _start) = generate(99, &cfg);
        let clubs = &world.competition.clubs;
        let (home, away) = (clubs[0], clubs[1]);
        let p1 = world.club(home).players[0];
        let p2 = world.club(away).players[0];
        (world, p1, p2, home, away)
    }

    #[test]
    fn template_renderer_never_panics_across_the_whole_newskind_alphabet() {
        let (world, p1, p2, home, away) = mini_world_for_render();
        let renderer = TemplateRenderer;
        let date = GameDate { days: 2030 * 365 };

        let kinds = vec![
            NewsKind::MatchResult {
                fixture: FixtureId(0),
                home,
                away,
                home_goals: 2,
                away_goals: 1,
            },
            NewsKind::TransferCompleted {
                player: p1,
                from: Some(away),
                to: home,
                fee: Money(1_000_000),
            },
            NewsKind::TransferCompleted {
                player: p1,
                from: None,
                to: home,
                fee: Money(0),
            },
            NewsKind::BidRejected {
                player: p2,
                from: Some(away),
                bidder: home,
                price: Money(500_000),
                reason: RejectReason::Outranked,
            },
            NewsKind::BidRejected {
                player: p2,
                from: None,
                bidder: home,
                price: Money(500_000),
                reason: RejectReason::PlayerRefused,
            },
            NewsKind::YouthIntake {
                club: home,
                players: vec![p1, p2],
            },
            NewsKind::Retirement { player: p1 },
            NewsKind::ContractExpiring {
                player: p1,
                club: home,
                days_remaining: 12,
            },
            NewsKind::FinancialWarning {
                club: home,
                balance: Money(-42),
            },
            NewsKind::RoleCoverageGap {
                club: home,
                role: Role::Gk,
            },
        ];

        for kind in kinds {
            let item = NewsItem {
                date,
                kind,
                sources: vec![EventRef(0)],
                salience: 50,
                audience: Audience::Club(home),
            };
            let rendered = renderer.render(&item, &world);
            assert!(!rendered.is_empty(), "{:?} rendered empty", item.kind);
        }
    }

    /// `TemplateRenderer` reads nothing beyond `World`'s public shape — a
    /// minimal hand-built world (no worldgen) renders exactly as expected.
    #[test]
    fn renders_from_a_hand_built_world_too() {
        let mk_player = |id: u32, name: &str| Player {
            id: PlayerId(id),
            name: name.to_string(),
            birth: GameDate { days: 2000 * 365 },
            natural_role: Role::St,
            attributes: Attributes::new([50u8; NUM_ATTRIBUTES]),
            character: Character {
                potential: 60,
                determination: 50,
                professionalism: 50,
                consistency: 50,
                injury_proneness: 50,
                natural_fitness: 50,
                leadership: 50,
            },
            development: DevProfile {
                efficiency_milli: 720,
                bloomer_phase_centi: 0,
            },
            contract: Some(Contract {
                wage: Money(1_000),
                expires: GameDate { days: 2031 * 365 },
            }),
            retired: false,
            injured_until: None,
        };
        let p1 = mk_player(0, "Alpha");
        let mut players = BTreeMap::new();
        players.insert(p1.id, p1.clone());

        let club = Club {
            id: ClubId(0),
            name: "Test United".to_string(),
            players: vec![p1.id],
            coaching_milli: 1000,
            finances: Finances {
                balance: Money(0),
                wage_budget: Money(0),
            },
            reputation: 50,
        };
        let mut clubs = BTreeMap::new();
        clubs.insert(club.id, club);

        let world = World {
            players,
            clubs,
            staff: BTreeMap::new(),
            competition: Competition {
                id: CompetitionId(0),
                name: "Test".to_string(),
                clubs: vec![ClubId(0)],
            },
        };

        let item = NewsItem {
            date: GameDate { days: 0 },
            kind: NewsKind::Retirement { player: p1.id },
            sources: vec![EventRef(0)],
            salience: 50,
            audience: Audience::Club(ClubId(0)),
        };
        let rendered = TemplateRenderer.render(&item, &world);
        assert_eq!(rendered, "Alpha announces retirement.");
    }
}
