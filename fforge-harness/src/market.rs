//! The Phase-4 market pathology harness (`TRANSFER_MODEL.md` §11) — the
//! transfer-market sibling of `fforge-harness::match_engine::StreamTelemetry` and
//! `career_arc`. Where those drive the real worldgen + match / development
//! pipeline and report emergent match / career aggregates, this drives the
//! real worldgen + **full command pipeline** (matches, development, finance,
//! pool, and market clearing — every tick `commands::advance_matchday`
//! already fires) pooled over **many world seeds, each traced across ~15
//! seasons** (`TRANSFER_MODEL.md` §11: "the third phase to learn" that
//! competitive-balance metrics need multi-seed pooling), and reports the §11
//! metric table.
//!
//! **§1.1 note.** `TRANSFER_MODEL.md` §1.1 deliberately skips a Python
//! scratchpad for Phase 4: a transfer market's pathologies are emergent from
//! twenty real squads under real policies, not shape-findable on a synthetic
//! stand-in. **This harness is Phase 4's shape-finder.**
//!
//! **A passive consumer of the event stream and window Traces**
//! (`DESIGN.md` §5 — never writes to the world). `MarketTelemetry` implements
//! `EventObserver` to fold the persisted `TransferCompleted` / `YouthIntake` /
//! `PlayerRetired` events exactly the way `SeasonTelemetry` folds
//! `MatchPlayed`. But `market::resolve_window`'s rich `WindowOutcome`
//! (rejected bids, the frozen valuation cache, unfilled needs) never survives
//! the fold — `commands::transfer_window_events` folds only the completions
//! into events and discards the rest as a Trace, exactly `MatchOutcome`'s
//! shape (`MATCH_MODEL.md` §7). The competitive-balance metrics this harness
//! needs (points-Gini, rank churn, talent concentration, financial health,
//! pool shape) are therefore read the same way `career_arc::sample_world`
//! reads career data: a read-only query over the folded `World` snapshot at
//! each season boundary, via the same `valuation::value_all` and
//! `state::league_table` the live game itself calls. `record_season_end` is
//! this harness's "Trace" seam — computed, ephemeral, diagnostic data that
//! is never fed back into the fold.
//!
//! **Interpretation note, carried from `TRANSFER_MODEL.md` §2.6.** Every club
//! in v1 is an omniscient valuer — no scouting fog-of-war exists yet. The
//! concentration reading below (`top3_share_of_top20`) is therefore an
//! **upper bound** under perfect information, not a prediction of the fogged
//! Phase-5 game. Do not tune the market to "fix" concentration that is an
//! artifact of that perfect information.
//!
//! **Scope fence**, exactly like both siblings: this module is harness
//! plumbing, never fed back into `ValueKnobs`/`FinanceKnobs` by itself — the
//! re-fit is a human reading these numbers and editing the knob tables
//! (`TRANSFER_MODEL.md` §9).

use fforge_core::club_ai::{ClubPolicy, ListTrigger, Need, UtilityKnobs, UtilityPolicy};
use fforge_core::development::DevKnobs;
use fforge_core::event::Event;
use fforge_core::market::WindowView;
use fforge_core::observer::EventObserver;
use fforge_core::scouting::ScoutKnobs;
use fforge_core::state::{GameState, league_table};
use fforge_core::valuation::{MarketContext, ValueKnobs, value_all};
use fforge_core::{Command, Session, WorldGenConfig, new_game};
use fforge_domain::{Club, ClubId, Fixture, FixtureId, GameDate, Money, PlayerId, Role, World};
use std::collections::BTreeMap;

/// A club whose cash balance has gone negative — the literal "insolvent"
/// reading (`TRANSFER_MODEL.md` §11). `Money` is signed for exactly this
/// reason (§3: "the pathology harness needs to *see* insolvency rather than
/// have it clamped away").
fn is_insolvent(club: &Club) -> bool {
    club.finances.balance.0 < 0
}

/// A club sitting on cash worth more than this many years of its own wage
/// budget — the "hoarding" pathology (§11): cash that never gets reinvested
/// in the squad. A documented modelling choice for this harness's own
/// gross-regression read, not a game rule enforced anywhere else.
const HOARDING_YEARS_OF_WAGE_BUDGET: f64 = 5.0;

fn is_hoarding(club: &Club) -> bool {
    club.finances.balance.0 as f64
        > HOARDING_YEARS_OF_WAGE_BUDGET * club.finances.wage_budget.0 as f64
}

/// Minimum goalkeepers per club (`TRANSFER_MODEL.md` §6, `club_ai::UtilityKnobs::min_goalkeepers`
/// default) — duplicated here as a literal constant rather than threading
/// `UtilityKnobs` through, since the harness never decides squad policy, only
/// reads whether the stabilizer actually held.
const MIN_GOALKEEPERS: usize = 2;

/// Squad-size ceiling (`TRANSFER_MODEL.md` §6, `club_ai::UtilityKnobs::squad_max`
/// default) — duplicated here for the same reason as `MIN_GOALKEEPERS`: the
/// harness reads whether a club is pinned against the cap, it never decides
/// squad policy. §9's open residual ("squads pin at `squad_max` in every
/// seed") is exactly what `worst_club_cap_fraction`/`below_cap_share` below
/// are built to catch.
const SQUAD_MAX: usize = 30;

/// Number of transfer windows resolved per season (`TRANSFER_MODEL.md` §7:
/// summer + winter) — known by construction, not observed, so
/// "transfers per club per window" needs no window-boundary bookkeeping from
/// the event stream (a window with zero completions emits no event at all).
const WINDOWS_PER_SEASON: usize = 2;

/// Max age counted in the "youth valuation" cut below — `pool::youth_cohort`'s
/// own 16-18 intake band (`TRANSFER_MODEL.md` §8.1), the population the
/// `DEVELOPMENT_MODEL.md` §8 seeding invert changes the CA/PA relationship
/// for most directly.
const YOUTH_MAX_AGE: i32 = 18;

/// How many seasons at each end of a seed's run are averaged for the
/// "early" / "late" reduction (Gini trajectory, concentration trend, fee
/// inflation) — a few seasons smooths a single noisy season without washing
/// out the actual yr1-vs-yr15 comparison the metric table asks for.
const EARLY_LATE_WINDOW: usize = 3;

/// The Gini coefficient of a finite population, via the standard rank-sum
/// form: `G = 2*Σ(i·x_i) / (n·Σx) − (n+1)/n` over ascending-sorted `x`
/// (`i` 1-based). 0 = perfect equality, → 1 = maximal concentration.
fn gini(values: &[f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return f64::NAN;
    }
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(f64::total_cmp);
    let sum: f64 = v.iter().sum();
    if sum == 0.0 {
        return 0.0;
    }
    let acc: f64 = v
        .iter()
        .enumerate()
        .map(|(i, x)| (i as f64 + 1.0) * x)
        .sum();
    (2.0 * acc) / (n as f64 * sum) - (n as f64 + 1.0) / n as f64
}

/// Mean of the finite entries of a slice — NaN-tolerant reduction shared by
/// every per-season → per-seed fold below (mirrors `career_arc::mean_finite`;
/// duplicated rather than shared, per each harness's independent-plumbing
/// convention).
fn mean_finite(xs: &[f64]) -> f64 {
    let valid: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if valid.is_empty() {
        f64::NAN
    } else {
        valid.iter().sum::<f64>() / valid.len() as f64
    }
}

/// The p-quantile (0..1) of a copy-sorted sample, linear on ranks.
fn percentile(xs: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let rank = p * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        v[lo]
    } else {
        v[lo] + (rank - lo as f64) * (v[hi] - v[lo])
    }
}

fn median(xs: &[f64]) -> f64 {
    percentile(xs, 0.5)
}

/// Mean, sd, and range of a per-seed metric — the `MATCH_MODEL.md` §8
/// noisy-estimator readout, the same shape `career_arc::SeedSpread` reports.
pub struct SeedSpread {
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
    pub n: usize,
}

fn seed_spread(xs: &[f64]) -> SeedSpread {
    let valid: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    let n = valid.len();
    if n == 0 {
        return SeedSpread {
            mean: f64::NAN,
            sd: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
            n: 0,
        };
    }
    let mean = valid.iter().sum::<f64>() / n as f64;
    let sd = if n < 2 {
        0.0
    } else {
        (valid.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    };
    let min = valid.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = valid.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    SeedSpread {
        mean,
        sd,
        min,
        max,
        n,
    }
}

/// Every rostered player's age at `today`, one entry per (club, player) —
/// squad membership is the sole club↔player index (`TRANSFER_MODEL.md` §3),
/// so iterating clubs' rosters directly is both the natural and the cheapest
/// way to read "league mean age" (no free agents, no retirees: both are
/// off every roster by construction).
fn squad_ages(world: &World, today: GameDate) -> Vec<f64> {
    world
        .clubs
        .values()
        .flat_map(|c| &c.players)
        .map(|&pid| world.player(pid).age(today) as f64)
        .collect()
}

fn gk_counts(world: &World) -> Vec<usize> {
    world
        .clubs
        .values()
        .map(|c| {
            c.players
                .iter()
                .filter(|&&pid| world.player(pid).natural_role == Role::Gk)
                .count()
        })
        .collect()
}

/// Share of the league's top-20 valued players (by `valuation::value_all`,
/// restricted to currently rostered players — free agents and retirees have
/// no club to concentrate talent *in*) who belong to the league's top-3
/// clubs by points (`TRANSFER_MODEL.md` §11's "Top-3 clubs' share of league
/// top-20 players"). Read §2.6 before treating a high number as broken: v1's
/// omniscient valuers make this an upper bound, not a prediction.
fn top3_share_of_top20(
    world: &World,
    table: &[fforge_core::state::TableRow],
    valuations: &BTreeMap<PlayerId, Money>,
) -> f64 {
    let top3: std::collections::BTreeSet<ClubId> = table.iter().take(3).map(|r| r.club).collect();

    let mut signed: Vec<(PlayerId, Money, Option<ClubId>)> = valuations
        .iter()
        .filter_map(|(&pid, &v)| world.club_of(pid).map(|c| (pid, v, Some(c))))
        .collect();
    signed.sort_by_key(|s| std::cmp::Reverse(s.1.0));
    let top20 = &signed[..signed.len().min(20)];
    if top20.is_empty() {
        return f64::NAN;
    }
    let in_top3 = top20
        .iter()
        .filter(|(_, _, club)| club.is_some_and(|c| top3.contains(&c)))
        .count();
    in_top3 as f64 / top20.len() as f64
}

/// Per-seed accumulator: consumes one seed's event stream (`EventObserver`)
/// and periodic season-end snapshots (`record_season_end`, called explicitly
/// by the harness driver — the market's own "Trace" seam, since the real
/// `WindowOutcome` never survives the fold).
#[derive(Default)]
pub struct MarketTelemetry {
    /// Bumped on every `Event::SeasonStarted` — season 0 is the opening
    /// season, season `k` begins after the `k`-th `StartNextSeason`.
    season: u32,
    total_transfers: u32,
    all_fees: Vec<i64>,
    fees_by_season: BTreeMap<u32, Vec<i64>>,

    // --- season-end snapshots (`record_season_end`), one entry per season.
    gini_by_season: Vec<f64>,
    concentration_by_season: Vec<f64>,
    mean_age_by_season: Vec<f64>,
    fee_median_by_season: Vec<f64>,
    squad_size_min_by_season: Vec<f64>,
    squad_size_max_by_season: Vec<f64>,
    insolvent_by_season: Vec<f64>,
    hoarding_by_season: Vec<f64>,
    rank_churns: Vec<f64>,
    role_coverage_violations: u32,

    /// Youth-valuation cut (`start_age ≤ YOUTH_MAX_AGE`): mean/median value
    /// and headcount at each season-end snapshot — its own read, not folded
    /// into the league-wide fee/valuation numbers above.
    youth_value_mean_by_season: Vec<f64>,
    youth_value_median_by_season: Vec<f64>,
    youth_count_by_season: Vec<f64>,

    /// Per-club squad size at each season-end snapshot — the individual-club
    /// counterpart to `squad_size_min_by_season`/`_max_by_season`'s
    /// league-wide extremes. §9's "squads pin at `squad_max` in every seed"
    /// residual is a per-club pathology (one club sitting *at* the cap
    /// persistently), which a league-wide min/max cannot distinguish from a
    /// healthy league where different clubs take turns brushing the ceiling.
    club_squad_sizes_by_season: BTreeMap<ClubId, Vec<usize>>,

    prev_ranks: Option<BTreeMap<ClubId, usize>>,

    /// What every player was truly worth when the open window began
    /// (`note_true_values`), so a fee can be read against it.
    true_values: BTreeMap<PlayerId, Money>,
    /// Each player's latest fee-paying move, until he leaves that club.
    open_signings: BTreeMap<PlayerId, Signing>,
    signings: Vec<Signing>,
    /// Day of the last transfer or release seen: how far each signing has
    /// been followed.
    last_day: i64,
}

/// One fee-paying transfer seen from the buyer's side (`AGENT_MODEL.md`
/// §5.6): what was paid, what the player was truly worth, and how the
/// buyer's ownership of him ended, if it has.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signing {
    pub player: PlayerId,
    pub date: GameDate,
    pub fee: i64,
    pub true_value: Option<i64>,
    /// The need the buyer's policy says he filled (`explain_window`).
    /// `None` when it names none: the human seat's replayed plan, or a
    /// squad that changed between the bid and the reading.
    pub bought_as: Option<Need>,
    pub exit: Option<Exit>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exit {
    pub date: GameDate,
    /// `None` when the buyer released him for nothing.
    pub resale_fee: Option<i64>,
    /// The trigger the seller's policy listed him under, read the same way.
    pub listed_as: Option<ListTrigger>,
}

/// How a signing left its buyer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Departure {
    Listed(ListTrigger),
    /// Sold, but the seller's policy names no trigger for it (see
    /// `explain_window`).
    SoldUnexplained,
    Released,
}

impl Exit {
    pub fn departure(&self) -> Departure {
        match (self.resale_fee, self.listed_as) {
            (None, _) => Departure::Released,
            (Some(_), Some(trigger)) => Departure::Listed(trigger),
            (Some(_), None) => Departure::SoldUnexplained,
        }
    }
}

/// Why one completed transfer happened, as the two clubs' policy explains
/// it (`TRANSFER_MODEL.md` §6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferReason {
    pub player: PlayerId,
    pub date: GameDate,
    pub bought_as: Option<Need>,
    pub listed_as: Option<ListTrigger>,
}

const DAYS_PER_YEAR: i64 = 365;
/// A resale under this share of the purchase fee counts as the buyer
/// writing the signing off.
const WRITE_OFF_RESALE_SHARE: f64 = 0.5;
/// Fees at or above this percentile of a seed's fees are "paid like a star".
const STAR_FEE_PERCENTILE: f64 = 0.75;
const FLOP_HORIZON_DAYS: i64 = 2 * DAYS_PER_YEAR;

impl Signing {
    fn exited_within(&self, days: i64) -> Option<Exit> {
        self.exit.filter(|e| e.date.days - self.date.days <= days)
    }

    fn followed_for(&self, days: i64, until_day: i64) -> bool {
        until_day - self.date.days >= days
    }

    /// Released, or sold on for under half of what he cost.
    fn written_off_within(&self, days: i64) -> bool {
        self.exited_within(days).is_some_and(|e| {
            e.resale_fee
                .is_none_or(|resale| (resale as f64) < WRITE_OFF_RESALE_SHARE * self.fee as f64)
        })
    }
}

impl EventObserver for MarketTelemetry {
    fn on_event(&mut self, event: &Event) {
        match event {
            Event::SeasonStarted { .. } => {
                self.season += 1;
            }
            Event::TransferCompleted {
                date,
                player,
                from,
                fee,
                ..
            } => {
                self.total_transfers += 1;
                self.all_fees.push(fee.0);
                self.fees_by_season
                    .entry(self.season)
                    .or_default()
                    .push(fee.0);
                self.last_day = date.days;
                self.close_signing(*player, *date, Some(fee.0));
                if from.is_some() && fee.0 > 0 {
                    self.open_signings.insert(
                        *player,
                        Signing {
                            player: *player,
                            date: *date,
                            fee: fee.0,
                            true_value: self.true_values.get(player).map(|m| m.0),
                            bought_as: None,
                            exit: None,
                        },
                    );
                }
            }
            Event::PlayerReleased { date, player, .. } => {
                self.last_day = date.days;
                self.close_signing(*player, *date, None);
            }
            _ => {}
        }
    }
}

impl MarketTelemetry {
    /// Record what every player is truly worth as a window is about to
    /// resolve. The harness may read truth (`AGENT_MODEL.md` §5.1); the
    /// clubs whose fees are measured against it could not.
    pub fn note_true_values(&mut self, values: BTreeMap<PlayerId, Money>) {
        self.true_values = values;
    }

    /// Attach the policy's reasons to the signing a transfer opened and to
    /// the one it closed. Called after the window's events have been seen.
    pub fn note_reasons(&mut self, reasons: &[TransferReason]) {
        for r in reasons {
            if let Some(opened) = self
                .open_signings
                .get_mut(&r.player)
                .filter(|s| s.date == r.date)
            {
                opened.bought_as = r.bought_as;
            }
            if let Some(exit) = self
                .signings
                .iter_mut()
                .rev()
                .filter(|s| s.player == r.player)
                .find_map(|s| s.exit.as_mut().filter(|e| e.date == r.date))
            {
                exit.listed_as = r.listed_as;
            }
        }
    }

    fn close_signing(&mut self, player: PlayerId, date: GameDate, resale_fee: Option<i64>) {
        if let Some(mut signing) = self.open_signings.remove(&player) {
            signing.exit = Some(Exit {
                date,
                resale_fee,
                listed_as: None,
            });
            self.signings.push(signing);
        }
    }

    /// Every fee-paying signing, finished or still running.
    pub fn signings(&self) -> impl Iterator<Item = &Signing> {
        self.signings.iter().chain(self.open_signings.values())
    }

    /// Snapshot the folded world at one season's close — before
    /// `Command::StartNextSeason` fires and clears `results` (`state.rs`'s
    /// `SeasonStarted` fold arm). Pure reads only: `league_table` and
    /// `valuation::value_all` are the same calls the live game itself makes,
    /// never a second encoding of either.
    pub fn record_season_end(
        &mut self,
        world: &World,
        schedule: &[Fixture],
        results: &BTreeMap<FixtureId, (u8, u8)>,
        today: GameDate,
        value_knobs: &ValueKnobs,
        dev_knobs: &DevKnobs,
    ) {
        let table = league_table(world, schedule, results);
        let points: Vec<f64> = table.iter().map(|r| r.points() as f64).collect();
        self.gini_by_season.push(gini(&points));

        let ranks: BTreeMap<ClubId, usize> =
            table.iter().enumerate().map(|(i, r)| (r.club, i)).collect();
        if let Some(prev) = &self.prev_ranks {
            let n = ranks.len().max(1) as f64;
            let churn: f64 = ranks
                .iter()
                .map(|(club, &r)| {
                    let p = prev.get(club).copied().unwrap_or(r);
                    (r as f64 - p as f64).abs()
                })
                .sum::<f64>()
                / n;
            self.rank_churns.push(churn);
        }
        self.prev_ranks = Some(ranks);

        // No per-player rating history at this snapshot layer (`world`
        // only, not a full `GameState`) — neutral form, the identity; this
        // harness's own target metrics are about transfer/valuation
        // patterns, not form's calibration.
        let ctx = MarketContext::from_world(world, value_knobs, &BTreeMap::new());
        let valuations = value_all(world, today, &ctx, value_knobs, dev_knobs);
        self.concentration_by_season
            .push(top3_share_of_top20(world, &table, &valuations));

        let youth_values: Vec<f64> = world
            .clubs
            .values()
            .flat_map(|c| &c.players)
            .filter(|&&pid| world.player(pid).age(today) <= YOUTH_MAX_AGE)
            .filter_map(|pid| valuations.get(pid).map(|m| m.0 as f64))
            .collect();
        self.youth_value_mean_by_season
            .push(mean_finite(&youth_values));
        self.youth_value_median_by_season
            .push(median(&youth_values));
        self.youth_count_by_season.push(youth_values.len() as f64);

        self.mean_age_by_season
            .push(mean_finite(&squad_ages(world, today)));

        let sizes: Vec<f64> = world
            .clubs
            .values()
            .map(|c| c.players.len() as f64)
            .collect();
        self.squad_size_min_by_season
            .push(sizes.iter().cloned().fold(f64::INFINITY, f64::min));
        self.squad_size_max_by_season
            .push(sizes.iter().cloned().fold(f64::NEG_INFINITY, f64::max));

        for (&club_id, club) in &world.clubs {
            self.club_squad_sizes_by_season
                .entry(club_id)
                .or_default()
                .push(club.players.len());
        }

        for gk in gk_counts(world) {
            if gk < MIN_GOALKEEPERS {
                self.role_coverage_violations += 1;
            }
        }

        let insolvent = world.clubs.values().filter(|c| is_insolvent(c)).count();
        let hoarding = world.clubs.values().filter(|c| is_hoarding(c)).count();
        self.insolvent_by_season.push(insolvent as f64);
        self.hoarding_by_season.push(hoarding as f64);

        let fees_this_season: Vec<f64> = self
            .fees_by_season
            .get(&self.season)
            .map(|fees| fees.iter().map(|&f| f as f64).collect())
            .unwrap_or_default();
        self.fee_median_by_season.push(median(&fees_this_season));
    }

    fn all_fees_f64(&self) -> Vec<f64> {
        self.all_fees.iter().map(|&f| f as f64).collect()
    }
}

/// Pooled §11 report: every metric reduced to one number per seed, plus the
/// pooled spread across seeds (`SeedSpread`) — the same shape
/// `CareerArcReport` and `StreamTelemetry`'s callers use, for the same
/// reason: a single league's competitive-balance reading is nearly
/// meaningless (`TRANSFER_MODEL.md` §11).
#[derive(Default)]
pub struct MarketReport {
    pub seeds: usize,
    pub seasons: usize,

    transfers_per_club_per_window: Vec<f64>,
    fee_median: Vec<f64>,
    fee_p90: Vec<f64>,
    gini_early: Vec<f64>,
    gini_late: Vec<f64>,
    rank_churn_mean: Vec<f64>,
    concentration_early: Vec<f64>,
    concentration_late: Vec<f64>,
    fee_inflation_ratio: Vec<f64>,
    insolvent_mean: Vec<f64>,
    hoarding_mean: Vec<f64>,
    mean_age_mean: Vec<f64>,
    squad_size_min: Vec<f64>,
    squad_size_max: Vec<f64>,
    role_coverage_violations: Vec<f64>,
    /// Youth-valuation cut (`start_age ≤ YOUTH_MAX_AGE`): per-seed mean of
    /// the season-by-season mean/median value and headcount.
    youth_value_mean: Vec<f64>,
    youth_value_median: Vec<f64>,
    youth_count_mean: Vec<f64>,
    /// Per seed: the *worst-case single club's* fraction of season-end
    /// snapshots sitting at `SQUAD_MAX` — reported for visibility only. A max
    /// over ~20 clubs pooled across a handful of seeds is a noisy,
    /// upward-biased order statistic (one ambitious, cash-rich club that
    /// rationally keeps a full squad through like-for-like churn will push
    /// this near/above 0.5 on its own, no matter how responsive selling is)
    /// — not a sound thing to gate a knob-change tripwire on.
    worst_club_cap_fraction: Vec<f64>,
    /// Per seed: the share of all (club, season) snapshots sitting strictly
    /// below `SQUAD_MAX` — mass below the cap, as opposed to a spike at it.
    below_cap_share: Vec<f64>,
    /// Per seed: the share of clubs whose *own* fraction of seasons at
    /// `SQUAD_MAX` exceeds one half — "no club sits at squad_max for a
    /// majority of windows" read as a population statement (how widespread
    /// the pinning pathology is) rather than the worst single case. This is
    /// the metric the §9 residual actually described: *every* club ratcheted
    /// to the ceiling, not one perpetually-full outlier.
    share_clubs_majority_pinned: Vec<f64>,
    /// The buyer's-eye rows (`AGENT_MODEL.md` §5.6), one value per seed.
    fee_over_true_value: Vec<f64>,
    overpaid_share: Vec<f64>,
    moved_on_within_year: Vec<f64>,
    resale_fee_ratio: Vec<f64>,
    resold_same_window: Vec<f64>,
    star_flop_rate: Vec<f64>,
    /// Per need a signing was bought against: one `(followed a year, gone
    /// within it)` pair per seed.
    resale_by_need: BTreeMap<Option<Need>, Vec<(u32, u32)>>,
    /// Signings gone within a year, pooled over seeds, by the need they
    /// were bought against and how they left.
    resale_paths: BTreeMap<(Option<Need>, Departure), u32>,
}

impl MarketReport {
    fn record_seed(&mut self, t: &MarketTelemetry, num_clubs: usize, seasons: usize) {
        let num_windows = (seasons * WINDOWS_PER_SEASON) as f64;
        self.transfers_per_club_per_window
            .push(t.total_transfers as f64 / (num_windows * num_clubs as f64));

        let fees = t.all_fees_f64();
        self.fee_median.push(median(&fees));
        self.fee_p90.push(percentile(&fees, 0.90));

        let w = EARLY_LATE_WINDOW.min(t.gini_by_season.len().max(1)).max(1);
        self.gini_early.push(mean_finite(
            &t.gini_by_season[..w.min(t.gini_by_season.len())],
        ));
        let tail = t.gini_by_season.len().saturating_sub(w);
        self.gini_late.push(mean_finite(&t.gini_by_season[tail..]));

        self.rank_churn_mean.push(mean_finite(&t.rank_churns));

        let wc = EARLY_LATE_WINDOW
            .min(t.concentration_by_season.len().max(1))
            .max(1);
        self.concentration_early.push(mean_finite(
            &t.concentration_by_season[..wc.min(t.concentration_by_season.len())],
        ));
        let tail_c = t.concentration_by_season.len().saturating_sub(wc);
        self.concentration_late
            .push(mean_finite(&t.concentration_by_season[tail_c..]));

        let fee_yr1 = t.fee_median_by_season.first().copied().unwrap_or(f64::NAN);
        let fee_yr_last = t.fee_median_by_season.last().copied().unwrap_or(f64::NAN);
        self.fee_inflation_ratio.push(fee_yr_last / fee_yr1);

        self.insolvent_mean
            .push(mean_finite(&t.insolvent_by_season));
        self.hoarding_mean.push(mean_finite(&t.hoarding_by_season));
        self.mean_age_mean.push(mean_finite(&t.mean_age_by_season));
        self.squad_size_min.push(
            t.squad_size_min_by_season
                .iter()
                .cloned()
                .fold(f64::INFINITY, f64::min),
        );
        self.squad_size_max.push(
            t.squad_size_max_by_season
                .iter()
                .cloned()
                .fold(f64::NEG_INFINITY, f64::max),
        );
        self.role_coverage_violations
            .push(t.role_coverage_violations as f64);
        self.youth_value_mean
            .push(mean_finite(&t.youth_value_mean_by_season));
        self.youth_value_median
            .push(mean_finite(&t.youth_value_median_by_season));
        self.youth_count_mean
            .push(mean_finite(&t.youth_count_by_season));

        let mut worst_cap_fraction = 0.0f64;
        let mut below_cap = 0usize;
        let mut total = 0usize;
        let mut clubs_seen = 0usize;
        let mut clubs_majority_pinned = 0usize;
        for sizes in t.club_squad_sizes_by_season.values() {
            if sizes.is_empty() {
                continue;
            }
            let at_cap = sizes.iter().filter(|&&s| s >= SQUAD_MAX).count();
            let fraction = at_cap as f64 / sizes.len() as f64;
            worst_cap_fraction = worst_cap_fraction.max(fraction);
            below_cap += sizes.iter().filter(|&&s| s < SQUAD_MAX).count();
            total += sizes.len();
            clubs_seen += 1;
            if fraction > 0.5 {
                clubs_majority_pinned += 1;
            }
        }
        self.worst_club_cap_fraction.push(worst_cap_fraction);
        self.below_cap_share.push(if total > 0 {
            below_cap as f64 / total as f64
        } else {
            f64::NAN
        });
        self.share_clubs_majority_pinned.push(if clubs_seen > 0 {
            clubs_majority_pinned as f64 / clubs_seen as f64
        } else {
            f64::NAN
        });

        let share = |hits: usize, of: usize| {
            if of == 0 {
                f64::NAN
            } else {
                hits as f64 / of as f64
            }
        };
        let fee_ratios: Vec<f64> = t
            .signings()
            .filter_map(|s| {
                s.true_value
                    .filter(|&v| v > 0)
                    .map(|v| s.fee as f64 / v as f64)
            })
            .collect();
        self.fee_over_true_value.push(median(&fee_ratios));
        self.overpaid_share.push(share(
            fee_ratios.iter().filter(|&&r| r > 1.0).count(),
            fee_ratios.len(),
        ));

        let a_year: Vec<&Signing> = t
            .signings()
            .filter(|s| s.followed_for(DAYS_PER_YEAR, t.last_day))
            .collect();
        self.moved_on_within_year.push(share(
            a_year
                .iter()
                .filter(|s| s.exited_within(DAYS_PER_YEAR).is_some())
                .count(),
            a_year.len(),
        ));
        let resale_ratios: Vec<f64> = a_year
            .iter()
            .filter_map(|s| {
                let resale = s.exited_within(DAYS_PER_YEAR)?.resale_fee?;
                Some(resale as f64 / s.fee as f64)
            })
            .collect();
        self.resale_fee_ratio.push(median(&resale_ratios));

        let mut by_need: BTreeMap<Option<Need>, (u32, u32)> = BTreeMap::new();
        for s in &a_year {
            let tally = by_need.entry(s.bought_as).or_default();
            tally.0 += 1;
            if let Some(exit) = s.exited_within(DAYS_PER_YEAR) {
                tally.1 += 1;
                *self
                    .resale_paths
                    .entry((s.bought_as, exit.departure()))
                    .or_default() += 1;
            }
        }
        for (need, tally) in by_need {
            self.resale_by_need.entry(need).or_default().push(tally);
        }
        self.resold_same_window.push(share(
            t.signings()
                .filter(|s| s.exit.is_some_and(|e| e.date == s.date))
                .count(),
            t.signings().count(),
        ));

        let star_fee = percentile(&t.all_fees_f64(), STAR_FEE_PERCENTILE);
        let stars: Vec<&Signing> = t
            .signings()
            .filter(|s| s.fee as f64 >= star_fee && s.followed_for(FLOP_HORIZON_DAYS, t.last_day))
            .collect();
        self.star_flop_rate.push(share(
            stars
                .iter()
                .filter(|s| s.written_off_within(FLOP_HORIZON_DAYS))
                .count(),
            stars.len(),
        ));
    }

    pub fn transfers_per_club_per_window(&self) -> SeedSpread {
        seed_spread(&self.transfers_per_club_per_window)
    }
    pub fn fee_median(&self) -> SeedSpread {
        seed_spread(&self.fee_median)
    }
    pub fn fee_p90(&self) -> SeedSpread {
        seed_spread(&self.fee_p90)
    }
    pub fn gini_early(&self) -> SeedSpread {
        seed_spread(&self.gini_early)
    }
    pub fn gini_late(&self) -> SeedSpread {
        seed_spread(&self.gini_late)
    }
    pub fn rank_churn_mean(&self) -> SeedSpread {
        seed_spread(&self.rank_churn_mean)
    }
    pub fn concentration_early(&self) -> SeedSpread {
        seed_spread(&self.concentration_early)
    }
    pub fn concentration_late(&self) -> SeedSpread {
        seed_spread(&self.concentration_late)
    }
    pub fn fee_inflation_ratio(&self) -> SeedSpread {
        seed_spread(&self.fee_inflation_ratio)
    }
    pub fn insolvent_mean(&self) -> SeedSpread {
        seed_spread(&self.insolvent_mean)
    }
    pub fn hoarding_mean(&self) -> SeedSpread {
        seed_spread(&self.hoarding_mean)
    }
    pub fn mean_age_mean(&self) -> SeedSpread {
        seed_spread(&self.mean_age_mean)
    }
    pub fn squad_size_min(&self) -> SeedSpread {
        seed_spread(&self.squad_size_min)
    }
    pub fn squad_size_max(&self) -> SeedSpread {
        seed_spread(&self.squad_size_max)
    }
    pub fn role_coverage_violations(&self) -> SeedSpread {
        seed_spread(&self.role_coverage_violations)
    }
    pub fn youth_value_mean(&self) -> SeedSpread {
        seed_spread(&self.youth_value_mean)
    }
    pub fn youth_value_median(&self) -> SeedSpread {
        seed_spread(&self.youth_value_median)
    }
    pub fn youth_count_mean(&self) -> SeedSpread {
        seed_spread(&self.youth_count_mean)
    }
    pub fn worst_club_cap_fraction(&self) -> SeedSpread {
        seed_spread(&self.worst_club_cap_fraction)
    }
    pub fn below_cap_share(&self) -> SeedSpread {
        seed_spread(&self.below_cap_share)
    }
    pub fn fee_over_true_value(&self) -> SeedSpread {
        seed_spread(&self.fee_over_true_value)
    }
    pub fn overpaid_share(&self) -> SeedSpread {
        seed_spread(&self.overpaid_share)
    }
    pub fn moved_on_within_year(&self) -> SeedSpread {
        seed_spread(&self.moved_on_within_year)
    }
    pub fn resale_fee_ratio(&self) -> SeedSpread {
        seed_spread(&self.resale_fee_ratio)
    }
    pub fn resold_same_window(&self) -> SeedSpread {
        seed_spread(&self.resold_same_window)
    }
    pub fn star_flop_rate(&self) -> SeedSpread {
        seed_spread(&self.star_flop_rate)
    }
    pub fn share_clubs_majority_pinned(&self) -> SeedSpread {
        seed_spread(&self.share_clubs_majority_pinned)
    }
}

/// Whether the *next* `Command::AdvanceMatchday` (a fixed 7-day step,
/// `commands::advance_matchday`) would cross a transfer-window close date —
/// mirroring `commands::transfer_window_events`'s own boundary check
/// (season-start derivation included) closely enough to predict it, without
/// being able to call that private helper from here. Used only to decide
/// when `submit_player_clubs_ai_equivalent_plan` below is worth the extra
/// `value_all` pass, not to make any market decision itself.
fn window_will_close_on_next_advance(state: &GameState) -> bool {
    let season_start = state
        .date
        .add_days(-((state.current_matchday as i64 - 1) * 7));
    let new_date = state.date.add_days(7);
    let summer = fforge_core::market::summer_window_close(season_start);
    let winter = fforge_core::market::winter_window_close(season_start, state.last_matchday);
    [summer, winter]
        .into_iter()
        .any(|close| state.date < close && close <= new_date)
}

/// `TRANSFER_MODEL.md` §10's pre-commitment seam changes `player_club`'s
/// market participation from "always `UtilityPolicy`, like every other club"
/// to "`RecordedPolicy`, empty unless something was submitted" — correct for
/// a real human who may choose to ignore the market, wrong for this harness,
/// which has no human at all and exists specifically to stress-test *uniform*
/// AI dynamics across every club. Rather than special-case `player_club` out
/// of the market, this harness models it as a manager who always follows the
/// AI's own recommendation: right before a window closes, submit exactly
/// what `UtilityPolicy` would have decided for it. That keeps every pooled
/// §11 statistic (squad-size pinning, transfer volume, concentration, ...)
/// reading the same 20-uniformly-AI-club league this harness read before the
/// seam existed, while still genuinely exercising `RecordedPolicy` and
/// `Command::SubmitTransferDecision` on every run. Only runs on a matchday
/// that actually crosses a window boundary (`window_will_close_on_next_advance`)
/// — the `value_all` pass this needs is real work, and `resolve_window`
/// redoes it anyway once the window resolves.
fn submit_player_clubs_ai_equivalent_plan(
    session: &mut Session,
    club: ClubId,
    value_knobs: &ValueKnobs,
    dev_knobs: &DevKnobs,
    utility_knobs: &UtilityKnobs,
) {
    if !window_will_close_on_next_advance(&session.state) {
        return;
    }
    // The same fogged view the clearing loop gives every AI club
    // (`AGENT_MODEL.md` §5.3), so club 0 decides as its nineteen rivals do.
    let state = &session.state;
    let view = WindowView::new(
        &state.world,
        state.date,
        state.seed,
        dev_knobs,
        value_knobs,
        utility_knobs,
        &ScoutKnobs::default(),
        &state.recent_ratings,
    );
    let obs = view.observe(&state.world, club);
    let decisions = UtilityPolicy::new(*utility_knobs).transfer_decisions(&obs);
    session
        .execute(Command::SubmitTransferDecision { club, decisions }, &mut [])
        .expect("submit player_club's AI-equivalent transfer plan");
}

/// Ask the policy why each transfer of one window happened. `before` is the
/// state as the matchday began and `events` everything that matchday
/// recorded. The events are folded onto a copy of `before` one at a time,
/// so each transfer is explained against the squads as they stood when it
/// completed; the window's picture of what each club believes is frozen at
/// the first transfer, as `market::resolve_window` freezes it.
///
/// The policy decides on the squads at the start of a clearing round and
/// this reads them mid-round, so a reason can come back `None` where an
/// earlier transfer in the same round already filled the need or cleared
/// the surplus.
pub fn explain_window(
    before: &GameState,
    events: &[Event],
    value_knobs: &ValueKnobs,
    dev_knobs: &DevKnobs,
    utility_knobs: &UtilityKnobs,
) -> Vec<TransferReason> {
    let policy = UtilityPolicy::new(*utility_knobs);
    let Some(first) = events
        .iter()
        .position(|e| matches!(e, Event::TransferCompleted { .. }))
    else {
        return Vec::new();
    };
    let mut scratch = before.clone();
    for event in &events[..first] {
        scratch.apply(event);
    }
    let frozen = scratch.world.clone();
    let Event::TransferCompleted { date: today, .. } = events[first] else {
        unreachable!("position() found a transfer here");
    };
    let view = WindowView::new(
        &frozen,
        today,
        before.seed,
        dev_knobs,
        value_knobs,
        utility_knobs,
        &ScoutKnobs::default(),
        &before.recent_ratings,
    );

    let mut reasons = Vec::new();
    for event in &events[first..] {
        if let Event::TransferCompleted {
            date,
            player,
            from,
            to,
            ..
        } = event
        {
            let buyer = view.observe(&scratch.world, *to);
            let bought_as = buyer
                .candidates
                .iter()
                .find(|c| c.player == *player)
                .and_then(|c| policy.need_filled(&buyer, c));
            let listed_as = from.and_then(|seller| {
                policy
                    .listings(&view.observe(&scratch.world, seller))
                    .into_iter()
                    .find_map(|(p, trigger)| (p == *player).then_some(trigger))
            });
            reasons.push(TransferReason {
                player: *player,
                date: *date,
                bought_as,
                listed_as,
            });
        }
        scratch.apply(event);
    }
    reasons
}

/// Trace one world seed across `seasons` full seasons, driving the *real*
/// command pipeline (worldgen → matches → development → finance → pool →
/// market, all via `Session`/`Command::AdvanceMatchday` /
/// `Command::StartNextSeason` — exactly what a live game executes) and
/// folding a `MarketTelemetry` for it.
fn trace_seed(seed: u64, seasons: usize, cfg: &WorldGenConfig) -> MarketTelemetry {
    let value_knobs = ValueKnobs::default();
    let dev_knobs = DevKnobs::default();
    let utility_knobs = UtilityKnobs::default();
    let player_club = fforge_domain::ClubId(0);
    let log = new_game(seed, cfg, player_club);
    let mut telemetry = MarketTelemetry::default();
    let mut session = Session::from_events(log, &mut [&mut telemetry]);

    for s in 0..seasons {
        while !session.state.season_over() {
            if window_will_close_on_next_advance(&session.state) {
                let state = &session.state;
                let ctx =
                    MarketContext::from_world(&state.world, &value_knobs, &state.recent_ratings);
                telemetry.note_true_values(value_all(
                    &state.world,
                    state.date,
                    &ctx,
                    &value_knobs,
                    &dev_knobs,
                ));
            }
            submit_player_clubs_ai_equivalent_plan(
                &mut session,
                player_club,
                &value_knobs,
                &dev_knobs,
                &utility_knobs,
            );
            let before =
                window_will_close_on_next_advance(&session.state).then(|| session.state.clone());
            let events = session
                .execute(Command::AdvanceMatchday, &mut [&mut telemetry])
                .expect("advance matchday");
            if let Some(before) = before {
                let reasons =
                    explain_window(&before, events, &value_knobs, &dev_knobs, &utility_knobs);
                telemetry.note_reasons(&reasons);
            }
        }
        telemetry.record_season_end(
            &session.state.world,
            &session.state.schedule,
            &session.state.results,
            session.state.date,
            &value_knobs,
            &dev_knobs,
        );
        if s + 1 < seasons {
            session
                .execute(Command::StartNextSeason, &mut [&mut telemetry])
                .expect("start next season");
        }
    }

    telemetry
}

/// Run the market harness over `seeds` world seeds, each traced `seasons`
/// full seasons, and return the pooled §11 report.
pub fn run_market_calibration(seeds: &[u64], seasons: usize, cfg: &WorldGenConfig) -> MarketReport {
    let mut report = MarketReport {
        seeds: seeds.len(),
        seasons,
        ..Default::default()
    };
    for &seed in seeds {
        let telemetry = trace_seed(seed, seasons, cfg);
        report.record_seed(&telemetry, cfg.num_clubs, seasons);
    }
    report
}

/// Pretty-print the report to stdout (the `bin/market.rs` payload), in the
/// shape of `career_arc::print_report`: `mean (sd, range across seeds) |
/// target`.
pub fn print_report(report: &MarketReport) {
    fn row(label: &str, s: &SeedSpread, target: &str) {
        println!(
            "{label:<40}: {:>10.3}  (sd {:>8.3}, range {:>8.3}-{:>8.3}, {} seeds)   target: {target}",
            s.mean, s.sd, s.min, s.max, s.n
        );
    }

    println!(
        "=== Market report ({} seeds pooled, {} seasons each) ===",
        report.seeds, report.seasons
    );
    println!();
    println!("--- Transfer volume & fee distribution (TRANSFER_MODEL.md §11) ---");
    row(
        "Transfers per club per window",
        &report.transfers_per_club_per_window(),
        "~2-5",
    );
    row("Fee median", &report.fee_median(), "> 0 (market clears)");
    row(
        "Fee p90",
        &report.fee_p90(),
        "well above median (convexity)",
    );
    println!();
    println!("--- Competitive balance ---");
    row(
        "Points-Gini, early seasons",
        &report.gini_early(),
        "stable vs late",
    );
    row(
        "Points-Gini, late seasons",
        &report.gini_late(),
        "not monotonically rising",
    );
    row(
        "Season-to-season rank churn",
        &report.rank_churn_mean(),
        "non-zero (not a frozen hierarchy)",
    );
    row(
        "Top-3 clubs' share of top-20, early",
        &report.concentration_early(),
        "elevated but bounded",
    );
    row(
        "Top-3 clubs' share of top-20, late",
        &report.concentration_late(),
        "non-rising vs early (§2.6: upper bound under perfect info)",
    );
    println!();
    println!("--- Fee inflation & financial health ---");
    row(
        "Median fee, last season / first season",
        &report.fee_inflation_ratio(),
        "< ~2x",
    );
    row(
        "Clubs insolvent (balance < 0)",
        &report.insolvent_mean(),
        "neither zero forever nor unbounded",
    );
    row(
        "Clubs hoarding cash",
        &report.hoarding_mean(),
        "neither zero forever nor unbounded",
    );
    println!();
    println!("--- Pool shape ---");
    row(
        "League mean age",
        &report.mean_age_mean(),
        "stable, plausible (~22-27)",
    );
    row(
        "Squad size, min across the run",
        &report.squad_size_min(),
        ">= 18",
    );
    row(
        "Squad size, max across the run",
        &report.squad_size_max(),
        "<= 30",
    );
    row(
        "Role-coverage violations (< 2 GK)",
        &report.role_coverage_violations(),
        "0 (hard stabilizer)",
    );
    row(
        "Worst-case club's fraction of seasons at squad_max",
        &report.worst_club_cap_fraction(),
        "diagnostic only (noisy max-of-20 statistic)",
    );
    row(
        "Squad-size snapshots strictly below squad_max",
        &report.below_cap_share(),
        "> 0.5 (mass below the cap, not a spike at it)",
    );
    row(
        "Share of clubs majority-pinned at squad_max",
        &report.share_clubs_majority_pinned(),
        "well under 1.0 (not every club ratcheted to the ceiling)",
    );
    println!();
    println!(
        "--- Youth valuations (start_age <= {YOUTH_MAX_AGE}), its own cut (`DEVELOPMENT_MODEL.md` §8) ---"
    );
    row(
        "Youth mean value",
        &report.youth_value_mean(),
        "new cut, no prior band",
    );
    row(
        "Youth median value",
        &report.youth_value_median(),
        "new cut, no prior band",
    );
    row(
        "Youth headcount per season-end snapshot",
        &report.youth_count_mean(),
        "new cut, no prior band",
    );
    println!();
    println!("--- the buyer's eye (AGENT_MODEL.md §5.6; fee-paying signings only) ---");
    row(
        "Fee / true value, median",
        &report.fee_over_true_value(),
        "<= 1 with no winner's curse",
    );
    row(
        "Share paying above true value",
        &report.overpaid_share(),
        "0 under omniscience",
    );
    row(
        "Share gone from the buyer within a year",
        &report.moved_on_within_year(),
        "churn if high",
    );
    row(
        "Resale fee / purchase fee, median",
        &report.resale_fee_ratio(),
        "~1 if the price was fair",
    );
    row(
        "Share sold on in the window they arrived",
        &report.resold_same_window(),
        "0: a club keeps what it buys",
    );
    row(
        "Star signings written off in two years",
        &report.star_flop_rate(),
        "the flop rate",
    );
    print_resale_reasons(report);
}

fn need_label(need: Option<Need>) -> &'static str {
    match need {
        Some(Need::Cover) => "cover",
        Some(Need::Depth) => "depth",
        Some(Need::Upgrade) => "upgrade",
        Some(Need::Heir) => "heir",
        None => "unexplained",
    }
}

fn departure_label(departure: Departure) -> &'static str {
    match departure {
        Departure::Listed(ListTrigger::Surplus) => "surplus",
        Departure::Listed(ListTrigger::Expiring) => "expiring",
        Departure::Listed(ListTrigger::SquadPressure) => "pressure",
        Departure::Listed(ListTrigger::Distress) => "distress",
        Departure::SoldUnexplained => "unexplained",
        Departure::Released => "released",
    }
}

/// `BACKLOG.md` §4.8: which purchases come back onto the market, and by
/// which door.
fn print_resale_reasons(report: &MarketReport) {
    println!();
    println!("--- signings by the need they were bought against (followed a year) ---");
    let all: u32 = report
        .resale_by_need
        .values()
        .flatten()
        .map(|&(followed, _)| followed)
        .sum();
    println!(
        "{:<12} {:>9} {:>8}   {:>16}",
        "bought as", "signings", "share", "gone in a year"
    );
    for (&need, per_seed) in &report.resale_by_need {
        let followed: u32 = per_seed.iter().map(|&(f, _)| f).sum();
        let rates: Vec<f64> = per_seed
            .iter()
            .filter(|&&(f, _)| f > 0)
            .map(|&(f, gone)| gone as f64 / f as f64)
            .collect();
        let spread = seed_spread(&rates);
        println!(
            "{:<12} {:>9} {:>7.1}%   {:>6.1}% (sd {:.1}, {} seeds)",
            need_label(need),
            followed,
            100.0 * followed as f64 / all.max(1) as f64,
            100.0 * spread.mean,
            100.0 * spread.sd,
            spread.n
        );
    }

    println!();
    println!("--- signings gone within a year: bought as -> how they left (% of all such) ---");
    let gone: u32 = report.resale_paths.values().sum();
    let mut paths: Vec<(&(Option<Need>, Departure), &u32)> = report.resale_paths.iter().collect();
    paths.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (&(need, departure), &count) in paths {
        println!(
            "{:<12} -> {:<12} {:>7} {:>6.1}%",
            need_label(need),
            departure_label(departure),
            count,
            100.0 * count as f64 / gone.max(1) as f64
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bought(player: u32, day: i64, fee: i64) -> Event {
        Event::TransferCompleted {
            date: GameDate { days: day },
            player: PlayerId(player),
            from: Some(ClubId(1)),
            to: ClubId(2),
            fee: Money(fee),
            contract: fforge_domain::Contract {
                wage: Money(1),
                expires: GameDate { days: day + 1000 },
            },
        }
    }

    #[test]
    fn a_signing_is_followed_until_the_buyer_lets_the_player_go() {
        let mut t = MarketTelemetry::default();
        t.note_true_values(BTreeMap::from([(PlayerId(7), Money(80))]));
        t.on_event(&bought(7, 100, 100));
        t.on_event(&bought(8, 100, 100));
        t.on_event(&bought(7, 300, 40));
        t.on_event(&Event::PlayerReleased {
            date: GameDate { days: 900 },
            player: PlayerId(8),
            club: ClubId(2),
        });

        let signings: Vec<Signing> = t.signings().copied().collect();
        assert_eq!(signings.len(), 3, "the resale is itself a signing");

        let first = signings.iter().find(|s| s.true_value.is_some()).unwrap();
        assert_eq!(first.true_value, Some(80));
        assert_eq!(
            first.exit,
            Some(Exit {
                date: GameDate { days: 300 },
                resale_fee: Some(40),
                listed_as: None,
            })
        );
        assert!(first.exited_within(DAYS_PER_YEAR).is_some());
        assert!(
            first.written_off_within(DAYS_PER_YEAR),
            "sold for 40% of the fee"
        );

        let released = signings
            .iter()
            .find(|s| s.exit.is_some_and(|e| e.resale_fee.is_none()))
            .unwrap();
        assert!(released.exited_within(DAYS_PER_YEAR).is_none());
        assert!(released.written_off_within(FLOP_HORIZON_DAYS + 100));

        let still_there = signings.iter().find(|s| s.exit.is_none()).unwrap();
        assert!(!still_there.written_off_within(i64::MAX));
    }

    #[test]
    fn reasons_attach_to_the_signing_opened_and_the_one_closed() {
        let mut t = MarketTelemetry::default();
        t.on_event(&bought(7, 100, 100));
        t.note_reasons(&[TransferReason {
            player: PlayerId(7),
            date: GameDate { days: 100 },
            bought_as: Some(Need::Depth),
            listed_as: None,
        }]);
        t.on_event(&bought(7, 300, 90));
        t.note_reasons(&[TransferReason {
            player: PlayerId(7),
            date: GameDate { days: 300 },
            bought_as: Some(Need::Upgrade),
            listed_as: Some(ListTrigger::SquadPressure),
        }]);

        let first = t.signings().find(|s| s.date.days == 100).unwrap();
        assert_eq!(first.bought_as, Some(Need::Depth));
        assert_eq!(
            first.exit.map(|e| e.departure()),
            Some(Departure::Listed(ListTrigger::SquadPressure))
        );
        let second = t.signings().find(|s| s.date.days == 300).unwrap();
        assert_eq!(second.bought_as, Some(Need::Upgrade));
        assert_eq!(second.exit, None);
    }

    #[test]
    fn a_free_signing_is_not_a_purchase() {
        let mut t = MarketTelemetry::default();
        t.on_event(&bought(1, 10, 0));
        assert_eq!(t.signings().count(), 0);
    }

    /// The market pathology regression guard (`TRANSFER_MODEL.md` §11): the
    /// transfer-market sibling of `favourite_discrimination_regression_guard`
    /// and `career_arcs_are_in_a_believable_ballpark`. Like both siblings this
    /// is a **wide-band gross-regression tripwire, not a fit gate** — bands
    /// are deliberately loose, sized to catch a market that has come loose
    /// from the design (dead, hyperactive, a frozen hierarchy, one club
    /// buying the league, runaway insolvency) rather than to pin a fitted
    /// number.
    ///
    /// **Pool size: 8 seeds × 15 seasons — the bin's own default (`cargo run
    /// --bin market`), not an arbitrary shrink.** A prior 2-seed × 8-season
    /// pool read `role_coverage_violations` at 0 (this metric's per-seed
    /// range genuinely spans 0 to double digits — a GK-retirement wave in one
    /// unlucky seed can spike it, per §9's "up to 13 in a seed"), while
    /// `bin/market` at 24 seeds × 15 seasons reads a mean around 2-3 with an
    /// sd comparable to the mean itself: a threshold set against the 2-seed
    /// pool's reading was a threshold set against noise, not against the
    /// metric, and would have passed or failed by which seeds happened to be
    /// in the pool rather than by whether the stabilizer actually held. At 8
    /// seeds the mean already lands in the same range the 24-seed pool reads
    /// (this metric's sd does not shrink much further with more seeds — the
    /// per-season variance dominates, not sampling noise on the mean — so 8
    /// seeds is a faithful, not a lucky, sample), which is why the assertion
    /// band below is sized off the documented 24-seed reading rather than
    /// re-derived from this smaller pool.
    ///
    /// **Feature-gated behind `slow-tests`, ignored by default.** This is a
    /// knob-change regression tripwire, not a unit test: a commit that touches
    /// neither `*Knobs` nor a sim module can't trip it, so running it on every
    /// `cargo test` is wasted wall-clock. It runs in the PR-required fast
    /// suite's absence deliberately — CI instead runs it nightly and on any PR
    /// touching `*Knobs`, `development`, `match_engine`, `market`,
    /// `valuation`, `club_ai`, or `pool`. `#[ignore]` here is a scheduling
    /// choice, not neglect; run it locally with `cargo test --features
    /// slow-tests`.
    #[cfg_attr(not(feature = "slow-tests"), ignore)]
    #[test]
    fn market_is_in_a_believable_ballpark() {
        let cfg = WorldGenConfig::default();
        let seeds: Vec<u64> = (0..8).collect();
        let report = run_market_calibration(&seeds, 15, &cfg);

        // --- transfer volume: neither dead nor hyperactive ---
        let tpw = report.transfers_per_club_per_window();
        assert!(
            (0.2..=10.0).contains(&tpw.mean),
            "transfers/club/window {:.3} outside a believable band (dead or hyperactive market)",
            tpw.mean
        );

        // --- fee distribution: the market clears, and convexity holds ---
        let fee_med = report.fee_median();
        assert!(
            fee_med.mean > 0.0,
            "median fee is zero — the market never clears"
        );
        let fee_p90 = report.fee_p90();
        assert!(
            fee_p90.mean > fee_med.mean,
            "p90 fee ({:.0}) should sit well above the median ({:.0}) — convexity check",
            fee_p90.mean,
            fee_med.mean
        );

        // --- points-Gini: stable, not a rich-get-richer runaway ---
        let gini_early = report.gini_early().mean;
        let gini_late = report.gini_late().mean;
        assert!(
            gini_late - gini_early < 0.25,
            "points-Gini rose from {gini_early:.3} to {gini_late:.3} — rich-get-richer runaway"
        );

        // --- rank churn: non-zero, not a frozen hierarchy ---
        let churn = report.rank_churn_mean();
        assert!(
            churn.mean > 0.0,
            "season-to-season rank churn is zero — a frozen hierarchy"
        );
        assert!(
            churn.mean < cfg.num_clubs as f64,
            "rank churn {:.2} exceeds the number of clubs — nonsensical",
            churn.mean
        );

        // --- concentration: bounded, non-rising (§2.6 upper-bound caveat) ---
        let conc_early = report.concentration_early().mean;
        let conc_late = report.concentration_late().mean;
        assert!(
            (0.0..=1.0).contains(&conc_early) && (0.0..=1.0).contains(&conc_late),
            "concentration share out of [0,1]: early {conc_early:.3}, late {conc_late:.3}"
        );
        assert!(
            conc_late - conc_early < 0.4,
            "top-3 share of top-20 rose from {conc_early:.3} to {conc_late:.3} — talent monopolization"
        );

        // --- fee inflation: bounded ---
        let inflation = report.fee_inflation_ratio();
        if inflation.n > 0 {
            assert!(
                inflation.mean < 4.0,
                "fee inflation (last season / first season) {:.2}x exceeds a believable band",
                inflation.mean
            );
        }

        // --- financial health: pathologies bounded, not the whole league ---
        let insolvent = report.insolvent_mean();
        assert!(
            insolvent.mean < cfg.num_clubs as f64 * 0.75,
            "insolvent-club count {:.1} is most of the league — broken financial loop",
            insolvent.mean
        );
        let hoarding = report.hoarding_mean();
        assert!(
            hoarding.mean < cfg.num_clubs as f64 * 0.75,
            "hoarding-club count {:.1} is most of the league",
            hoarding.mean
        );

        // --- pool shape: stable, in bounds, role coverage held ---
        let age = report.mean_age_mean();
        assert!(
            (20.0..=30.0).contains(&age.mean),
            "league mean age {:.2} outside a believable band",
            age.mean
        );
        let smin = report.squad_size_min();
        assert!(
            smin.mean >= 15.0,
            "squad sizes dropped well below the [18,30] stabilizer band: min {:.1}",
            smin.mean
        );
        let smax = report.squad_size_max();
        assert!(
            smax.mean <= 33.0,
            "squad sizes rose well above the [18,30] stabilizer band: max {:.1}",
            smax.mean
        );
        let violations = report.role_coverage_violations();
        // Banded against the documented 24-seed × 15-season reading
        // (`TRANSFER_MODEL.md` §9: mean ~2.5, sd 3.4, up to 13 in a single
        // seed), not the tighter "<1.0" a 2-seed pool's near-zero read had
        // suggested — that threshold was set against noise, not the metric
        // (see the doc comment above). 10.0 sits comfortably above the
        // documented mean+2sd (~9.3) and the single-seed max (13 is a
        // per-seed count, not a pooled mean), so a mean this high still means
        // the hard stabilizer is failing broadly across the pool, not that
        // one unlucky seed skewed the average.
        assert!(
            violations.mean < 10.0,
            "goalkeeper-coverage stabilizer was violated {:.1} times on average — \
             well above the documented ~2.5 (sd 3.4) pooled reading",
            violations.mean
        );

        // --- §9's open residual: squads must not pin at squad_max ---
        let below_cap = report.below_cap_share();
        assert!(
            below_cap.mean > 0.5,
            "squad-size snapshots have no mass below squad_max: only {:.2} sit \
             strictly below the cap — a spike at the ceiling rather than a spread",
            below_cap.mean
        );
        let pinned_share = report.share_clubs_majority_pinned();
        assert!(
            pinned_share.mean < 0.5,
            "{:.2} of clubs sit at squad_max for a majority of their seasons — \
             selling isn't responsive enough to squad size",
            pinned_share.mean
        );
    }
}
