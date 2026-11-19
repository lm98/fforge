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
pub use crate::event::TransferDecision;
use crate::valuation::project_ca_batch;
use crate::worldgen::SQUAD_TEMPLATE;
use fforge_domain::{ClubId, GameDate, Money, PlayerId, ROLE_WEIGHTS, Role, World, best_role};
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
    /// His best role as the observing club sees him, and his ability in it.
    pub role: Role,
    pub ca: u8,
    /// The slot he fills in a squad template (`SquadMember::natural_role`).
    pub natural_role: Role,
    pub age: i32,
    /// What the observing club thinks he is worth — or `Money(0)` if he is
    /// beyond its means and so was never appraised (`observe_as`).
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

impl ClubObservation {
    /// Whether a signing at `ask` and `wage` is within this club's means
    /// right now: cash above the reserve floor, and room under the wage
    /// budget. The one encoding of that test — the buying policy filters on
    /// it, and `observe_as` appraises only the candidates that pass it.
    pub fn within_means(&self, ask: Money, wage: Money, knobs: &UtilityKnobs) -> bool {
        within_means(
            self.balance,
            self.wage_budget,
            self.committed_wages,
            ask,
            wage,
            knobs,
        )
    }
}

fn within_means(
    balance: Money,
    wage_budget: Money,
    committed_wages: Money,
    ask: Money,
    wage: Money,
    knobs: &UtilityKnobs,
) -> bool {
    ask.0 <= balance.0 - knobs.cash_reserve_floor.0 && wage.0 <= wage_budget.0 - committed_wages.0
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

    /// How much better than the incumbent a candidate must look before he
    /// counts as an upgrade or an heir (§6), in ability points.
    pub upgrade_margin: u8,
    /// How many of its highest-paid players a club in financial difficulty
    /// lists each window on top of the ordinary triggers (§6).
    pub distress_listings: usize,
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
            upgrade_margin: 2,
            distress_listings: 2,
        }
    }
}

/// Why a club would sign a candidate (`TRANSFER_MODEL.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Need {
    /// A role under its hard minimum (today, goalkeepers).
    Cover,
    /// A role under its template headcount.
    Depth,
    /// Better than a starter who is under the club's target.
    Upgrade,
    /// Younger than a declining starter and already better than what he
    /// is projected to become.
    Heir,
}

/// Why a club would list one of its own players (`TRANSFER_MODEL.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ListTrigger {
    /// His role is over its template headcount.
    Surplus,
    /// His contract is running out and he is not worth renewing.
    Expiring,
    /// The squad is near its ceiling and his role is exactly at template.
    SquadPressure,
    /// The club is in financial difficulty and he is among its best paid.
    Distress,
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

    fn template_count(role: Role) -> usize {
        SQUAD_TEMPLATE
            .iter()
            .find(|(r, _)| *r == role)
            .map_or(0, |&(_, c)| c)
    }

    fn template_size() -> usize {
        SQUAD_TEMPLATE.iter().map(|&(_, c)| c).sum()
    }

    /// The need signing `c` would fill, if any (§6). `need` ranks the
    /// candidates this admits; on its own it is positive almost everywhere
    /// and admits nearly all of them.
    pub fn need_filled(&self, obs: &ClubObservation, c: &Candidate) -> Option<Need> {
        if self.hard_minimum_violations(obs).contains(&c.role) {
            return Some(Need::Cover);
        }
        let held = obs
            .squad
            .iter()
            .filter(|m| m.natural_role == c.natural_role)
            .count();
        // A full squad does not buy for depth (§6): with a hole in one
        // role it is over-staffed in another, and sells there first.
        let squad_is_short = obs.squad.len() < Self::template_size();
        if squad_is_short && held < Self::template_count(c.natural_role) {
            return Some(Need::Depth);
        }
        let Some(starter) = obs
            .squad
            .iter()
            .filter(|m| m.role == c.role)
            .max_by_key(|m| m.current_ca)
        else {
            return squad_is_short.then_some(Need::Depth);
        };
        let target = self.target_ca(obs.reputation);
        let beats = |incumbent: u8| c.ca >= incumbent.saturating_add(self.knobs.upgrade_margin);
        if (starter.current_ca as f64) < target && beats(starter.current_ca) {
            return Some(Need::Upgrade);
        }
        let heir = starter.projected_ca < starter.current_ca
            && (starter.projected_ca as f64) < target
            && c.age < starter.age
            && beats(starter.projected_ca);
        heir.then_some(Need::Heir)
    }

    pub fn fills_need(&self, obs: &ClubObservation, c: &Candidate) -> bool {
        self.need_filled(obs, c).is_some()
    }

    /// Whether a club would offer `m` a new contract (§7.1): his projected
    /// ability is within `renew_worth_margin` of the club's target. Its
    /// failure is also what lists an expiring player (§6).
    pub fn worth_renewing(&self, m: &SquadMember, reputation: u8) -> bool {
        (m.projected_ca as f64) >= self.target_ca(reputation) - self.knobs.renew_worth_margin
    }

    /// A negative balance, or wages committed beyond the budget (§6).
    pub fn in_financial_difficulty(obs: &ClubObservation) -> bool {
        obs.balance.0 < 0 || obs.committed_wages.0 > obs.wage_budget.0
    }

    /// §6's hard per-role minimum, if `role` has one — today just `Gk` (`≥ 2
    /// GK`, "a club with no keeper is a crash, not a strategy"). Written as a
    /// match rather than a single `Gk`-only check so a future hard minimum on
    /// another role is one arm, not a new mechanism.
    fn hard_minimum(&self, role: Role) -> Option<usize> {
        match role {
            Role::Gk => Some(self.knobs.min_goalkeepers),
            _ => None,
        }
    }

    /// Roles currently below their §6 hard minimum — the role-coverage
    /// override's trigger (`TRANSFER_MODEL.md` §11, "hard stabilizer, target
    /// 0 violations"). Only ever `Gk` today, but resolved generically against
    /// `hard_minimum` rather than hardcoding the one existing case at the call
    /// site.
    fn hard_minimum_violations(&self, obs: &ClubObservation) -> BTreeSet<Role> {
        Role::ALL
            .iter()
            .copied()
            .filter(|&role| {
                self.hard_minimum(role).is_some_and(|min| {
                    let count = obs.squad.iter().filter(|m| m.natural_role == role).count();
                    count < min
                })
            })
            .collect()
    }

    /// Buy shortlist (§6): candidates ranked by `need(role) · (value −
    /// asking_price)`, filtered to a positive need, a positive surplus, and
    /// the cash/wage-headroom stabilizers. Squad-ceiling stabilizer: no
    /// buying at/above `squad_max`.
    ///
    /// **Role-coverage override** (`TRANSFER_MODEL.md` §11's hard
    /// stabilizer): a candidate in a role currently below its §6 hard
    /// minimum (`hard_minimum_violations`) is ranked ahead of every ordinary
    /// candidate, independent of `need · surplus` — an override on the
    /// ranking, not a large weight, so no ordinary opportunity's utility,
    /// however large, can ever outbid it. It is also exempt from the
    /// positive-surplus filter ordinary candidates face: filling the gap
    /// outranks quality gaps, succession risk, *and* surplus value. The hard
    /// cash/wage/squad-ceiling stabilizers above still apply unchanged — the
    /// override re-ranks among what's affordable, it does not waive
    /// affordability; a club with no headroom must sell first (§6) to open
    /// it, which is the "may force a sale to fund it" behaviour in practice.
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
        let violations = self.hard_minimum_violations(obs);

        let mut scored: Vec<(bool, f64, &Candidate)> = obs
            .candidates
            .iter()
            .filter(|c| obs.within_means(c.asking_price, c.wage, &self.knobs))
            .filter_map(|c| {
                let surplus = (c.value.0 - c.asking_price.0) as f64;
                if violations.contains(&c.role) {
                    return Some((true, surplus, c));
                }
                if !self.fills_need(obs, c) {
                    return None;
                }
                let need = *needs.get(&c.role)?;
                if need <= 0.0 {
                    return None;
                }
                let utility = need * surplus;
                (utility > 0.0).then_some((false, utility, c))
            })
            .collect();
        // Deterministic: override candidates first, then utility (or, within
        // the override tier, surplus) descending, ties broken by PlayerId —
        // never iteration-order accident.
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.total_cmp(&a.1))
                .then_with(|| a.2.player.cmp(&b.2.player))
        });

        scored
            .into_iter()
            .map(|(_, _, c)| TransferDecision::Bid {
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
    pub fn listings(&self, obs: &ClubObservation) -> Vec<(PlayerId, ListTrigger)> {
        if obs.squad.len() <= self.knobs.squad_min {
            return Vec::new();
        }
        let pressure = self.squad_pressure(obs.squad.len());
        let pressure_quota =
            (pressure * self.knobs.squad_pressure_max_listings as f64).round() as usize;
        let mut counts: BTreeMap<Role, usize> = BTreeMap::new();
        for m in &obs.squad {
            *counts.entry(m.natural_role).or_default() += 1;
        }
        let mut gk_count = counts.get(&Role::Gk).copied().unwrap_or(0);

        // The squad-pressure quota caps only `SquadPressure` listings,
        // never the two genuine §6 triggers.
        let mut expendable: Vec<(&SquadMember, ListTrigger)> = obs
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
                    && !self.worth_renewing(m, obs.reputation);
                if surplus_to_depth {
                    return Some((m, ListTrigger::Surplus));
                }
                if expiring_and_declining {
                    return Some((m, ListTrigger::Expiring));
                }
                // GK is excluded from pressure-only eligibility: its
                // template (3) sits only one above `min_goalkeepers` (2), so
                // squeezing it for squad-size relief is exactly what
                // produces the GK-coverage violations `TRANSFER_MODEL.md` §9
                // already flags as a *consequence* of pinning, not a
                // separate problem pressure should go looking to cause.
                let at_template = template > 0 && count == template && m.natural_role != Role::Gk;
                at_template.then_some((m, ListTrigger::SquadPressure))
            })
            .collect();
        // The player the club least wants to keep first: the lower of what
        // he is and what he is projected to become (§6), so an heir signed
        // for his future is not the first man out.
        let keep = |m: &SquadMember| m.current_ca.max(m.projected_ca);
        expendable.sort_by(|(a, _), (b, _)| keep(a).cmp(&keep(b)).then(a.player.cmp(&b.player)));

        let mut decisions = Vec::new();
        let mut remaining = obs.squad.len();
        let mut pressure_used = 0usize;
        for (m, trigger) in expendable {
            let pressure_only = trigger == ListTrigger::SquadPressure;
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
            decisions.push((m.player, trigger));
            remaining -= 1;
            if pressure_only {
                pressure_used += 1;
            }
        }

        if Self::in_financial_difficulty(obs) {
            let listed: BTreeSet<PlayerId> = decisions.iter().map(|&(p, _)| p).collect();
            let mut by_wage: Vec<&SquadMember> = obs
                .squad
                .iter()
                .filter(|m| !listed.contains(&m.player))
                .collect();
            by_wage.sort_by(|a, b| b.wage.0.cmp(&a.wage.0).then(a.player.cmp(&b.player)));
            let mut distress_used = 0usize;
            for m in by_wage {
                if remaining <= self.knobs.squad_min
                    || distress_used >= self.knobs.distress_listings
                {
                    break;
                }
                if m.natural_role == Role::Gk {
                    if gk_count <= self.knobs.min_goalkeepers {
                        continue;
                    }
                    gk_count -= 1;
                }
                decisions.push((m.player, ListTrigger::Distress));
                remaining -= 1;
                distress_used += 1;
            }
        }
        decisions
    }

    fn sell_decisions(&self, obs: &ClubObservation) -> Vec<TransferDecision> {
        self.listings(obs)
            .into_iter()
            .map(|(player, _)| TransferDecision::List { player })
            .collect()
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

/// The posted price of a player his owner values at `value`
/// (`AGENT_MODEL.md` §5.3). Under fog this is what makes a trade possible at
/// all: a buyer who rates a player above his seller's ask sees a surplus.
pub fn asking_price(value: Money, knobs: &UtilityKnobs) -> Money {
    Money((value.0 as f64 * knobs.asking_markup).round() as i64)
}

/// What `player` costs in fees: his owner's ask on `value`, or nothing for
/// a free agent (`TRANSFER_MODEL.md` §7.1). The one place a posted price is
/// decided; `value` is consulted only for a player under contract.
pub fn posted_ask(
    world: &World,
    player: PlayerId,
    value: impl FnOnce() -> Option<Money>,
    knobs: &UtilityKnobs,
) -> Money {
    if world.club_of(player).is_none() {
        return Money(0);
    }
    value().map_or(Money(0), |v| asking_price(v, knobs))
}

/// What one club can see of the market's players (`AGENT_MODEL.md` §5): the
/// world as it sees it, its valuation of a player under that view, and the
/// posted price list every club shares.
#[derive(Clone, Copy)]
pub struct MarketSight<'a> {
    pub seen: &'a World,
    pub appraise: &'a dyn Fn(PlayerId) -> Money,
    pub asks: &'a BTreeMap<PlayerId, Money>,
}

/// `observe_as` with no fog: the club sees `world` itself and every ask is
/// the shared valuation times the markup. The identity of §5.
pub fn observe(
    world: &World,
    club: ClubId,
    today: GameDate,
    valuations: &BTreeMap<PlayerId, Money>,
    dev: &DevKnobs,
    knobs: &UtilityKnobs,
) -> ClubObservation {
    let asks: BTreeMap<PlayerId, Money> = valuations
        .iter()
        .map(|(&pid, &value)| (pid, posted_ask(world, pid, || Some(value), knobs)))
        .collect();
    let market = MarketSight {
        seen: world,
        appraise: &|pid| valuations.get(&pid).copied().unwrap_or(Money(0)),
        asks: &asks,
    };
    observe_as(world, club, today, &market, dev, knobs)
}

/// One club's view of its own squad (§6.1): every member's decision-relevant
/// features, read through `seen`, the club's own sighting of its players.
pub(crate) fn squad_members(
    world: &World,
    seen: &World,
    club: ClubId,
    today: GameDate,
    dev: &DevKnobs,
) -> Vec<SquadMember> {
    let c = world.club(club);
    // One batched projection for the whole squad (`valuation::project_ca_batch`)
    // rather than one `project_ca` call per member: the knob-derived
    // `DevTables` grid scans are the same regardless of which player is being
    // projected, and `observe` runs once per club per clearing round, so
    // rebuilding them per member would redo the same work dozens of times a
    // window.
    let projections = project_ca_batch(
        seen,
        c.players.iter().copied(),
        today,
        SUCCESSION_HORIZON_YEARS,
        dev,
    );
    c.players
        .iter()
        .map(|&pid| {
            let p = world.player(pid);
            let (role, current_ca) = best_role(&seen.player(pid).attributes, &ROLE_WEIGHTS);
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
        .collect()
}

/// Build a `ClubObservation` (§6.1: reading `World` happens here, at the
/// edge — the seam `ClubPolicy` itself never crosses). Squads, contracts and
/// money are read from `world`; what a player *is* — attributes, and so role,
/// ability and projection — is read through `market`, the club's own fogged
/// view, frozen for the window (§2.7).
///
/// Only a candidate `within_means` is appraised: valuing a player is the
/// expensive part of an observation, and a club has no use for an opinion on
/// one it cannot pay for. Such a candidate carries `value: Money(0)`.
pub fn observe_as(
    world: &World,
    club: ClubId,
    today: GameDate,
    market: &MarketSight,
    dev: &DevKnobs,
    knobs: &UtilityKnobs,
) -> ClubObservation {
    let MarketSight {
        seen,
        appraise,
        asks,
    } = *market;
    let c = world.club(club);
    let squad = squad_members(world, seen, club, today, dev);

    let committed_wages: i64 = squad.iter().map(|m| m.wage.0).sum();
    let squad_set: BTreeSet<PlayerId> = c.players.iter().copied().collect();

    let candidates: Vec<Candidate> = world
        .players
        .values()
        .filter(|p| !p.retired && !squad_set.contains(&p.id))
        .map(|p| {
            let (role, ca) = best_role(&seen.player(p.id).attributes, &ROLE_WEIGHTS);
            let asking_price = asks.get(&p.id).copied().unwrap_or(Money(0));
            let wage = p.contract.map(|c| c.wage).unwrap_or(Money(0));
            let affordable = within_means(
                c.finances.balance,
                c.finances.wage_budget,
                Money(committed_wages),
                asking_price,
                wage,
                knobs,
            );
            let value = if affordable { appraise(p.id) } else { Money(0) };
            Candidate {
                player: p.id,
                club: world.club_of(p.id),
                role,
                ca,
                natural_role: p.natural_role,
                age: p.age(today),
                value,
                asking_price,
                wage,
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
    use crate::valuation::{MarketContext, ValueKnobs, value_all};
    use crate::worldgen::{WorldGenConfig, generate};

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

    /// A 25-year-old candidate whose natural role is his best one.
    fn candidate(
        player: u32,
        club: Option<ClubId>,
        role: Role,
        ca: u8,
        value: i64,
        asking_price: i64,
        wage: i64,
    ) -> Candidate {
        Candidate {
            player: PlayerId(player),
            club,
            role,
            ca,
            natural_role: role,
            age: 25,
            value: Money(value),
            asking_price: Money(asking_price),
            wage: Money(wage),
        }
    }

    fn bids(decisions: &[TransferDecision]) -> BTreeSet<PlayerId> {
        decisions
            .iter()
            .filter_map(|d| match d {
                TransferDecision::Bid { player, .. } => Some(*player),
                _ => None,
            })
            .collect()
    }

    fn listings(decisions: &[TransferDecision]) -> Vec<PlayerId> {
        decisions
            .iter()
            .filter_map(|d| match d {
                TransferDecision::List { player } => Some(*player),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bargain_that_fills_no_need_is_not_bought() {
        let mut obs = baseline_observation();
        let target = UtilityPolicy::default().target_ca(obs.reputation) as u8;
        for m in &mut obs.squad {
            m.current_ca = target - 1;
            m.projected_ca = target - 1;
        }
        // Under the club's target everywhere, so `need` is positive for
        // every role; priced at a fifth of his value; no better than the
        // man he would sit behind.
        obs.candidates.push(candidate(
            9001,
            None,
            Role::St,
            target - 1,
            5_000_000,
            1_000_000,
            100_000,
        ));
        let policy = UtilityPolicy::default();
        assert!(policy.need(&obs, Role::St) > 0.0);
        assert!(bids(&policy.transfer_decisions(&obs)).is_empty());
    }

    #[test]
    fn a_hole_in_the_squad_an_upgrade_and_an_heir_are_each_a_need() {
        let policy = UtilityPolicy::default();
        let margin = policy.knobs.upgrade_margin;
        let base = baseline_observation();
        let target = policy.target_ca(base.reputation) as u8;
        let striker = |ca| candidate(9001, None, Role::St, ca, 5_000_000, 1_000_000, 100_000);

        let mut hole = base.clone();
        let gone = hole
            .squad
            .iter()
            .position(|m| m.natural_role == Role::St)
            .unwrap();
        hole.squad.remove(gone);
        assert!(
            policy.fills_need(&hole, &striker(1)),
            "depth asks nothing of quality"
        );

        let mut short = base.clone();
        for m in &mut short.squad {
            m.current_ca = target - 5;
            m.projected_ca = target - 5;
        }
        assert!(policy.fills_need(&short, &striker(target - 5 + margin)));
        assert!(!policy.fills_need(&short, &striker(target - 5 + margin - 1)));

        let mut on_target = base.clone();
        for m in &mut on_target.squad {
            m.current_ca = target + 3;
            m.projected_ca = target + 3;
        }
        assert!(
            !policy.fills_need(&on_target, &striker(99)),
            "a starter at the club's level is not a gap, whoever is on offer"
        );

        let mut ageing = on_target.clone();
        for m in &mut ageing.squad {
            m.age = 32;
            m.projected_ca = target - 10;
        }
        assert!(policy.fills_need(&ageing, &striker(target - 10 + margin)));
        let mut older = striker(target - 10 + margin);
        older.age = 33;
        assert!(!policy.fills_need(&ageing, &older), "an heir is younger");
    }

    #[test]
    fn a_full_squad_does_not_buy_for_depth() {
        let policy = UtilityPolicy::default();
        let striker = candidate(9001, None, Role::St, 1, 5_000_000, 1_000_000, 100_000);

        // One striker short, and one centre-back over: a full squad with a
        // hole in it.
        let mut lopsided = baseline_observation();
        let gone = lopsided
            .squad
            .iter()
            .position(|m| m.natural_role == Role::St)
            .unwrap();
        lopsided.squad.remove(gone);
        lopsided
            .squad
            .push(member(9500, Role::Cb, Role::Cb, 65, 65));
        assert_eq!(policy.need_filled(&lopsided, &striker), None);

        // The same hole once the spare centre-back has gone.
        lopsided.squad.pop();
        assert_eq!(policy.need_filled(&lopsided, &striker), Some(Need::Depth));
    }

    #[test]
    fn a_surplus_role_sheds_the_player_with_the_least_now_and_later() {
        let mut obs = baseline_observation();
        // A fourth striker takes the role over its template. The heir is the
        // weakest today and the best in three years; the veteran is the
        // reverse; the journeyman is neither.
        let (heir, veteran, journeyman) = (PlayerId(9001), PlayerId(9002), PlayerId(9003));
        obs.squad.retain(|m| m.natural_role != Role::St);
        obs.squad.push(member(heir.0, Role::St, Role::St, 55, 75));
        obs.squad
            .push(member(veteran.0, Role::St, Role::St, 75, 55));
        obs.squad
            .push(member(journeyman.0, Role::St, Role::St, 60, 60));
        obs.squad.push(member(9004, Role::St, Role::St, 70, 70));
        let listed = listings(&UtilityPolicy::default().transfer_decisions(&obs));
        assert_eq!(listed.first(), Some(&journeyman));
    }

    #[test]
    fn a_club_in_financial_difficulty_lists_its_best_paid_and_buys_nobody() {
        let policy = UtilityPolicy::default();
        let mut obs = baseline_observation();
        assert!(!UtilityPolicy::in_financial_difficulty(&obs));
        assert!(listings(&policy.transfer_decisions(&obs)).is_empty());

        obs.squad[5].wage = Money(900_000);
        obs.squad[9].wage = Money(800_000);
        obs.balance = Money(-1);
        obs.candidates
            .push(candidate(9001, None, Role::St, 99, 5_000_000, 1, 1));
        assert!(UtilityPolicy::in_financial_difficulty(&obs));
        let decisions = policy.transfer_decisions(&obs);
        assert_eq!(
            listings(&decisions),
            vec![obs.squad[5].player, obs.squad[9].player]
        );
        assert!(bids(&decisions).is_empty());
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
    fn role_coverage_override_ranks_a_gk_bid_first_regardless_of_other_needs() {
        // TRANSFER_MODEL.md §11's hard stabilizer: a club below a role's hard
        // minimum (here, one keeper against `min_goalkeepers = 2`) must rank
        // filling it above every ordinary need — even an outlandishly
        // attractive opportunity elsewhere must not outbid it. Reduced to one
        // keeper, kept otherwise full-depth so nothing else is a genuine
        // depth gap; only the GK hard minimum is violated.
        let mut obs = baseline_observation();
        let mut gks_seen = 0;
        obs.squad.retain(|m| {
            if m.natural_role != Role::Gk {
                return true;
            }
            gks_seen += 1;
            gks_seen <= 1
        });

        // A merely-adequate goalkeeper candidate...
        obs.candidates.push(candidate(
            9001,
            Some(ClubId(1)),
            Role::Gk,
            65,
            1_000_000,
            900_000,
            100_000,
        ));
        // ...against a strike candidate whose enormous surplus would win
        // under plain `need · surplus` ranking even though the club's
        // strikers are already at full template depth (need for `St` is low,
        // but `need · surplus` is unbounded in surplus).
        obs.candidates.push(candidate(
            9002,
            Some(ClubId(2)),
            Role::St,
            65,
            50_000_000,
            1_000_000,
            100_000,
        ));

        let policy = UtilityPolicy::default();
        let decisions = policy.transfer_decisions(&obs);
        let top_bid = decisions
            .iter()
            .find(|d| matches!(d, TransferDecision::Bid { .. }))
            .expect("at least one bid");
        assert_eq!(
            *top_bid,
            TransferDecision::Bid {
                player: PlayerId(9001),
                from: Some(ClubId(1)),
                role: Role::Gk,
                price: Money(900_000),
            },
            "a club below its GK hard minimum must bid on a keeper first, \
             regardless of what else its squad needs: {decisions:?}"
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
        obs.candidates.push(candidate(
            9999,
            Some(ClubId(1)),
            Role::Gk,
            65,
            1_000_000,
            500_000,
            100_000,
        ));

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
        obs.candidates.push(candidate(
            9999,
            Some(ClubId(1)),
            Role::Gk,
            65,
            1_000_000,
            500_000,
            100_000,
        ));

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
        obs.candidates.push(candidate(
            9001,
            Some(ClubId(1)),
            Role::Gk,
            65,
            2_000_000,
            1_000_000,
            200_000,
        ));
        obs.candidates.push(candidate(
            9002,
            None,
            Role::Gk,
            65,
            2_000_000,
            1_000_000,
            200_000,
        ));

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
        let ctx = MarketContext::from_world(&world, &vk, &BTreeMap::new());
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
        let ctx = MarketContext::from_world(&world, &vk, &BTreeMap::new());
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
        let ctx = MarketContext::from_world(&world, &vk, &BTreeMap::new());
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
