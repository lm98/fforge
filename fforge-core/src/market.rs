//! The Phase-4 market: the simultaneous, deferred-acceptance clearing loop
//! (`TRANSFER_MODEL.md` §5) and the window mechanics that trigger it (§7).
//!
//! `DESIGN.md` §10's standing sequential-vs-simultaneous question is resolved
//! here in favour of simultaneous, deferred-acceptance rounds (Gale–Shapley-
//! flavoured): every club decides its one bid for the round against the
//! *same* frozen snapshot, contention is resolved by the selling club's own
//! ranking, and the player himself consents or refuses — never "whoever bid
//! first wins," which would make the pathology harness (§11) measure its own
//! iteration order instead of the economics.
//!
//! **Events and Trace, exactly like `MatchOutcome`** (`MATCH_MODEL.md` §7):
//! `WindowOutcome.transfers` is the resolved outcome a caller folds into
//! `Event::TransferCompleted`; `rejected_bids`, `valuations`, and
//! `unfilled_needs` are a Trace — the journalist agent's raw material
//! ("City's third bid rejected") — kept, but never fed to the fold.
//!
//! **Human decisions** (`TRANSFER_MODEL.md` §10's pre-commitment model,
//! promoted from "the seam is left open"): `resolve_window`'s `human_club`
//! substitutes `club_ai::RecordedPolicy` for that one club, replaying
//! whatever was pre-committed via `Command::SubmitTransferDecision`
//! unchanged in every round; every other club still runs `UtilityPolicy`.
//! `filter_affordable` applies the same resolve-time affordability/squad-
//! bounds/GK-floor gate to every club's decisions regardless of which
//! policy produced them — a no-op for `UtilityPolicy` output (already
//! compliant by construction), the actual gate for a human's plan, which
//! bypasses `UtilityPolicy`'s own producer-side filtering entirely.
//!
//! **Scope fence.** No loans, no negotiation rounds, no transfer clauses, no
//! in-window re-bidding — a submitted plan is replayed exactly as
//! submitted, never adapted round to round (that is what "pre-commitment"
//! means here).

pub mod calibrate;
pub use calibrate::{print_report, run_market_calibration, MarketReport, MarketTelemetry, SeedSpread};

use crate::club_ai::{
    observe, ClubObservation, ClubPolicy, RecordedPolicy, TransferDecision, UtilityKnobs,
    UtilityPolicy,
};
use crate::development::DevKnobs;
use crate::rng::derive_stream;
use crate::state::apply_transfer_completed;
use crate::valuation::{value_all, MarketContext, ValueKnobs};
use crate::worldgen::wage_for_quality;
use fforge_domain::{
    best_role, date::DAYS_PER_YEAR, ClubId, Contract, GameDate, Money, PlayerId, Role, World,
    ROLE_WEIGHTS,
};
use std::collections::BTreeMap;

/// Tag namespace for the per-window transfer-market RNG stream
/// (`rng::derive_stream`), following the existing `"MATC"`/`"DEVE"`
/// convention. The window index is OR'd into the low bits, so every window
/// draws an independent stream.
pub const TRANSFER_STREAM_NS: u64 = 0x5452_414E_0000_0000; // "TRAN"

/// Hard cap on clearing rounds (§5 step 5). Termination is guaranteed by
/// construction (a bounded loop), but the doc's guarantee is that a
/// well-behaved window reaches its own fixpoint — no bids produced — well
/// before this fires; the cap exists for adversarial inputs.
pub const MAX_ROUNDS: u32 = 12;

/// Summer window: closes this many days after `SeasonStarted` (§7).
pub const SUMMER_WINDOW_CLOSE_DAYS: i64 = 30;

/// Winter window: half of its ~30-day span (§7) — the window runs
/// `[midpoint − this, midpoint + this]` around the schedule midpoint.
pub const WINTER_WINDOW_HALF_SPAN_DAYS: i64 = 15;

/// The market's own knob table (`TRANSFER_MODEL.md` §9-adjacent — a
/// plausibility-picked starting point, sibling of `DevKnobs`/`ValueKnobs`/
/// `UtilityKnobs`/`FinanceKnobs`; the §11 harness re-fits it). Governs the
/// one piece of the clearing loop the design doc leaves unspecified in
/// detail: the player's own consent roll (step 3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketKnobs {
    /// A player refuses a wage cut deeper than this ratio of his current
    /// wage; irrelevant (no barrier) for a free agent with no prior wage.
    pub min_wage_accept_ratio: f64,
    /// Logistic-free spread: how many reputation points of shortfall below
    /// the player's expectation it takes to fully sink his consent odds.
    pub reputation_tolerance: f64,
    /// Best-role CA span the reputation expectation is linear over.
    pub ca_span_min: f64,
    pub ca_span_max: f64,
    /// The buyer-reputation a player expects at `ca_span_min`/`ca_span_max`
    /// — better players want more reputable clubs.
    pub reputation_expectation_at_min_ca: f64,
    pub reputation_expectation_at_max_ca: f64,
    /// Length of a freshly agreed contract (no negotiation model — v1 simply
    /// carries the player's existing wage forward on a fresh term, or
    /// synthesizes one via `worldgen::wage_for_quality` for a free agent).
    pub new_contract_years: f64,
}

impl Default for MarketKnobs {
    fn default() -> Self {
        MarketKnobs {
            min_wage_accept_ratio: 0.90,
            reputation_tolerance: 40.0,
            ca_span_min: 30.0,
            ca_span_max: 90.0,
            reputation_expectation_at_min_ca: 15.0,
            reputation_expectation_at_max_ca: 80.0,
            new_contract_years: 3.0,
        }
    }
}

/// One completed transfer — the only part of a `WindowOutcome` that becomes
/// an event (`TRANSFER_MODEL.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub player: PlayerId,
    pub from: Option<ClubId>,
    pub to: ClubId,
    pub fee: Money,
    pub contract: Contract,
}

/// Why a bid did not land this player for that club this round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Ranked below the bid that won this target this round by the selling
    /// club's own ranking (fee, then buyer reputation, then `ClubId`) —
    /// never reached the player-consent step.
    Outranked,
    /// Reached the player-consent step and was refused (wage or buyer
    /// reputation short of the player's own threshold).
    PlayerRefused,
}

/// One non-winning bid — Trace material (`MATCH_MODEL.md` §7's pattern): the
/// journalist agent's raw material, kept out of the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectedBid {
    pub round: u32,
    pub player: PlayerId,
    pub from: Option<ClubId>,
    pub bidder: ClubId,
    pub price: Money,
    pub reason: RejectReason,
}

/// The result of resolving one window (§5). Only `transfers` folds into
/// events; everything else is a Trace, exactly as `MatchOutcome::stream` is
/// (`MATCH_MODEL.md` §7) — kept for the journalist agent and diagnostics,
/// never persisted through the event log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOutcome {
    pub transfers: Vec<Transfer>,
    pub rejected_bids: Vec<RejectedBid>,
    /// The frozen §2.7 valuation cache this whole window priced against.
    pub valuations: BTreeMap<PlayerId, Money>,
    /// Each club's still-positive needs at window close (post-transfer).
    pub unfilled_needs: BTreeMap<ClubId, Vec<Role>>,
    /// How many rounds actually ran (`<= MAX_ROUNDS`) — diagnostic only.
    pub rounds_used: u32,
}

/// Resolve one transfer window (`TRANSFER_MODEL.md` §5): the simultaneous,
/// deferred-acceptance clearing loop. Pure function of its inputs — no
/// wall-clock, no I/O, and its only randomness is `derive_stream(seed,
/// TRANSFER_STREAM_NS | window_index)`, drawn unconditionally in `ClubId`/
/// `PlayerId` order so replay reproduces an identical `WindowOutcome`.
///
/// `world` is never mutated — a working copy absorbs each round's completed
/// transfers so later rounds see updated squads, cash, and committed wages,
/// exactly as a real window would.
///
/// `human_club`/`human_decisions` are §10's pre-commitment seam: when
/// `human_club` is `Some(club)`, that one club's decisions each round are
/// `human_decisions` replayed verbatim via `RecordedPolicy` rather than
/// `UtilityPolicy`'s own fresh-each-round reasoning — every other club is
/// unaffected. `human_club: None` (every existing caller before this seam
/// existed, and any context that wants a pure all-AI league) reproduces the
/// prior behaviour exactly.
#[allow(clippy::too_many_arguments)]
pub fn resolve_window(
    world: &World,
    today: GameDate,
    seed: u64,
    window_index: u64,
    dev: &DevKnobs,
    value_knobs: &ValueKnobs,
    utility_knobs: &UtilityKnobs,
    market_knobs: &MarketKnobs,
    human_club: Option<ClubId>,
    human_decisions: &[TransferDecision],
) -> WindowOutcome {
    // Step 1 (§5): freeze the world snapshot and the valuation cache. Every
    // club's `Candidate.value` this whole window comes from this one map,
    // computed once — §2.7's simultaneity guarantee.
    let ctx = MarketContext::from_world(world, value_knobs);
    let valuations = value_all(world, today, &ctx, value_knobs, dev);
    let ai_policy = UtilityPolicy::new(*utility_knobs);
    let human_policy = RecordedPolicy::new(human_decisions.to_vec());

    let mut work_world = world.clone();
    let mut transfers = Vec::new();
    let mut rejected_bids = Vec::new();
    let mut rng = derive_stream(seed, TRANSFER_STREAM_NS | window_index);
    let mut rounds_used = 0u32;
    // Deferred acceptance never re-proposes a refused pair (classic
    // Gale-Shapley): once a player refuses a club's offer, re-asking with an
    // identical offer could only repeat the same answer, so the pair is
    // struck for the rest of this window. This is what actually guarantees
    // convergence well inside `MAX_ROUNDS` in practice — the cap exists for
    // truly adversarial inputs, not as the normal stopping mechanism.
    let mut refused: BTreeMap<ClubId, std::collections::BTreeSet<PlayerId>> = BTreeMap::new();

    for round in 0..MAX_ROUNDS {
        rounds_used = round + 1;
        // ClubId order — a `BTreeMap`, never a `HashMap` — so decisions are
        // gathered in the same fixed order every run.
        let club_ids: Vec<ClubId> = work_world.clubs.keys().copied().collect();

        // Step 2 (§5): every club decides against the SAME round-start
        // snapshot — genuinely simultaneous, not "whoever goes first."
        let mut listed: BTreeMap<PlayerId, ClubId> = BTreeMap::new();
        let mut club_decisions: Vec<(ClubId, Vec<TransferDecision>)> = Vec::new();
        for &club in &club_ids {
            let obs = observe(&work_world, club, today, &valuations, dev, utility_knobs);
            let raw_decisions = if Some(club) == human_club {
                human_policy.transfer_decisions(&obs)
            } else {
                ai_policy.transfer_decisions(&obs)
            };
            let decisions = filter_affordable(&obs, &raw_decisions, utility_knobs);
            for d in &decisions {
                if let TransferDecision::List { player } = d {
                    listed.insert(*player, club);
                }
            }
            club_decisions.push((club, decisions));
        }

        // One bid per club (§5 step 2): the top-ranked decision that is
        // actually biddable — a free agent, or a player its own club has
        // listed this round (the supply side `club_ai::UtilityPolicy`
        // already computed; not re-derived here).
        let mut bids: BTreeMap<PlayerId, Vec<(ClubId, Money)>> = BTreeMap::new();
        for (club, decisions) in &club_decisions {
            for d in decisions {
                let TransferDecision::Bid {
                    player,
                    from,
                    price,
                    ..
                } = d
                else {
                    continue;
                };
                let already_refused = refused.get(club).is_some_and(|s| s.contains(player));
                let biddable = !already_refused
                    && match from {
                        None => true,
                        Some(seller) => listed.get(player) == Some(seller),
                    };
                if biddable {
                    bids.entry(*player).or_default().push((*club, *price));
                    break;
                }
            }
        }

        if bids.is_empty() {
            break; // fixpoint (§5 step 5): no bids, nothing left to resolve
        }

        // Steps 3-4 (§5): resolve contention per target, in `PlayerId`
        // order — deterministic, never a `HashMap`.
        for (player, mut offers) in bids {
            // The selling club's ranking: fee desc, buyer reputation desc,
            // `ClubId` asc. Content-driven; `ClubId` only breaks an exact
            // tie, so no club is favoured merely for existing first.
            offers.sort_by(|a, b| {
                b.1.0
                    .cmp(&a.1.0)
                    .then_with(|| {
                        let rep_a = work_world.club(a.0).reputation;
                        let rep_b = work_world.club(b.0).reputation;
                        rep_b.cmp(&rep_a)
                    })
                    .then_with(|| a.0.cmp(&b.0))
            });

            let seller = work_world.club_of(player);
            let p = work_world.player(player);
            let (_, ca) = best_role(&p.attributes, &ROLE_WEIGHTS);
            let current_wage = p.contract.map(|c| c.wage);

            // The deferred-acceptance step: ask each ranked bidder in turn.
            // The RNG draw happens for every bidder we reach — never
            // skipped because an earlier shortcut could avoid needing it —
            // mirroring `tick_changes`'s discipline of keeping stream
            // position independent of which values happen to matter.
            let mut winner: Option<usize> = None;
            for (i, &(bidder, fee)) in offers.iter().enumerate() {
                let buyer_reputation = work_world.club(bidder).reputation;
                let prob =
                    consent_probability(fee, current_wage, buyer_reputation, ca, market_knobs);
                let roll = rng.f64();
                if roll < prob {
                    winner = Some(i);
                    break;
                }
                rejected_bids.push(RejectedBid {
                    round,
                    player,
                    from: seller,
                    bidder,
                    price: fee,
                    reason: RejectReason::PlayerRefused,
                });
                refused.entry(bidder).or_default().insert(player);
            }

            if let Some(i) = winner {
                let (bidder, fee) = offers[i];
                let wage = current_wage.unwrap_or_else(|| wage_for_quality(&mut rng, ca));
                let contract = Contract {
                    wage,
                    expires: today
                        .add_days((market_knobs.new_contract_years * DAYS_PER_YEAR as f64) as i64),
                };
                apply_transfer_completed(&mut work_world, player, seller, bidder, fee, contract);
                transfers.push(Transfer {
                    player,
                    from: seller,
                    to: bidder,
                    fee,
                    contract,
                });
                // Ranked below the winner: never reached the consent step.
                for &(bidder, price) in offers.iter().skip(i + 1) {
                    rejected_bids.push(RejectedBid {
                        round,
                        player,
                        from: seller,
                        bidder,
                        price,
                        reason: RejectReason::Outranked,
                    });
                }
            }
        }
    }

    let unfilled_needs = final_unfilled_needs(&work_world, today, &valuations, dev, &ai_policy);

    WindowOutcome {
        transfers,
        rejected_bids,
        valuations,
        unfilled_needs,
        rounds_used,
    }
}

/// Resolve-time sanity filter (§10's pre-commitment model): the same
/// affordability, wage-headroom, squad-bounds, GK-floor, and availability
/// stabilizers `UtilityPolicy`'s own producer-side filtering already
/// guarantees for every AI club's decisions apply here too, uniformly, to
/// whichever policy produced `decisions` — a no-op for `UtilityPolicy`
/// output (already compliant by construction, so this never changes a
/// pre-existing AI-vs-AI outcome), the actual gate for a human's
/// `RecordedPolicy` plan, which bypasses that producer-side filtering by
/// submitting decisions directly and — being static, replayed unchanged
/// every round — can go stale *within* a single window, not just between
/// windows. Checked against this round's own `obs` (the round-start
/// snapshot), so a plan that was affordable, or a target that was
/// available, at submission time but no longer is by the time its window
/// resolves — or by a later round within the same window — is silently
/// dropped: never an error, never a panic.
///
/// **Availability** is the subtle one: a `Bid`'s `from: None` claims "this
/// player is a free agent," and the bid-collection loop below trusts that
/// claim unconditionally (`from: None => true`, no re-check) because
/// `UtilityPolicy` regenerates it fresh every round off the live
/// `ClubObservation`, so it is never stale *for that policy*. A static
/// `RecordedPolicy` plan has no such guarantee: if the claimed free agent
/// gets signed by a third club in an earlier round of the same window, the
/// stale `from: None` would otherwise stay "biddable" forever after,
/// contending for a player who was never on offer to that bidder. Requiring
/// `candidate.club == from` here — the *current* observed owner, matched
/// against whatever the decision itself claims — closes that hole for any
/// policy, not just this one.
fn filter_affordable(
    obs: &ClubObservation,
    decisions: &[TransferDecision],
    knobs: &UtilityKnobs,
) -> Vec<TransferDecision> {
    let spendable = obs.balance.0 - knobs.cash_reserve_floor.0;
    let wage_room = obs.wage_budget.0 - obs.committed_wages.0;
    let mut gk_count = obs
        .squad
        .iter()
        .filter(|m| m.natural_role == Role::Gk)
        .count();
    let mut squad_len = obs.squad.len();

    let mut kept = Vec::new();
    for d in decisions {
        match *d {
            TransferDecision::Bid {
                player,
                from,
                price,
                ..
            } => {
                if squad_len >= knobs.squad_max || price.0 < 0 || price.0 > spendable {
                    continue;
                }
                let Some(candidate) = obs.candidates.iter().find(|c| c.player == player) else {
                    continue;
                };
                if candidate.club != from || candidate.wage.0 > wage_room {
                    continue;
                }
                kept.push(*d);
            }
            TransferDecision::List { player } => {
                if squad_len <= knobs.squad_min {
                    continue;
                }
                let Some(member) = obs.squad.iter().find(|m| m.player == player) else {
                    continue;
                };
                if member.natural_role == Role::Gk {
                    if gk_count <= knobs.min_goalkeepers {
                        continue;
                    }
                    gk_count -= 1;
                }
                squad_len -= 1;
                kept.push(*d);
            }
        }
    }
    kept
}

/// Probability a player consents to a move (§5 step 3): independent wage and
/// buyer-reputation adequacy terms, each clamped to `[0, 1]`, multiplied. A
/// free agent has no prior wage to protect, so the wage term is not a
/// barrier for him.
fn consent_probability(
    offered_wage: Money,
    current_wage: Option<Money>,
    buyer_reputation: u8,
    ca: u8,
    knobs: &MarketKnobs,
) -> f64 {
    let wage_term = match current_wage {
        Some(w) if w.0 > 0 => {
            let ratio = offered_wage.0 as f64 / w.0 as f64;
            ((ratio - knobs.min_wage_accept_ratio) / (1.0 - knobs.min_wage_accept_ratio))
                .clamp(0.0, 1.0)
        }
        _ => 1.0,
    };

    let span =
        ((ca as f64 - knobs.ca_span_min) / (knobs.ca_span_max - knobs.ca_span_min)).clamp(0.0, 1.0);
    let expected_rep = knobs.reputation_expectation_at_min_ca
        + (knobs.reputation_expectation_at_max_ca - knobs.reputation_expectation_at_min_ca) * span;
    let rep_term = (1.0 - (expected_rep - buyer_reputation as f64) / knobs.reputation_tolerance)
        .clamp(0.0, 1.0);

    (wage_term * rep_term).clamp(0.0, 1.0)
}

/// Every club's still-positive needs against the post-window world — Trace
/// material for `WindowOutcome`, computed the same way `club_ai` computes
/// need anywhere else (no second encoding).
fn final_unfilled_needs(
    world: &World,
    today: GameDate,
    valuations: &BTreeMap<PlayerId, Money>,
    dev: &DevKnobs,
    policy: &UtilityPolicy,
) -> BTreeMap<ClubId, Vec<Role>> {
    world
        .clubs
        .keys()
        .map(|&club| {
            let obs = observe(world, club, today, valuations, dev, &policy.knobs);
            let needs = policy.needs(&obs);
            let roles: Vec<Role> = Role::ALL
                .iter()
                .copied()
                .filter(|r| needs.get(r).copied().unwrap_or(0.0) > 0.0)
                .collect();
            (club, roles)
        })
        .collect()
}

/// The summer window's close date (§7): `SUMMER_WINDOW_CLOSE_DAYS` after
/// `SeasonStarted`.
pub fn summer_window_close(season_start: GameDate) -> GameDate {
    season_start.add_days(SUMMER_WINDOW_CLOSE_DAYS)
}

/// The winter window's close date (§7): the schedule midpoint plus half its
/// ~30-day span. Matchdays are 7 days apart from `season_start` by
/// construction (`commands::advance_matchday`), so the midpoint matchday's
/// date is derived, never stored.
pub fn winter_window_close(season_start: GameDate, last_matchday: u8) -> GameDate {
    let midpoint_matchday = (last_matchday as i64 / 2).max(1);
    let midpoint_date = season_start.add_days((midpoint_matchday - 1) * 7);
    midpoint_date.add_days(WINTER_WINDOW_HALF_SPAN_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{generate, WorldGenConfig};
    use fforge_domain::{
        Attribute, Attributes, Character, Club, Competition, CompetitionId, DevProfile, Finances,
        Player, NUM_ATTRIBUTES,
    };
    use std::collections::BTreeSet;

    const TODAY: GameDate = GameDate { days: 2030 * 365 };

    /// A player shaped toward `role` (the way `worldgen` shapes attributes),
    /// with an explicit wage and contract length — `contract_years <= 0` is a
    /// free agent (`None`).
    fn mk_player(
        id: u32,
        role: Role,
        base: i32,
        age_years: i64,
        pa: u8,
        wage: i64,
        contract_years: i64,
    ) -> Player {
        let mut vals = [0u8; NUM_ATTRIBUTES];
        for attr in Attribute::ALL {
            let w = ROLE_WEIGHTS.weight(role, attr) as i32;
            vals[attr.index()] = (base + (w - 3) * 5).clamp(1, 99) as u8;
        }
        Player {
            id: PlayerId(id),
            name: "Test".to_string(),
            birth: TODAY.add_days(-age_years * DAYS_PER_YEAR),
            natural_role: role,
            attributes: Attributes::new(vals),
            character: Character {
                potential: pa,
                determination: 50,
                professionalism: 50,
                consistency: 50,
                injury_proneness: 50,
                leadership: 50,
            },
            development: DevProfile {
                efficiency_milli: 720,
                bloomer_phase_centi: 0,
            },
            contract: (contract_years > 0).then_some(Contract {
                wage: Money(wage),
                expires: TODAY.add_days(contract_years * DAYS_PER_YEAR),
            }),
            retired: false,
        }
    }

    fn mk_club(
        id: u16,
        players: Vec<PlayerId>,
        reputation: u8,
        balance: i64,
        wage_budget: i64,
    ) -> Club {
        Club {
            id: ClubId(id),
            name: format!("Club {id}"),
            players,
            coaching_milli: 1000,
            finances: Finances {
                balance: Money(balance),
                wage_budget: Money(wage_budget),
            },
            reputation,
        }
    }

    fn mk_world(clubs: Vec<Club>, players: Vec<Player>) -> World {
        let club_ids: Vec<ClubId> = clubs.iter().map(|c| c.id).collect();
        let mut cmap = BTreeMap::new();
        for c in clubs {
            cmap.insert(c.id, c);
        }
        let mut pmap = BTreeMap::new();
        for p in players {
            pmap.insert(p.id, p);
        }
        World {
            players: pmap,
            clubs: cmap,
            staff: BTreeMap::new(),
            competition: Competition {
                id: CompetitionId(0),
                name: "Test League".to_string(),
                clubs: club_ids,
            },
        }
    }

    /// Two tiny, otherwise-identical single-player clubs, both missing a
    /// goalkeeper entirely, plus one free-agent goalkeeper both will want —
    /// `high_rep_club` and `low_rep_club` fix which `ClubId` gets which
    /// reputation, so callers can swap the labelling while holding the
    /// underlying scenario fixed.
    fn two_bidder_world(high_rep_club: u16, low_rep_club: u16) -> World {
        let mut players = Vec::new();
        let mut clubs = Vec::new();
        for &(id, rep) in &[(high_rep_club, 90u8), (low_rep_club, 20u8)] {
            let pid = 1000 + id as u32;
            players.push(mk_player(pid, Role::St, 60, 25, 65, 400_000, 3));
            clubs.push(mk_club(
                id,
                vec![PlayerId(pid)],
                rep,
                50_000_000,
                50_000_000,
            ));
        }
        players.push(mk_player(9999, Role::Gk, 75, 24, 85, 0, 0));
        mk_world(clubs, players)
    }

    /// Relaxed squad bounds so a 1-player hand-built club isn't itself
    /// treated as a stabilizer violation in tests that aren't about bounds.
    fn relaxed_utility_knobs() -> UtilityKnobs {
        UtilityKnobs {
            squad_min: 0,
            squad_max: 999,
            ..UtilityKnobs::default()
        }
    }

    #[test]
    fn adversarial_every_club_wants_one_player_terminates_within_max_rounds() {
        let n: u16 = 15; // > MAX_ROUNDS, so a naive one-shot-per-round design would overrun it
        let mut players = Vec::new();
        let mut clubs = Vec::new();
        for i in 0..n {
            let pid = 100 + i as u32;
            players.push(mk_player(pid, Role::St, 60, 25, 65, 500_000, 3));
            clubs.push(mk_club(i, vec![PlayerId(pid)], 50, 50_000_000, 50_000_000));
        }
        // The one player every club is missing entirely and therefore wants
        // most: a free agent, so there is no seller-side gate to thin the
        // field before contention.
        players.push(mk_player(9999, Role::Gk, 80, 24, 90, 0, 0));
        let world = mk_world(clubs, players);

        // Rig consent so nobody ever succeeds: an expectation no reputation
        // can clear. Every one of the n clubs is refused, every round.
        let market_knobs = MarketKnobs {
            reputation_expectation_at_min_ca: 1_000.0,
            reputation_expectation_at_max_ca: 1_000.0,
            reputation_tolerance: 1.0,
            ..MarketKnobs::default()
        };

        let outcome = resolve_window(
            &world,
            TODAY,
            1,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &relaxed_utility_knobs(),
            &market_knobs,
            None,
            &[],
        );

        assert!(
            outcome.rounds_used <= MAX_ROUNDS,
            "must respect the hard cap: {} rounds",
            outcome.rounds_used
        );
        assert!(
            outcome.transfers.is_empty(),
            "an impossible consent bar means nobody ever signs him"
        );
        let refusals: Vec<&RejectedBid> = outcome
            .rejected_bids
            .iter()
            .filter(|r| r.player == PlayerId(9999) && r.reason == RejectReason::PlayerRefused)
            .collect();
        assert_eq!(
            refusals.len(),
            n as usize,
            "every contending club must be recorded as refused exactly once"
        );
        let bidders: BTreeSet<ClubId> = refusals.iter().map(|r| r.bidder).collect();
        assert_eq!(bidders.len(), n as usize, "no club is asked twice");
    }

    #[test]
    fn same_seed_yields_an_identical_transfer_set() {
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(41, &cfg);
        let a = resolve_window(
            &world,
            start,
            41,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &UtilityKnobs::default(),
            &MarketKnobs::default(),
            None,
            &[],
        );
        let b = resolve_window(
            &world,
            start,
            41,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &UtilityKnobs::default(),
            &MarketKnobs::default(),
            None,
            &[],
        );
        assert_eq!(
            a, b,
            "identical inputs must yield an identical WindowOutcome"
        );
    }

    #[test]
    fn no_first_mover_advantage_winner_tracks_content_not_club_id() {
        // The whole point of §5's simultaneous design: swap which numeric
        // `ClubId` the high-reputation club holds and the winner — tracked
        // by content (reputation), not by number — must swap with it.
        let market_knobs = MarketKnobs {
            // Consent guaranteed regardless of reputation, so the ranking
            // (fee tie, then reputation) is the only thing deciding the
            // winner — isolating exactly the property under test.
            reputation_expectation_at_min_ca: 0.0,
            reputation_expectation_at_max_ca: 0.0,
            ..MarketKnobs::default()
        };
        let uk = relaxed_utility_knobs();

        let world_a = two_bidder_world(0, 1); // ClubId(0) = high reputation
        let outcome_a = resolve_window(
            &world_a,
            TODAY,
            7,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &uk,
            &market_knobs,
            None,
            &[],
        );
        let world_b = two_bidder_world(1, 0); // ClubId(1) = high reputation
        let outcome_b = resolve_window(
            &world_b,
            TODAY,
            7,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &uk,
            &market_knobs,
            None,
            &[],
        );

        assert_eq!(outcome_a.transfers.len(), 1);
        assert_eq!(outcome_b.transfers.len(), 1);
        assert_eq!(
            outcome_a.transfers[0].to,
            ClubId(0),
            "the high-reputation club must win regardless of which ClubId it holds"
        );
        assert_eq!(
            outcome_b.transfers[0].to,
            ClubId(1),
            "same content, different numbering — the same (relabelled) winner"
        );
    }

    #[test]
    fn no_player_is_transferred_twice_in_a_window() {
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(9, &cfg);
        let outcome = resolve_window(
            &world,
            start,
            9,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &UtilityKnobs::default(),
            &MarketKnobs::default(),
            None,
            &[],
        );
        let mut seen = BTreeSet::new();
        for t in &outcome.transfers {
            assert!(
                seen.insert(t.player),
                "player {:?} was transferred more than once this window",
                t.player
            );
        }
    }

    #[test]
    fn no_club_ends_outside_its_squad_bounds_or_over_wage_budget() {
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(17, &cfg);
        let knobs = UtilityKnobs::default();
        let outcome = resolve_window(
            &world,
            start,
            17,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &knobs,
            &MarketKnobs::default(),
            None,
            &[],
        );

        let mut post = world.clone();
        for t in &outcome.transfers {
            apply_transfer_completed(&mut post, t.player, t.from, t.to, t.fee, t.contract);
        }
        for club in post.clubs.values() {
            assert!(
                (knobs.squad_min..=knobs.squad_max).contains(&club.players.len()),
                "{} ended with {} players, outside [{}, {}]",
                club.name,
                club.players.len(),
                knobs.squad_min,
                knobs.squad_max
            );
            let wage_bill: i64 = club
                .players
                .iter()
                .filter_map(|pid| post.player(*pid).contract.as_ref())
                .map(|c| c.wage.0)
                .sum();
            assert!(
                wage_bill <= club.finances.wage_budget.0,
                "{} committed wages {} exceed wage budget {}",
                club.name,
                wage_bill,
                club.finances.wage_budget.0
            );
        }
    }

    // --- §10's pre-commitment model ---

    #[test]
    fn human_club_submitting_utility_policys_own_decisions_reproduces_the_same_outcome() {
        // The equivalence property the seam is built on: RecordedPolicy
        // replaying exactly what UtilityPolicy would have decided must not
        // change the clearing loop's outcome at all — the substitution is
        // behaviour-preserving. Guaranteed consent isolates the single
        // GK contest to one round, so a static (never-adapting) replay
        // cannot diverge from the live, freshly-recomputed original.
        let market_knobs = MarketKnobs {
            reputation_expectation_at_min_ca: 0.0,
            reputation_expectation_at_max_ca: 0.0,
            ..MarketKnobs::default()
        };
        let uk = relaxed_utility_knobs();
        let world = two_bidder_world(0, 1);
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();

        let baseline = resolve_window(
            &world,
            TODAY,
            7,
            0,
            &dev,
            &vk,
            &uk,
            &market_knobs,
            None,
            &[],
        );

        // Exactly what UtilityPolicy decides for ClubId(0) against the same
        // round-1 snapshot `resolve_window` itself starts from.
        let ctx = MarketContext::from_world(&world, &vk);
        let valuations = value_all(&world, TODAY, &ctx, &vk, &dev);
        let obs = observe(&world, ClubId(0), TODAY, &valuations, &dev, &uk);
        let ai_decisions = UtilityPolicy::new(uk).transfer_decisions(&obs);

        let substituted = resolve_window(
            &world,
            TODAY,
            7,
            0,
            &dev,
            &vk,
            &uk,
            &market_knobs,
            Some(ClubId(0)),
            &ai_decisions,
        );

        assert_eq!(
            baseline.transfers, substituted.transfers,
            "RecordedPolicy replaying UtilityPolicy's own decisions must reproduce the same transfers"
        );
    }

    #[test]
    fn an_unaffordable_pre_committed_bid_is_dropped_without_panicking() {
        let human = ClubId(0);
        let world = mk_world(
            vec![mk_club(human.0, vec![], 50, 1_000, 1_000)],
            vec![mk_player(9999, Role::Gk, 75, 24, 85, 0, 0)], // free agent
        );
        let decisions = vec![TransferDecision::Bid {
            player: PlayerId(9999),
            from: None,
            role: Role::Gk,
            price: Money(50_000_000), // far beyond the club's cash
        }];

        let outcome = resolve_window(
            &world,
            TODAY,
            3,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &relaxed_utility_knobs(),
            &MarketKnobs::default(),
            Some(human),
            &decisions,
        );

        assert!(
            outcome.transfers.iter().all(|t| t.to != human),
            "an unaffordable pre-committed bid must never complete a transfer: {:?}",
            outcome.transfers
        );
    }

    #[test]
    fn human_club_with_nothing_submitted_completes_no_transfers_of_its_own() {
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(23, &cfg);
        let human = world.competition.clubs[0];
        let outcome = resolve_window(
            &world,
            start,
            23,
            0,
            &DevKnobs::default(),
            &ValueKnobs::default(),
            &UtilityKnobs::default(),
            &MarketKnobs::default(),
            Some(human),
            &[],
        );
        assert!(
            outcome
                .transfers
                .iter()
                .all(|t| t.to != human && t.from != Some(human)),
            "a human club with nothing submitted must complete no transfers of its own: {:?}",
            outcome.transfers
        );
    }
}
