//! Contract renewal and expiry at each transfer window (`TRANSFER_MODEL.md`
//! §7.1). Pure: the result is a list of events, resolved here so the fold
//! only records them.

use crate::club_ai::{UtilityKnobs, UtilityPolicy, squad_members};
use crate::development::DevKnobs;
use crate::event::Event;
use crate::market::{MarketKnobs, consent_probability};
use crate::rng::derive_stream;
use crate::scouting::{self, ScoutKnobs};
use crate::worldgen::wage_for_quality;
use fforge_domain::date::DAYS_PER_YEAR;
use fforge_domain::{Contract, GameDate, Money, ROLE_WEIGHTS, Role, World, best_role};
use std::collections::BTreeSet;

pub const CONTRACT_STREAM_NS: u64 = 0x434F_4E54_0000_0000; // "CONT"

/// Every renewal and every expiry at the window closing on `close`, in
/// `ClubId` then `PlayerId` order. Renewals come first, so a contract that
/// ends at this window and is renewed in it is not also released.
#[allow(clippy::too_many_arguments)]
pub fn window_contract_events(
    world: &World,
    close: GameDate,
    seed: u64,
    window_index: u64,
    dev: &DevKnobs,
    utility_knobs: &UtilityKnobs,
    market_knobs: &MarketKnobs,
    scout: &ScoutKnobs,
) -> Vec<Event> {
    let policy = UtilityPolicy::new(*utility_knobs);
    let mut rng = derive_stream(seed, CONTRACT_STREAM_NS | window_index);
    let term_days = (market_knobs.new_contract_years * DAYS_PER_YEAR as f64) as i64;
    let mut events = Vec::new();
    let mut renewed = BTreeSet::new();

    for (&club_id, club) in &world.clubs {
        let seen = scouting::seen_squad(world, seed, club_id, club_id, close, scout, dev);
        let mut squad = squad_members(world, &seen, club_id, close, dev);
        squad.sort_by_key(|m| m.player);
        let mut committed: i64 = squad.iter().map(|m| m.wage.0).sum();

        for m in &squad {
            if world.player(m.player).contract.is_none()
                || m.years_left_on_contract > utility_knobs.expiring_within_years
            {
                continue;
            }
            let (_, true_ca) = best_role(&world.player(m.player).attributes, &ROLE_WEIGHTS);
            let market_wage = wage_for_quality(&mut rng, true_ca);
            let roll = rng.f64();

            let wage = Money(m.wage.0.max(market_wage.0));
            let affordable = committed - m.wage.0 + wage.0 <= club.finances.wage_budget.0;
            if !policy.worth_renewing(m, club.reputation) || !affordable {
                continue;
            }
            let consent =
                consent_probability(wage, Some(m.wage), club.reputation, true_ca, market_knobs);
            if roll < consent {
                committed += wage.0 - m.wage.0;
                renewed.insert(m.player);
                events.push(Event::ContractRenewed {
                    date: close,
                    player: m.player,
                    club: club_id,
                    contract: Contract {
                        wage,
                        expires: close.add_days(term_days),
                    },
                });
            }
        }
    }

    // Expiry respects §6's hard stabilizers: a release that would take a
    // squad under `squad_min`, or its goalkeepers under `min_goalkeepers`,
    // becomes a one-year extension at the same wage, as a club option.
    for (&club_id, club) in &world.clubs {
        let mut players = club.players.clone();
        players.sort();
        let mut squad = players.len();
        let mut keepers = players
            .iter()
            .filter(|&&p| world.player(p).natural_role == Role::Gk)
            .count();
        for pid in players {
            let Some(contract) = world.player(pid).contract else {
                continue;
            };
            if contract.expires.days > close.days || renewed.contains(&pid) {
                continue;
            }
            let keeper = world.player(pid).natural_role == Role::Gk;
            let needed = squad <= utility_knobs.squad_min
                || (keeper && keepers <= utility_knobs.min_goalkeepers);
            if needed {
                events.push(Event::ContractRenewed {
                    date: close,
                    player: pid,
                    club: club_id,
                    contract: Contract {
                        wage: contract.wage,
                        expires: close.add_days(DAYS_PER_YEAR),
                    },
                });
                continue;
            }
            squad -= 1;
            if keeper {
                keepers -= 1;
            }
            events.push(Event::PlayerReleased {
                date: close,
                player: pid,
                club: club_id,
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{WorldGenConfig, generate};
    use fforge_domain::{ClubId, PlayerId};

    /// A consent bar every club clears: the player always says yes.
    fn agreeable() -> MarketKnobs {
        MarketKnobs {
            reputation_expectation_at_min_ca: 0.0,
            reputation_expectation_at_max_ca: 0.0,
            ..MarketKnobs::default()
        }
    }

    fn events_for(world: &World, close: GameDate, market: &MarketKnobs) -> Vec<Event> {
        window_contract_events(
            world,
            close,
            7,
            0,
            &DevKnobs::default(),
            &UtilityKnobs::default(),
            market,
            &ScoutKnobs::omniscient(),
        )
    }

    /// The world with every contract running years past `close` except
    /// `player`'s, which ends on `expires`.
    fn one_expiring(close: GameDate, expires: GameDate) -> (World, PlayerId, ClubId) {
        let (mut world, _, _) = generate(3, &WorldGenConfig::default());
        for p in world.players.values_mut() {
            if let Some(c) = p.contract.as_mut() {
                c.expires = close.add_days(5 * DAYS_PER_YEAR);
            }
        }
        let club = ClubId(0);
        let player = world.club(club).players[0];
        world
            .players
            .get_mut(&player)
            .unwrap()
            .contract
            .as_mut()
            .unwrap()
            .expires = expires;
        (world, player, club)
    }

    #[test]
    fn a_player_worth_keeping_is_offered_a_new_term_at_no_less_than_his_wage() {
        let close = GameDate::from_year_day(2030, 230);
        let (mut world, player, club) = one_expiring(close, close.add_days(100));
        world.clubs.get_mut(&club).unwrap().reputation = 1;
        let old_wage = world.player(player).contract.unwrap().wage;

        let events = events_for(&world, close, &agreeable());
        assert_eq!(events.len(), 1, "{events:?}");
        let Event::ContractRenewed { contract, .. } = events[0] else {
            panic!("expected a renewal, got {:?}", events[0]);
        };
        assert!(contract.wage.0 >= old_wage.0);
        assert_eq!(contract.expires, close.add_days(3 * DAYS_PER_YEAR));
    }

    #[test]
    fn an_expired_contract_nobody_renews_is_a_free_agent() {
        let close = GameDate::from_year_day(2030, 230);
        let (mut world, player, club) = one_expiring(close, close);
        world.clubs.get_mut(&club).unwrap().reputation = 99;
        world.players.get_mut(&player).unwrap().attributes =
            fforge_domain::Attributes::new([20; fforge_domain::NUM_ATTRIBUTES]);

        let events = events_for(&world, close, &agreeable());
        assert_eq!(
            events,
            vec![Event::PlayerReleased {
                date: close,
                player,
                club
            }]
        );
    }

    #[test]
    fn a_player_who_refuses_leaves_when_his_contract_ends() {
        let close = GameDate::from_year_day(2030, 230);
        let (mut world, player, club) = one_expiring(close, close);
        world.clubs.get_mut(&club).unwrap().reputation = 1;
        let proud = MarketKnobs {
            reputation_expectation_at_min_ca: 100.0,
            reputation_expectation_at_max_ca: 100.0,
            reputation_tolerance: 1.0,
            ..MarketKnobs::default()
        };
        let events = events_for(&world, close, &proud);
        assert!(matches!(events[..], [Event::PlayerReleased { player: p, .. }] if p == player));
    }

    #[test]
    fn a_club_does_not_renew_beyond_its_wage_budget() {
        let close = GameDate::from_year_day(2030, 230);
        let (mut world, _, club) = one_expiring(close, close.add_days(100));
        let c = world.clubs.get_mut(&club).unwrap();
        c.reputation = 1;
        c.finances.wage_budget = Money(0);
        let events = events_for(&world, close, &agreeable());
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, Event::ContractRenewed { .. })),
            "{events:?}"
        );
    }

    #[test]
    fn expiry_never_takes_a_squad_under_its_minimum_or_its_last_keepers() {
        let close = GameDate::from_year_day(2030, 230);
        let (mut world, _, _) = generate(3, &WorldGenConfig::default());
        for p in world.players.values_mut() {
            if let Some(c) = p.contract.as_mut() {
                c.expires = close;
            }
        }
        for club in world.clubs.values_mut() {
            club.reputation = 99;
        }
        let proud = MarketKnobs {
            reputation_expectation_at_min_ca: 100.0,
            reputation_expectation_at_max_ca: 100.0,
            reputation_tolerance: 1.0,
            ..MarketKnobs::default()
        };
        let knobs = UtilityKnobs::default();
        let events = events_for(&world, close, &proud);
        for (&id, club) in &world.clubs {
            let leaving: BTreeSet<PlayerId> = events
                .iter()
                .filter_map(|e| match e {
                    Event::PlayerReleased { player, club, .. } if *club == id => Some(*player),
                    _ => None,
                })
                .collect();
            let staying: Vec<PlayerId> = club
                .players
                .iter()
                .copied()
                .filter(|p| !leaving.contains(p))
                .collect();
            assert_eq!(staying.len(), knobs.squad_min);
            let keepers = staying
                .iter()
                .filter(|&&p| world.player(p).natural_role == Role::Gk)
                .count();
            assert!(keepers >= knobs.min_goalkeepers);
        }
    }

    #[test]
    fn a_contract_with_years_to_run_is_left_alone() {
        let close = GameDate::from_year_day(2030, 230);
        let (world, _, _) = one_expiring(close, close.add_days(2 * DAYS_PER_YEAR));
        assert!(events_for(&world, close, &agreeable()).is_empty());
    }
}
