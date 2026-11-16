//! Scouting fog-of-war (`AGENT_MODEL.md` §5): what one club can see of a
//! player, as a pure function of the seed and of facts already in the world.
//!
//! Nothing here is stored. A sighting is *derived* each time it is needed —
//! the same rule that makes condition a view over recent appearances — so an
//! estimate can never disagree with the truth it was drawn from, and a replay
//! needs nothing extra.
//!
//! The model is two hidden offsets per `(observer, player, season)`:
//! one shifts every performance attribute together (a scout misjudges a
//! player's *level*, not each attribute independently), the other shifts the
//! estimate of his potential. Perceived ability is then `current_ability` of
//! the attributes seen — one encoding of ability, never a second formula.
//!
//! `ScoutKnobs::omniscient()` is the identity setting: every observer sees
//! the world exactly as it is, and every consumer behaves as it did before
//! this module existed.

use crate::development::DevKnobs;
use crate::rng::{Rng, derive_stream};
use fforge_domain::{
    Attribute, ClubId, DevProfile, GameDate, Player, PlayerId, ROLE_WEIGHTS, Role, World,
    best_role, date::DAYS_PER_YEAR,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Tag namespace for scouting draws (`rng::derive_stream`).
pub const SCOUT_NS: u64 = 0x5343_4F55_0000_0000; // "SCOU"

/// Day of the year a season starts on, matching `worldgen` and `commands`.
const SEASON_START_DOY: i64 = 220;

/// The value every hidden `Character` field takes in a sighting: the
/// midpoint, biasing nothing.
const NEUTRAL: u8 = 50;

/// How wide the fog is, and what narrows it (`AGENT_MODEL.md` §5.2).
/// Plausibility-picked to the owner's reference points — a 17-year-old at
/// another club reads roughly `62–88` for potential, a 27-year-old in one's
/// own squad roughly `74–78` — and then read on the harness like every other
/// table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoutKnobs {
    /// Standard deviation of the potential estimate at `young_age` and below.
    pub sigma_pot_young: f64,
    /// ... and at `mature_age` and above; linear in between.
    pub sigma_pot_mature: f64,
    /// Standard deviation of the level shift on another club's player, in
    /// attribute points, at `young_age` and below.
    pub sigma_now_young: f64,
    /// ... and at `mature_age` and above.
    pub sigma_now_mature: f64,
    pub young_age: f64,
    pub mature_age: f64,
    /// Multiplier on the potential fog for an observer's own players. Their
    /// present attributes are always seen exactly.
    pub own_pot_mult: f64,
    /// Multiplier on both fogs for a player with no club: nobody watches him
    /// week to week.
    pub unattached_mult: f64,
    /// Scouting reach: the multiplier on both fogs for a club of reputation
    /// 100, and of reputation 0. Linear in between. This is the term a future
    /// scouting upgrade lowers.
    pub reach_best: f64,
    pub reach_worst: f64,
    /// Half-width of a displayed range in standard deviations. `1.645` puts
    /// the truth inside the range about nine times in ten — deliberately not
    /// always (§5.2).
    pub range_k: f64,
    /// Whether a sighting replaces the hidden `Character` and `DevProfile`
    /// with neutral values. `false` only at the identity.
    pub hide_character: bool,
}

impl Default for ScoutKnobs {
    fn default() -> Self {
        ScoutKnobs {
            sigma_pot_young: 8.0,
            sigma_pot_mature: 2.0,
            sigma_now_young: 2.5,
            sigma_now_mature: 1.5,
            young_age: 17.0,
            mature_age: 27.0,
            own_pot_mult: 0.6,
            unattached_mult: 1.25,
            reach_best: 0.85,
            reach_worst: 1.15,
            range_k: 1.645,
            hide_character: true,
        }
    }
}

impl ScoutKnobs {
    /// The identity: no fog. Every sighting is the player himself.
    pub fn omniscient() -> Self {
        ScoutKnobs {
            sigma_pot_young: 0.0,
            sigma_pot_mature: 0.0,
            sigma_now_young: 0.0,
            sigma_now_mature: 0.0,
            hide_character: false,
            ..ScoutKnobs::default()
        }
    }

    pub fn is_omniscient(&self) -> bool {
        !self.hide_character
            && self.sigma_pot_young == 0.0
            && self.sigma_pot_mature == 0.0
            && self.sigma_now_young == 0.0
            && self.sigma_now_mature == 0.0
    }

    fn by_age(&self, young: f64, mature: f64, age: f64) -> f64 {
        let t = ((age - self.young_age) / (self.mature_age - self.young_age)).clamp(0.0, 1.0);
        young + (mature - young) * t
    }

    fn reach(&self, reputation: u8) -> f64 {
        let t = reputation as f64 / 100.0;
        self.reach_worst + (self.reach_best - self.reach_worst) * t
    }
}

/// An inclusive band on the 0–100 scale. Its width is the uncertainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub lo: u8,
    pub hi: u8,
}

impl Range {
    pub fn exact(v: u8) -> Self {
        Range { lo: v, hi: v }
    }

    pub fn is_exact(self) -> bool {
        self.lo == self.hi
    }

    pub fn contains(self, v: u8) -> bool {
        (self.lo..=self.hi).contains(&v)
    }

    fn around(centre: f64, half_width: f64, floor: u8) -> Self {
        let lo = (centre - half_width).round().clamp(floor as f64, 100.0) as u8;
        let hi = (centre + half_width).round().clamp(lo as f64, 100.0) as u8;
        Range { lo, hi }
    }
}

/// What `observer` makes of one player this season.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    /// The player as seen: attributes shifted, potential replaced by its
    /// estimate, hidden character neutralised. A whole `Player`, so every
    /// function that reads a player reads a sighting unchanged.
    pub seen: Player,
    pub role: Role,
    /// Best-role ability. Exact for an observer's own player.
    pub ability: Range,
    pub potential: Range,
}

impl Sighting {
    /// The player at the pessimistic (`low`) or optimistic end of both
    /// ranges at once: his seen attributes shifted to the edge of the ability
    /// range, his potential at the edge of the potential range. Valuing the
    /// two corners brackets what he might be worth, since value rises with
    /// both.
    pub fn corner(&self, low: bool) -> Player {
        let point = best_role(&self.seen.attributes, &ROLE_WEIGHTS).1 as i32;
        let (ability, potential) = if low {
            (self.ability.lo, self.potential.lo)
        } else {
            (self.ability.hi, self.potential.hi)
        };
        let shift = ability as i32 - point;
        let mut player = self.seen.clone();
        for attr in Attribute::ALL {
            let v = (self.seen.attributes.get(attr) as i32 + shift).clamp(1, 100);
            player.attributes.set(attr, v as u8);
        }
        player.character.potential = potential;
        player
    }
}

/// The season `today` falls in — the key that keeps a sighting stable for a
/// whole season and re-draws it the next.
pub fn season_of(today: GameDate) -> i64 {
    (today.days - SEASON_START_DOY).div_euclid(DAYS_PER_YEAR)
}

/// The scouting context for one observer in one season: everything about a
/// sighting that does not depend on the player. Build it once, sight many.
pub struct Scout<'a> {
    observer: ClubId,
    stream_seed: u64,
    reach: f64,
    today: GameDate,
    knobs: &'a ScoutKnobs,
    neutral_profile: DevProfile,
}

impl<'a> Scout<'a> {
    pub fn new(
        world: &World,
        seed: u64,
        observer: ClubId,
        today: GameDate,
        knobs: &'a ScoutKnobs,
        dev: &DevKnobs,
    ) -> Self {
        let season = season_of(today) as u64 & 0xFFFF;
        let tag = SCOUT_NS | season << 16 | observer.0 as u64;
        Scout {
            observer,
            stream_seed: derive_stream(seed, tag).next_u64(),
            reach: knobs.reach(world.club(observer).reputation),
            today,
            knobs,
            neutral_profile: DevProfile {
                efficiency_milli: (dev.e_base * 1000.0).round() as u16,
                bloomer_phase_centi: 0,
            },
        }
    }

    fn draws(&self, player: PlayerId) -> (f64, f64) {
        let mut rng: Rng = derive_stream(self.stream_seed, player.0 as u64);
        (rng.normal(0.0, 1.0), rng.normal(0.0, 1.0))
    }

    /// The two standard deviations for `player`: `(now, potential)`.
    fn sigmas(&self, world: &World, player: &Player) -> (f64, f64) {
        let k = self.knobs;
        let age = player.age(self.today) as f64;
        let familiarity = match world.club_of(player.id) {
            Some(club) if club == self.observer => None,
            Some(_) => Some(1.0),
            None => Some(k.unattached_mult),
        };
        let pot = k.by_age(k.sigma_pot_young, k.sigma_pot_mature, age) * self.reach;
        match familiarity {
            None => (0.0, pot * k.own_pot_mult),
            Some(m) => (
                k.by_age(k.sigma_now_young, k.sigma_now_mature, age) * self.reach * m,
                pot * m,
            ),
        }
    }

    pub fn sight(&self, world: &World, player: PlayerId) -> Sighting {
        let truth = world.player(player);
        let (z_now, z_pot) = self.draws(player);
        let (sigma_now, sigma_pot) = self.sigmas(world, truth);

        let mut seen = truth.clone();
        if sigma_now > 0.0 {
            let shift = sigma_now * z_now;
            for attr in Attribute::ALL {
                let v = (truth.attributes.get(attr) as f64 + shift)
                    .round()
                    .clamp(1.0, 100.0);
                seen.attributes.set(attr, v as u8);
            }
        }
        let (role, ability_seen) = best_role(&seen.attributes, &ROLE_WEIGHTS);

        let k = self.knobs;
        // Nobody scouts a ceiling below the present — but with no fog there
        // is nothing to reconcile, and the truth is shown as it is.
        let potential_seen = if sigma_pot > 0.0 {
            (truth.character.potential as f64 + sigma_pot * z_pot)
                .round()
                .clamp(ability_seen as f64, 100.0)
        } else {
            truth.character.potential as f64
        };
        seen.character.potential = potential_seen as u8;
        if k.hide_character {
            seen.character.determination = NEUTRAL;
            seen.character.professionalism = NEUTRAL;
            seen.character.consistency = NEUTRAL;
            seen.character.injury_proneness = NEUTRAL;
            seen.character.natural_fitness = NEUTRAL;
            seen.character.leadership = NEUTRAL;
            seen.development = self.neutral_profile;
        }

        Sighting {
            ability: Range::around(ability_seen as f64, k.range_k * sigma_now, 1),
            potential: if sigma_pot > 0.0 {
                Range::around(potential_seen, k.range_k * sigma_pot, ability_seen)
            } else {
                Range::exact(potential_seen as u8)
            },
            role,
            seen,
        }
    }
}

/// The world as `observer` sees it: every player replaced by his sighting.
/// Squads, contracts and finances are untouched — fog is about players.
/// Borrows `world` unchanged at the identity.
pub fn seen_world<'w>(
    world: &'w World,
    seed: u64,
    observer: ClubId,
    today: GameDate,
    knobs: &ScoutKnobs,
    dev: &DevKnobs,
) -> Cow<'w, World> {
    if knobs.is_omniscient() {
        return Cow::Borrowed(world);
    }
    let scout = Scout::new(world, seed, observer, today, knobs, dev);
    let mut seen = world.clone();
    for (&pid, player) in seen.players.iter_mut() {
        *player = scout.sight(world, pid).seen;
    }
    Cow::Owned(seen)
}

/// `target`'s squad as `observer` sees it — all a team-sheet decision needs
/// to know about an opponent. The returned world holds **only** that club
/// and its players, which keeps a per-fixture view cheap; at the identity it
/// is `world` itself.
pub fn seen_squad<'w>(
    world: &'w World,
    seed: u64,
    observer: ClubId,
    target: ClubId,
    today: GameDate,
    knobs: &ScoutKnobs,
    dev: &DevKnobs,
) -> Cow<'w, World> {
    if knobs.is_omniscient() {
        return Cow::Borrowed(world);
    }
    let scout = Scout::new(world, seed, observer, today, knobs, dev);
    let club = world.club(target);
    Cow::Owned(World {
        players: club
            .players
            .iter()
            .map(|&pid| (pid, scout.sight(world, pid).seen))
            .collect(),
        clubs: BTreeMap::from([(target, club.clone())]),
        staff: BTreeMap::new(),
        competition: world.competition.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{WorldGenConfig, generate};

    fn world() -> (World, GameDate) {
        let (world, _, start) = generate(5, &WorldGenConfig::default());
        (world, start)
    }

    fn clubs(world: &World) -> (ClubId, ClubId) {
        let mut ids = world.clubs.keys().copied();
        (ids.next().unwrap(), ids.next().unwrap())
    }

    #[test]
    fn the_identity_setting_shows_the_world_as_it_is() {
        let (world, today) = world();
        let (a, _) = clubs(&world);
        let k = ScoutKnobs::omniscient();
        let dev = DevKnobs::default();
        assert!(matches!(
            seen_world(&world, 9, a, today, &k, &dev),
            Cow::Borrowed(_)
        ));
        let scout = Scout::new(&world, 9, a, today, &k, &dev);
        for &pid in world.players.keys() {
            let s = scout.sight(&world, pid);
            assert_eq!(&s.seen, world.player(pid));
            assert!(s.ability.is_exact() && s.potential.is_exact());
        }
    }

    #[test]
    fn a_club_sees_its_own_players_as_they_are_but_not_their_ceiling() {
        let (world, today) = world();
        let (a, _) = clubs(&world);
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let scout = Scout::new(&world, 9, a, today, &k, &dev);
        let mut any_uncertain = false;
        for &pid in &world.club(a).players {
            let s = scout.sight(&world, pid);
            assert_eq!(s.seen.attributes, world.player(pid).attributes);
            assert!(s.ability.is_exact());
            any_uncertain |= !s.potential.is_exact();
        }
        assert!(
            any_uncertain,
            "nobody sees a true potential, even their own"
        );
    }

    #[test]
    fn looking_twice_changes_nothing_but_a_new_season_does() {
        let (world, today) = world();
        let (a, b) = clubs(&world);
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let target = world.club(b).players[0];
        let first = Scout::new(&world, 9, a, today, &k, &dev).sight(&world, target);
        let later = Scout::new(&world, 9, a, today.add_days(100), &k, &dev).sight(&world, target);
        assert_eq!(first.seen.attributes, later.seen.attributes);
        assert_eq!(first.potential, later.potential);

        let squad = &world.club(b).players;
        let next_season = today.add_days(DAYS_PER_YEAR);
        let moved = squad.iter().any(|&pid| {
            let now = Scout::new(&world, 9, a, today, &k, &dev).sight(&world, pid);
            let then = Scout::new(&world, 9, a, next_season, &k, &dev).sight(&world, pid);
            now.seen.attributes != then.seen.attributes
        });
        assert!(moved, "a new season is a fresh look");
    }

    #[test]
    fn two_clubs_hold_different_opinions_of_the_same_player() {
        let (world, today) = world();
        let mut ids = world.clubs.keys().copied();
        let (a, b, c) = (
            ids.next().unwrap(),
            ids.next().unwrap(),
            ids.next().unwrap(),
        );
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let from_a = Scout::new(&world, 9, a, today, &k, &dev);
        let from_b = Scout::new(&world, 9, b, today, &k, &dev);
        let differing = world
            .club(c)
            .players
            .iter()
            .filter(|&&pid| {
                from_a.sight(&world, pid).seen.character.potential
                    != from_b.sight(&world, pid).seen.character.potential
            })
            .count();
        assert!(differing * 2 > world.club(c).players.len());
    }

    #[test]
    fn a_shift_moves_the_level_and_keeps_the_shape() {
        let (world, today) = world();
        let (a, b) = clubs(&world);
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let scout = Scout::new(&world, 9, a, today, &k, &dev);
        for &pid in &world.club(b).players {
            let truth = &world.player(pid).attributes;
            let seen = scout.sight(&world, pid).seen.attributes;
            let deltas: Vec<i32> = Attribute::ALL
                .iter()
                .filter(|&&at| (2..=99).contains(&seen.get(at)))
                .map(|&at| seen.get(at) as i32 - truth.get(at) as i32)
                .collect();
            let (lo, hi) = (
                deltas.iter().min().copied().unwrap_or(0),
                deltas.iter().max().copied().unwrap_or(0),
            );
            assert!(hi - lo <= 1, "every unclamped attribute moves together");
        }
    }

    #[test]
    fn ranges_hold_the_truth_most_of_the_time_and_not_always() {
        let (world, today) = world();
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let (mut inside, mut total) = (0u32, 0u32);
        for &observer in world.clubs.keys().take(6) {
            let scout = Scout::new(&world, 9, observer, today, &k, &dev);
            for (&pid, p) in &world.players {
                if world.club_of(pid) == Some(observer) {
                    continue;
                }
                total += 1;
                inside += scout
                    .sight(&world, pid)
                    .potential
                    .contains(p.character.potential) as u32;
            }
        }
        let share = inside as f64 / total as f64;
        assert!(
            (0.80..0.99).contains(&share),
            "potential ranges should hold the truth about nine times in ten, got {share:.3}"
        );
    }

    #[test]
    fn the_young_and_the_unfamiliar_are_seen_least_clearly() {
        let (world, today) = world();
        let (a, _) = clubs(&world);
        let k = ScoutKnobs::default();
        let dev = DevKnobs::default();
        let scout = Scout::new(&world, 9, a, today, &k, &dev);
        let mean_width = |own: bool, young: bool| {
            let widths: Vec<f64> = world
                .players
                .values()
                .filter(|p| (world.club_of(p.id) == Some(a)) == own)
                .filter(|p| (p.age(today) <= 20) == young)
                .map(|p| {
                    let r = scout.sight(&world, p.id).potential;
                    (r.hi - r.lo) as f64
                })
                .collect();
            widths.iter().sum::<f64>() / widths.len().max(1) as f64
        };
        assert!(mean_width(false, true) > mean_width(false, false));
        assert!(mean_width(false, false) > mean_width(true, false));
    }
}
