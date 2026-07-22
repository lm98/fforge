//! The Phase-4 Layer-3 club decision AI (`TRANSFER_MODEL.md` §6, §6.1): the
//! `ClubPolicy` trait, shaped in the spirit of the Gym contract — a plain
//! serializable observation in, a constrained enum out, no world internals
//! read, no state mutated — without building the Gym/PettingZoo wrapper
//! itself (`DESIGN.md` §9 places that in Phase 5). `ai_pick_lineup`
//! (`match_engine.rs`) was described in-code as "the Phase-1 stub of the
//! layer-3 club decision AI"; this module is the real thing, for transfer
//! decisions rather than team selection.
//!
//! **Decisions only.** This module produces `TransferDecision`s — proposals
//! — and never resolves them. §5's simultaneous clearing loop (bidding,
//! contention, player consent) and the events that record a completed
//! transfer (`TRANSFER_MODEL.md` §4) are both out of this task's scope: a
//! `Bid` here is exactly the ranked-shortlist-with-a-reservation-price §5
//! step 1 describes a club producing, not a submitted, arbitrated offer.
//!
//! **The trait is the seam, not a crate** (§6.1). A second policy
//! implementation (Phase 5's LLM agent) substitutes at this exact seam ("a
//! config change, not a rewrite"); the crate extraction waits for that
//! second implementation to exist, per `DESIGN.md`'s own "reusability is an
//! extraction, not a prediction."

use crate::development::DevKnobs;
use crate::valuation::project_ca_batch;
use crate::worldgen::SQUAD_TEMPLATE;
use fforge_domain::{best_role, ClubId, GameDate, Money, PlayerId, Role, World, ROLE_WEIGHTS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How far ahead succession risk looks (§6: "the current starter's
/// **projected** CA in 2–3 years").
const SUCCESSION_HORIZON_YEARS: u32 = 3;

/// One squad member's decision-relevant features, pre-resolved so the policy
/// never touches `World` (§6.1). `natural_role` drives `depth_gap` — the
/// doc's own choice, since it is the stable "which slot does he fill"
/// identity `SQUAD_TEMPLATE` is itself keyed on. `role` (his current
/// best-role) plus `current_ca`/`projected_ca` drive `quality_gap` and
/// `succession_risk`, which reason about ability-in-role rather than the
/// player's original training slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SquadMember {
    pub player: PlayerId,
    pub natural_role: Role,
    pub role: Role,
    pub age: i32,
    pub current_ca: u8,
    /// `project_ca` at `SUCCESSION_HORIZON_YEARS` — the third consumer of
    /// §2.3's projection (§6).
    pub projected_ca: u8,
    pub wage: Money,
    pub years_left_on_contract: f64,
}

/// One prospective signing: a player not on this club's books, priced
/// against the frozen §2.7 valuation snapshot. `asking_price` is v1's markup
/// on `value` (`TRANSFER_MODEL.md` §12 item 6, "v1 sets a selling club's ask
/// as a markup on value") — there is no negotiation model yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub player: PlayerId,
    /// `None` = a free agent.
    pub club: Option<ClubId>,
    pub role: Role,
    pub value: Money,
    pub asking_price: Money,
    pub wage: Money,
}

/// The plain, serializable observation a `ClubPolicy` consumes (§6.1):
/// everything `need`/the buy-sell utility needs, already resolved against
/// one frozen window snapshot. No RNG, no clock, no `World` reference — the
/// policy reasons over this alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClubObservation {
    pub club: ClubId,
    pub today: GameDate,
    pub reputation: u8,
    pub balance: Money,
    pub wage_budget: Money,
    pub committed_wages: Money,
    pub squad: Vec<SquadMember>,
    pub candidates: Vec<Candidate>,
}

/// A club's resolved transfer decision (§6.1) — a constrained enum, never a
/// world mutation. Neither variant is itself a transfer: `Bid` is a
/// shortlist entry for §5's (not-yet-built) clearing loop to arbitrate,
/// `List` a listing proposal for that same loop to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDecision {
    Bid {
        player: PlayerId,
        from: Option<ClubId>,
        role: Role,
        price: Money,
    },
    List {
        player: PlayerId,
    },
}

/// `fn transfer_decisions(&self, obs) -> Vec<TransferDecision>` — the
/// Gym-shaped seam (§6.1) `ai_pick_lineup`'s doc comment anticipated.
pub trait ClubPolicy {
    fn transfer_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision>;
}

/// `UtilityPolicy`'s knob table (`TRANSFER_MODEL.md` §6, §9): a
/// plausibility-picked starting point, sibling of `DevKnobs`/`ValueKnobs`/
/// `FinanceKnobs` — the `market` harness (§11) re-fits it against real play.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UtilityKnobs {
    // --- need(club, role) = w_depth·depth_gap + w_quality·quality_gap +
    // w_age·succession_risk (§6) ---
    pub w_depth: f64,
    pub w_quality: f64,
    pub w_age: f64,

    /// Reputation-implied target best-role CA (§6: "against its own
    /// reputation-implied target level, *not* the league mean") — linear
    /// between reputation 1 and 99.
    pub target_ca_at_min_reputation: f64,
    pub target_ca_at_max_reputation: f64,

    // --- stabilizers (§6): hard constraints, not utility terms ---
    pub squad_min: usize,
    pub squad_max: usize,
    pub min_goalkeepers: usize,
    /// Cash kept unspent; a bid is affordable only out of the remainder.
    pub cash_reserve_floor: Money,

    // --- the first two of §6's three selling triggers ---
    pub expiring_within_years: f64,
    /// Worth renewing only if the projection stays within this many CA
    /// points of the club's own target; otherwise an expiring player lists.
    pub renew_worth_margin: f64,

    /// v1's asking-price factor on `value` (§12 item 6, "v1 sets a selling
    /// club's ask as a markup on value"). **Held to `<= 1.0` here, not the
    /// `>1` "markup" the note's plain-English phrasing suggests**: every
    /// club prices off the *same* omniscient `value()` in v1 (§2.6 — private
    /// valuations are explicitly deferred), and §6's utility formula is
    /// `need · (value − asking_price)`. With no private-valuation gap to
    /// exploit, an ask *above* value makes `(value − asking_price)`
    /// negative for every buyer regardless of `need`, so `need`'s multiplier
    /// can never turn a bad deal good — no trade ever clears. A factor
    /// `<= 1.0` (a modest below-value ask) is what keeps §6's formula
    /// capable of producing a signing at all until §12's fog-of-war wrapper
    /// gives sellers a genuinely independent ask.
    pub asking_markup: f64,

    // --- squad-size selling pressure (§9's open residual: squads pinning
    // at `squad_max` because the two triggers above never fire hard enough
    // to trim a squad that's simply *full*, not depth-surplus or
    // contract-expiring anywhere). A *policy* term, not a stabilizer:
    // `squad_min`/`squad_max` above remain the hard bounds this never
    // touches — it only ever widens what the *policy* is willing to list,
    // by a bounded, continuously-growing quota each window, never by
    // dropping a role below its `SQUAD_TEMPLATE` count in one shot. ---
    /// Fraction of `squad_max` below which squad-size pressure is zero —
    /// selling behaves exactly as if this whole mechanism didn't exist.
    pub squad_pressure_start: f64,
    /// Exponent shaping the pressure's ramp over `[squad_pressure_start,
    /// squad_max]`. `>1` back-loads it — willingness to list rises *sharply*
    /// near the cap rather than linearly across the whole range.
    pub squad_pressure_exponent: f64,
    /// The most at-template (`count == SQUAD_TEMPLATE` for that role, i.e.
    /// not already genuinely surplus) players the pressure term may add to
    /// one window's sell list, reached only at `squad_max` itself. Bounds
    /// the mechanism to a steady release valve rather than a one-window
    /// purge: a squad pinned at the cap sheds a few players a window until
    /// it has real headroom again, rather than emptying toward `squad_min`.
    pub squad_pressure_max_listings: usize,
}

impl Default for UtilityKnobs {
    fn default() -> Self {
        UtilityKnobs {
            w_depth: 40.0,
            w_quality: 30.0,
            w_age: 20.0,
            target_ca_at_min_reputation: 45.0,
            target_ca_at_max_reputation: 85.0,
            squad_min: 18,
            squad_max: 30,
            min_goalkeepers: 2,
            cash_reserve_floor: Money(250_000),
            expiring_within_years: 1.0,
            renew_worth_margin: 4.0,
            asking_markup: 0.95,
            squad_pressure_start: 0.75,
            squad_pressure_exponent: 1.5,
            squad_pressure_max_listings: 5,
        }
    }
}

/// The v1 club decision policy (`TRANSFER_MODEL.md` §6): a utility-based
/// buy/sell policy over a `ClubObservation`, with the §6 stabilizers as hard
/// constraints rather than utility terms.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UtilityPolicy {
    pub knobs: UtilityKnobs,
}

impl UtilityPolicy {
    pub fn new(knobs: UtilityKnobs) -> Self {
        UtilityPolicy { knobs }
    }

    /// The reputation-implied target best-role CA (§6) — a club's own bar,
    /// never the league mean.
    pub fn target_ca(&self, reputation: u8) -> f64 {
        let span = (reputation as f64 / 99.0).clamp(0.0, 1.0);
        self.knobs.target_ca_at_min_reputation
            + (self.knobs.target_ca_at_max_reputation - self.knobs.target_ca_at_min_reputation)
                * span
    }

    /// `need(club, role)` (§6): depth vs `SQUAD_TEMPLATE`, quality vs the
    /// club's own reputation-implied target, and succession risk from the
    /// incumbent's `project_ca` projection collapsing below that same
    /// target. A role nobody in the squad plays scores the maximal quality
    /// and succession gap on top of its depth gap.
    pub fn need(&self, obs: &ClubObservation, role: Role) -> f64 {
        let target = self.target_ca(obs.reputation);

        let template = SQUAD_TEMPLATE
            .iter()
            .find(|(r, _)| *r == role)
            .map(|&(_, c)| c as f64)
            .unwrap_or(0.0);
        let current = obs.squad.iter().filter(|m| m.natural_role == role).count() as f64;
        let depth_gap = if template > 0.0 {
            ((template - current) / template).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let starter = obs
            .squad
            .iter()
            .filter(|m| m.role == role)
            .max_by_key(|m| m.current_ca);
        let (quality_gap, succession_risk) = match starter {
            Some(s) => (
                ((target - s.current_ca as f64) / 100.0).clamp(0.0, 1.0),
                ((target - s.projected_ca as f64) / 100.0).clamp(0.0, 1.0),
            ),
            None => (1.0, 1.0),
        };

        self.knobs.w_depth * depth_gap
            + self.knobs.w_quality * quality_gap
            + self.knobs.w_age * succession_risk
    }

    /// `need` for every role — the ranking basis for buy shortlists.
    pub fn needs(&self, obs: &ClubObservation) -> BTreeMap<Role, f64> {
        Role::ALL.iter().map(|&r| (r, self.need(obs, r))).collect()
    }

    /// Buy shortlist (§6): candidates ranked by `need(role) · (value −
    /// asking_price)`, filtered to a positive need, a positive surplus, and
    /// the cash/wage-headroom stabilizers. Squad-ceiling stabilizer: no
    /// buying at/above `squad_max`.
    fn buy_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision> {
        if obs.squad.len() >= self.knobs.squad_max {
            return Vec::new();
        }
        let spendable = obs.balance.0 - self.knobs.cash_reserve_floor.0;
        if spendable <= 0 {
            return Vec::new();
        }
        let wage_room = obs.wage_budget.0 - obs.committed_wages.0;
        if wage_room <= 0 {
            return Vec::new();
        }
        let needs = self.needs(obs);

        let mut scored: Vec<(f64, &Candidate)> = obs
            .candidates
            .iter()
            .filter(|c| c.asking_price.0 <= spendable && c.wage.0 <= wage_room)
            .filter_map(|c| {
                let need = *needs.get(&c.role)?;
                if need <= 0.0 {
                    return None;
                }
                let surplus = (c.value.0 - c.asking_price.0) as f64;
                let utility = need * surplus;
                (utility > 0.0).then_some((utility, c))
            })
            .collect();
        // Deterministic: utility descending, ties broken by PlayerId — never
        // iteration-order accident.
        scored.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| a.1.player.cmp(&b.1.player))
        });

        scored
            .into_iter()
            .map(|(_, c)| TransferDecision::Bid {
                player: c.player,
                from: c.club,
                role: c.role,
                price: c.asking_price,
            })
            .collect()
    }

    /// Squad-size pressure (§9's open residual: squads pinning at
    /// `squad_max` because nothing makes a club *want* to sell when it's
    /// merely full, not depth-surplus or contract-expiring anywhere). `0.0`
    /// below `squad_pressure_start`; ramps to `1.0` at `squad_max`,
    /// back-loaded by `squad_pressure_exponent` so it is negligible in the
    /// middle of the range and rises sharply near the cap.
    fn squad_pressure(&self, squad_len: usize) -> f64 {
        let max = self.knobs.squad_max as f64;
        let start = self.knobs.squad_pressure_start * max;
        let span = (max - start).max(1.0);
        let pressure = ((squad_len as f64 - start) / span).clamp(0.0, 1.0);
        pressure.powf(self.knobs.squad_pressure_exponent)
    }

    /// Sell list: §6's first two triggers (surplus to depth; expiring within
    /// the year and not worth renewing) plus squad-size pressure. The third
    /// §6 trigger — a standing offer above value — needs a live offer, which
    /// does not exist without §5's clearing loop; not modeled here. `≥2 GK`
    /// and `squad_min` are hard stabilizers: listing never proposes dropping
    /// below either.
    ///
    /// **Squad-size pressure** (§9 residual) adds a third, bounded source of
    /// listings: players in a role sitting *exactly* at its `SQUAD_TEMPLATE`
    /// count — not already genuinely surplus — become eligible too, up to a
    /// quota that grows continuously with `squad_pressure` and is capped at
    /// `squad_pressure_max_listings`. It is a quota, not a threshold change,
    /// specifically so a squad pinned at the cap sheds a *few* players a
    /// window rather than every at-template role at once the instant
    /// pressure engages — approached and relieved over successive windows,
    /// not purged in one.
    fn sell_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision> {
        if obs.squad.len() <= self.knobs.squad_min {
            return Vec::new();
        }
        let target = self.target_ca(obs.reputation);
        let pressure = self.squad_pressure(obs.squad.len());
        let pressure_quota =
            (pressure * self.knobs.squad_pressure_max_listings as f64).round() as usize;
        let mut counts: BTreeMap<Role, usize> = BTreeMap::new();
        for m in &obs.squad {
            *counts.entry(m.natural_role).or_default() += 1;
        }
        let mut gk_count = counts.get(&Role::Gk).copied().unwrap_or(0);

        // `bool` tags whether a member is expendable only through the
        // squad-pressure quota (never through a genuine §6 trigger) — the
        // quota below caps only those, never the real triggers.
        let mut expendable: Vec<(&SquadMember, bool)> = obs
            .squad
            .iter()
            .filter_map(|m| {
                let template = SQUAD_TEMPLATE
                    .iter()
                    .find(|(r, _)| *r == m.natural_role)
                    .map(|&(_, c)| c)
                    .unwrap_or(0);
                let count = counts.get(&m.natural_role).copied().unwrap_or(0);
                let surplus_to_depth = count > template;
                let expiring_and_declining = m.years_left_on_contract
                    <= self.knobs.expiring_within_years
                    && (m.projected_ca as f64) < target - self.knobs.renew_worth_margin;
                if surplus_to_depth || expiring_and_declining {
                    return Some((m, false));
                }
                // GK is excluded from pressure-only eligibility: its
                // template (3) sits only one above `min_goalkeepers` (2), so
                // squeezing it for squad-size relief is exactly what
                // produces the GK-coverage violations `TRANSFER_MODEL.md` §9
                // already flags as a *consequence* of pinning, not a
                // separate problem pressure should go looking to cause.
                let at_template = template > 0 && count == template && m.natural_role != Role::Gk;
                at_template.then_some((m, true))
            })
            .collect();
        // Weakest / most expendable first, deterministic tie-break.
        expendable.sort_by(|(a, _), (b, _)| {
            a.current_ca
                .cmp(&b.current_ca)
                .then(a.player.cmp(&b.player))
        });

        let mut decisions = Vec::new();
        let mut remaining = obs.squad.len();
        let mut pressure_used = 0usize;
        for (m, pressure_only) in expendable {
            if remaining <= self.knobs.squad_min {
                break;
            }
            if pressure_only && pressure_used >= pressure_quota {
                continue;
            }
            if m.natural_role == Role::Gk {
                if gk_count <= self.knobs.min_goalkeepers {
                    continue;
                }
                gk_count -= 1;
            }
            decisions.push(TransferDecision::List { player: m.player });
            remaining -= 1;
            if pressure_only {
                pressure_used += 1;
            }
        }
        decisions
    }
}

impl ClubPolicy for UtilityPolicy {
    fn transfer_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision> {
        let mut decisions = self.buy_decisions(obs);
        decisions.extend(self.sell_decisions(obs));
        decisions
    }
}

/// `TRANSFER_MODEL.md` §10's pre-commitment model, promoted from "the seam
/// is left open" to a real second `ClubPolicy`: replays a human's already-
/// validated, already-submitted `TransferDecision`s verbatim, every round,
/// never adapting to how the window unfolds — the client-visible meaning of
/// "pre-commitment." A club with nothing submitted gets an empty list, not a
/// fallback to `UtilityPolicy`: §10 is explicit that a human who ignores the
/// market does nothing in it, exactly as before this seam existed.
/// Affordability, squad bounds, and every other stabilizer are *not* this
/// policy's job — §5's clearing loop applies the same resolve-time filter to
/// every club's decisions regardless of which `ClubPolicy` produced them
/// (`market::filter_affordable`), so a submitted plan that has gone stale or
/// unaffordable by the time its window resolves is dropped there, silently.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecordedPolicy {
    pub decisions: Vec<TransferDecision>,
}

impl RecordedPolicy {
    pub fn new(decisions: Vec<TransferDecision>) -> Self {
        RecordedPolicy { decisions }
    }
}

impl ClubPolicy for RecordedPolicy {
    fn transfer_decisions(&self, _obs: &ClubObservation) -> Vec<TransferDecision> {
        self.decisions.clone()
    }
}

/// Build a `ClubObservation` from the world (§6.1: reading `World` happens
/// here, at the edge — the seam `ClubPolicy` itself never crosses).
/// `valuations` is the §2.7 frozen-snapshot cache (`valuation::value_all`),
/// computed once per window and shared by every club's observation so they
/// all price against the same world.
pub fn observe(
    world: &World,
    club: ClubId,
    today: GameDate,
    valuations: &BTreeMap<PlayerId, Money>,
    dev: &DevKnobs,
    knobs: &UtilityKnobs,
) -> ClubObservation {
    let c = world.club(club);
    // One batched projection for the whole squad (`valuation::project_ca_batch`)
    // rather than one `project_ca` call per member: the knob-derived
    // `DevTables` grid scans are the same regardless of which player is being
    // projected, and `observe` runs once per club per clearing round, so
    // rebuilding them per member would redo the same work dozens of times a
    // window.
    let projections = project_ca_batch(
        world,
        c.players.iter().copied(),
        today,
        SUCCESSION_HORIZON_YEARS,
        dev,
    );
    let squad: Vec<SquadMember> = c
        .players
        .iter()
        .map(|&pid| {
            let p = world.player(pid);
            let (role, current_ca) = best_role(&p.attributes, &ROLE_WEIGHTS);
            let projected_ca = projections[&pid];
            let (wage, years_left_on_contract) = match p.contract {
                Some(contract) => (
                    contract.wage,
                    ((contract.expires.days - today.days) as f64
                        / fforge_domain::date::DAYS_PER_YEAR as f64)
                        .max(0.0),
                ),
                None => (Money(0), 0.0),
            };
            SquadMember {
                player: pid,
                natural_role: p.natural_role,
                role,
                age: p.age(today),
                current_ca,
                projected_ca,
                wage,
                years_left_on_contract,
            }
        })
        .collect();

    let committed_wages: i64 = squad.iter().map(|m| m.wage.0).sum();
    let squad_set: BTreeSet<PlayerId> = c.players.iter().copied().collect();

    let candidates: Vec<Candidate> = world
        .players
        .values()
        .filter(|p| !p.retired && !squad_set.contains(&p.id))
        .map(|p| {
            let (role, _) = best_role(&p.attributes, &ROLE_WEIGHTS);
            let value = valuations.get(&p.id).copied().unwrap_or(Money(0));
            let asking_price = Money((value.0 as f64 * knobs.asking_markup).round() as i64);
            Candidate {
                player: p.id,
                club: world.club_of(p.id),
                role,
                value,
                asking_price,
                wage: p.contract.map(|c| c.wage).unwrap_or(Money(0)),
            }
        })
        .collect();

    ClubObservation {
        club,
        today,
        reputation: c.reputation,
        balance: c.finances.balance,
        wage_budget: c.finances.wage_budget,
        committed_wages: Money(committed_wages),
        squad,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valuation::{value_all, MarketContext, ValueKnobs};
    use crate::worldgen::{generate, WorldGenConfig};

    const TODAY: GameDate = GameDate { days: 2030 * 365 };

    fn member(
        player: u32,
        natural_role: Role,
        role: Role,
        current_ca: u8,
        projected_ca: u8,
    ) -> SquadMember {
        SquadMember {
            player: PlayerId(player),
            natural_role,
            role,
            age: 25,
            current_ca,
            projected_ca,
            wage: Money(500_000),
            years_left_on_contract: 3.0,
        }
    }

    /// A full-depth, roughly on-target squad at mid reputation — the neutral
    /// baseline the individual tests perturb one thing at a time.
    fn baseline_squad() -> Vec<SquadMember> {
        let mut squad = Vec::new();
        let mut next_id = 0u32;
        for &(role, count) in SQUAD_TEMPLATE.iter() {
            for _ in 0..count {
                squad.push(member(next_id, role, role, 65, 65));
                next_id += 1;
            }
        }
        squad
    }

    fn baseline_observation() -> ClubObservation {
        ClubObservation {
            club: ClubId(0),
            today: TODAY,
            reputation: 50,
            balance: Money(10_000_000),
            wage_budget: Money(30_000_000),
            committed_wages: Money(12_000_000),
            squad: baseline_squad(),
            candidates: Vec::new(),
        }
    }

    #[test]
    fn need_ranks_the_short_role_highest() {
        // §6: depth_gap should dominate when a club is missing an entire
        // role — here, no goalkeeper at all.
        let mut obs = baseline_observation();
        obs.squad.retain(|m| m.natural_role != Role::Gk);

        let policy = UtilityPolicy::default();
        let needs = policy.needs(&obs);
        let (top_role, _) = needs
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("at least one role");
        assert_eq!(
            *top_role,
            Role::Gk,
            "the role missing entirely must rank highest, got needs {needs:?}"
        );
    }

    #[test]
    fn squad_at_ceiling_produces_no_buy_decisions() {
        let mut obs = baseline_observation();
        // Pad to the squad_max ceiling with extra strikers.
        let knobs = UtilityKnobs::default();
        while obs.squad.len() < knobs.squad_max {
            let id = obs.squad.len() as u32 + 1000;
            obs.squad.push(member(id, Role::St, Role::St, 65, 65));
        }
        // A juicy, otherwise-fundable candidate — irrelevant once the squad
        // ceiling filter fires first.
        obs.candidates.push(Candidate {
            player: PlayerId(9999),
            club: Some(ClubId(1)),
            role: Role::Gk,
            value: Money(1_000_000),
            asking_price: Money(500_000),
            wage: Money(100_000),
        });

        let policy = UtilityPolicy::new(knobs);
        let decisions = policy.transfer_decisions(&obs);
        assert!(
            !decisions
                .iter()
                .any(|d| matches!(d, TransferDecision::Bid { .. })),
            "a club at its squad ceiling must produce no buy decisions: {decisions:?}"
        );
    }

    #[test]
    fn squad_size_pressure_lists_at_template_players_once_pinned_at_the_ceiling() {
        // §9's open residual: a squad at `squad_max` where only one role
        // (`St`, bumped from its template of 3 up to 9) is genuinely
        // depth-surplus. Every other role sits exactly at its
        // `SQUAD_TEMPLATE` count. Before this fix, only the 9 overflowing
        // strikers would ever be sell-eligible and the other 21 players
        // would never budge — exactly the "pinned at the ceiling" failure
        // mode. The fix's squad-size pressure must additionally surface a
        // bounded number of at-template players once the squad is genuinely
        // full.
        let mut squad = Vec::new();
        let mut next_id = 0u32;
        for &(role, count) in SQUAD_TEMPLATE.iter() {
            let n = if role == Role::St { count + 6 } else { count };
            for _ in 0..n {
                squad.push(member(next_id, role, role, 65, 65));
                next_id += 1;
            }
        }
        let knobs = UtilityKnobs::default();
        assert_eq!(
            squad.len(),
            knobs.squad_max,
            "test setup must land exactly at squad_max"
        );

        let mut obs = baseline_observation();
        obs.squad = squad;

        let policy = UtilityPolicy::new(knobs);
        let decisions = policy.transfer_decisions(&obs);
        let listed: BTreeSet<PlayerId> = decisions
            .iter()
            .filter_map(|d| match d {
                TransferDecision::List { player } => Some(*player),
                _ => None,
            })
            .collect();

        let st_ids: BTreeSet<PlayerId> = obs
            .squad
            .iter()
            .filter(|m| m.natural_role == Role::St)
            .map(|m| m.player)
            .collect();
        let non_st_listed: Vec<PlayerId> = listed
            .iter()
            .filter(|p| !st_ids.contains(p))
            .copied()
            .collect();

        assert!(
            !non_st_listed.is_empty(),
            "squad-size pressure must list at least one at-template player \
             once the squad is pinned at squad_max, got {decisions:?}"
        );
        assert!(
            non_st_listed.len() <= knobs.squad_pressure_max_listings,
            "pressure-driven listings must stay within the quota, got {non_st_listed:?}"
        );
    }

    #[test]
    fn squad_size_pressure_never_lists_an_understaffed_role() {
        // `Cb` is understaffed (1 against a template of 4); `St` is bumped
        // to bring the squad to `squad_max` so pressure is at its sharpest.
        // Squad-size pressure must never pull a player from a role that is
        // already short of its template — that would be actively harmful,
        // not a relief valve.
        let mut squad = Vec::new();
        let mut next_id = 0u32;
        for &(role, count) in SQUAD_TEMPLATE.iter() {
            let n = match role {
                Role::Cb => 1,
                Role::St => count + 9,
                _ => count,
            };
            for _ in 0..n {
                squad.push(member(next_id, role, role, 65, 65));
                next_id += 1;
            }
        }
        let knobs = UtilityKnobs::default();
        assert_eq!(squad.len(), knobs.squad_max);

        let mut obs = baseline_observation();
        obs.squad = squad;

        let policy = UtilityPolicy::new(knobs);
        let decisions = policy.transfer_decisions(&obs);
        let cb_ids: BTreeSet<PlayerId> = obs
            .squad
            .iter()
            .filter(|m| m.natural_role == Role::Cb)
            .map(|m| m.player)
            .collect();

        assert!(
            decisions.iter().all(|d| match d {
                TransferDecision::List { player } => !cb_ids.contains(player),
                _ => true,
            }),
            "an understaffed role must never be listed under squad-size pressure: {decisions:?}"
        );
    }

    #[test]
    fn squad_size_pressure_is_negligible_well_below_the_ceiling() {
        // A full-template squad (24, well under `squad_max = 30`) has no
        // depth surplus, no expiring contracts, and sits below
        // `squad_pressure_start` (0.75 * 30 = 22.5) — squad-size pressure
        // must not manufacture listings out of nothing here.
        let obs = baseline_observation();
        let policy = UtilityPolicy::default();
        let decisions = policy.transfer_decisions(&obs);
        assert!(
            decisions
                .iter()
                .all(|d| !matches!(d, TransferDecision::List { .. })),
            "a comfortably-sized squad must not be pressured into listing: {decisions:?}"
        );
    }

    #[test]
    fn club_with_no_cash_produces_no_buy_decisions() {
        let mut obs = baseline_observation();
        obs.squad.retain(|m| m.natural_role != Role::Gk); // create real need
        obs.balance = Money(0);
        obs.candidates.push(Candidate {
            player: PlayerId(9999),
            club: Some(ClubId(1)),
            role: Role::Gk,
            value: Money(1_000_000),
            asking_price: Money(500_000),
            wage: Money(100_000),
        });

        let policy = UtilityPolicy::default();
        let decisions = policy.transfer_decisions(&obs);
        assert!(
            !decisions
                .iter()
                .any(|d| matches!(d, TransferDecision::Bid { .. })),
            "a club with no cash must produce no buy decisions: {decisions:?}"
        );
    }

    #[test]
    fn collapsing_projection_raises_succession_risk_at_his_role() {
        // §6: an aging starter whose *projected* CA (not his current one)
        // has collapsed must raise need at his role relative to an
        // identically-rated starter with a stable projection.
        let mut stable = baseline_observation();
        let mut declining = baseline_observation();
        for m in stable.squad.iter_mut().filter(|m| m.role == Role::St) {
            m.current_ca = 70;
            m.projected_ca = 70;
        }
        for m in declining.squad.iter_mut().filter(|m| m.role == Role::St) {
            m.current_ca = 70; // same today
            m.projected_ca = 40; // collapsing
        }

        let policy = UtilityPolicy::default();
        let need_stable = policy.need(&stable, Role::St);
        let need_declining = policy.need(&declining, Role::St);
        assert!(
            need_declining > need_stable,
            "a collapsing projection must raise need: stable {need_stable} vs declining {need_declining}"
        );
    }

    #[test]
    fn transfer_decisions_are_deterministic() {
        let mut obs = baseline_observation();
        obs.squad.retain(|m| m.natural_role != Role::Gk);
        obs.candidates.push(Candidate {
            player: PlayerId(9001),
            club: Some(ClubId(1)),
            role: Role::Gk,
            value: Money(2_000_000),
            asking_price: Money(1_000_000),
            wage: Money(200_000),
        });
        obs.candidates.push(Candidate {
            player: PlayerId(9002),
            club: None,
            role: Role::Gk,
            value: Money(2_000_000),
            asking_price: Money(1_000_000),
            wage: Money(200_000),
        });

        let policy = UtilityPolicy::default();
        let a = policy.transfer_decisions(&obs);
        let b = policy.transfer_decisions(&obs);
        assert_eq!(a, b, "the same observation must yield the same decisions");
        // Equal-utility candidates break ties by PlayerId, never by
        // incidental iteration order.
        let bids: Vec<PlayerId> = a
            .iter()
            .filter_map(|d| match d {
                TransferDecision::Bid { player, .. } => Some(*player),
                _ => None,
            })
            .collect();
        assert_eq!(bids, vec![PlayerId(9001), PlayerId(9002)]);
    }

    #[test]
    fn observe_excludes_the_clubs_own_squad_and_uses_the_valuation_cache() {
        let cfg = WorldGenConfig {
            num_clubs: 4,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(11, &cfg);
        let club = world.competition.clubs[0];
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::from_world(&world, &vk);
        let valuations = value_all(&world, start, &ctx, &vk, &dev);
        let knobs = UtilityKnobs::default();

        let obs = observe(&world, club, start, &valuations, &dev, &knobs);

        assert_eq!(obs.squad.len(), world.club(club).players.len());
        let own: BTreeSet<PlayerId> = world.club(club).players.iter().copied().collect();
        assert!(
            obs.candidates.iter().all(|c| !own.contains(&c.player)),
            "candidates must never include the club's own players"
        );
        for c in &obs.candidates {
            let expected =
                Money((valuations[&c.player].0 as f64 * knobs.asking_markup).round() as i64);
            assert_eq!(c.asking_price, expected);
        }

        // Same inputs, same observation — the pipeline is pure.
        let obs2 = observe(&world, club, start, &valuations, &dev, &knobs);
        assert_eq!(obs, obs2);
    }

    #[test]
    fn real_observed_candidates_can_actually_produce_a_bid() {
        // A regression guard for a real bug this integration surfaced:
        // `asking_markup > 1.0` makes `value - asking_price` negative for
        // every real candidate, so `need * surplus` can never be positive
        // and `buy_decisions` silently produces nothing, no matter how
        // desperate the need. Every prior test hand-built candidates with
        // `asking_price < value`, so it went uncaught. This one runs the
        // real `observe()` pipeline end to end and requires at least one
        // real club to find at least one real, affordable, worthwhile Bid.
        let cfg = WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(5, &cfg);
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::from_world(&world, &vk);
        let valuations = value_all(&world, start, &ctx, &vk, &dev);
        let knobs = UtilityKnobs::default();
        let policy = UtilityPolicy::new(knobs);

        let any_bid = world.competition.clubs.iter().any(|&club| {
            let obs = observe(&world, club, start, &valuations, &dev, &knobs);
            policy
                .transfer_decisions(&obs)
                .iter()
                .any(|d| matches!(d, TransferDecision::Bid { .. }))
        });
        assert!(
            any_bid,
            "no club in a real 6-club league produced a single Bid — asking_markup regression"
        );
    }

    #[test]
    fn recorded_policy_replays_exactly_what_was_submitted() {
        let obs = baseline_observation();
        let decisions = vec![
            TransferDecision::List {
                player: obs.squad[0].player,
            },
            TransferDecision::Bid {
                player: PlayerId(9001),
                from: Some(ClubId(1)),
                role: Role::St,
                price: Money(1_000_000),
            },
        ];
        let policy = RecordedPolicy::new(decisions.clone());
        assert_eq!(policy.transfer_decisions(&obs), decisions);
        // A second, differently-shaped observation must not perturb the
        // replay — it is not a function of `obs` at all, by design.
        let mut other = obs.clone();
        other.squad.clear();
        assert_eq!(policy.transfer_decisions(&other), decisions);
    }

    #[test]
    fn recorded_policy_with_nothing_submitted_produces_no_decisions() {
        let policy = RecordedPolicy::default();
        let decisions = policy.transfer_decisions(&baseline_observation());
        assert!(
            decisions.is_empty(),
            "no submission must mean no decisions, never a UtilityPolicy fallback: {decisions:?}"
        );
    }

    #[test]
    fn decisions_are_stable_across_clubs_in_club_id_order() {
        let cfg = WorldGenConfig {
            num_clubs: 4,
            ..Default::default()
        };
        let (world, _schedule, start) = generate(21, &cfg);
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::from_world(&world, &vk);
        let valuations = value_all(&world, start, &ctx, &vk, &dev);
        let knobs = UtilityKnobs::default();
        let policy = UtilityPolicy::default();

        let run = || -> Vec<(ClubId, Vec<TransferDecision>)> {
            world
                .clubs
                .keys()
                .map(|&club| {
                    let obs = observe(&world, club, start, &valuations, &dev, &knobs);
                    (club, policy.transfer_decisions(&obs))
                })
                .collect()
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "the whole per-club pipeline must be deterministic");
        let ids: Vec<ClubId> = a.iter().map(|(c, _)| *c).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "clubs must be processed in ClubId order");
    }
}
