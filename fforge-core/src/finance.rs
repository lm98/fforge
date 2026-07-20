//! The Phase-4 finance tick (`TRANSFER_MODEL.md` §4): money's
//! `DevelopmentTick`. Monthly revenue (proportional to `Club.reputation`)
//! minus the monthly share of committed wages, resolved into per-club
//! `Money` deltas that `state::apply` integer-adds to `Club.finances.balance`
//! — no re-derivation, no float in the fold. Fires on the same 30-day period
//! boundary crossing `DevelopmentTick` does (`commands::dev_ticks_between`).
//!
//! Deliberately RNG-free, unlike `tick_changes`: revenue and the wage bill
//! are both already-resolved per-club quantities (`reputation`,
//! `Player.contract`), so this tick is pure arithmetic over the current world
//! snapshot — there is no seeded noise that needs its own stream position.

use crate::development::DEV_TICK_PERIOD_DAYS;
use fforge_domain::{ClubId, Money, World};

const DAYS_PER_YEAR: f64 = fforge_domain::date::DAYS_PER_YEAR as f64;

/// The finance knob table (`TRANSFER_MODEL.md` §9): a plausibility-picked
/// starting point, not a fitted result — the `market` harness (§11) re-fits
/// `revenue_per_reputation` against real wage bills ("sets whether the
/// market clears at all"), exactly as `DevKnobs`'s envelopes were re-fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinanceKnobs {
    /// Annual matchday/broadcast/commercial revenue per reputation point.
    pub revenue_per_reputation: f64,
}

impl Default for FinanceKnobs {
    fn default() -> Self {
        FinanceKnobs {
            // Re-fit against real `worldgen` output (`TRANSFER_MODEL.md` §9):
            // see that section for the harness reading and the reasoning.
            revenue_per_reputation: 500_000.0,
        }
    }
}

/// A tick's fraction of a year (`DEV_TICK_PERIOD_DAYS / 365`), shared with
/// `DevKnobs::dt()`.
#[inline]
fn dt() -> f64 {
    DEV_TICK_PERIOD_DAYS as f64 / DAYS_PER_YEAR
}

/// Resolve one `FinanceTick`'s per-club deltas: this period's revenue minus
/// this period's share of the squad's committed annual wages. `World.clubs`
/// is a `BTreeMap`, so iteration — and therefore the returned order — is
/// deterministic.
pub fn finance_deltas(world: &World, knobs: &FinanceKnobs) -> Vec<(ClubId, Money)> {
    let period = dt();
    world
        .clubs
        .values()
        .map(|club| {
            let revenue = knobs.revenue_per_reputation * club.reputation as f64 * period;
            let wage_bill: f64 = club
                .players
                .iter()
                .filter_map(|pid| world.player(*pid).contract.as_ref())
                .map(|c| c.wage.0 as f64)
                .sum();
            let wages = wage_bill * period;
            (club.id, Money((revenue - wages).round() as i64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{generate, WorldGenConfig};

    #[test]
    fn deltas_cover_every_club_in_id_order() {
        let (world, _schedule, _start) = generate(17, &WorldGenConfig::default());
        let deltas = finance_deltas(&world, &FinanceKnobs::default());
        let ids: Vec<ClubId> = deltas.iter().map(|(c, _)| *c).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "deltas must be in ClubId order");
        assert_eq!(deltas.len(), world.clubs.len());
    }

    #[test]
    fn revenue_scales_with_reputation_at_equal_wage_bill() {
        // Two clubs with the same squad (so identical wage bills) but
        // different reputation must see different deltas, monotone in
        // reputation — revenue is the only reputation-dependent term.
        let (mut world, _schedule, _start) = generate(23, &WorldGenConfig::default());
        let ids: Vec<ClubId> = world.clubs.keys().copied().collect();
        let (lo, hi) = (ids[0], ids[1]);
        let squad = world.club(lo).players.clone();
        world.clubs.get_mut(&hi).unwrap().players = squad;
        world.clubs.get_mut(&lo).unwrap().reputation = 20;
        world.clubs.get_mut(&hi).unwrap().reputation = 80;

        let deltas = finance_deltas(&world, &FinanceKnobs::default());
        let d_lo = deltas.iter().find(|(c, _)| *c == lo).unwrap().1;
        let d_hi = deltas.iter().find(|(c, _)| *c == hi).unwrap().1;
        assert!(
            d_hi.0 > d_lo.0,
            "higher reputation must yield a higher delta at equal wage bill: {} vs {}",
            d_hi.0,
            d_lo.0
        );
    }
}
