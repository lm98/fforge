//! Phase 0.2 — the core entity model: players, clubs, staff, competitions.
//!
//! Pure data. Lookups use `BTreeMap`, never `HashMap`: hashmap iteration
//! order is exactly the accidental nondeterminism the architecture bans from
//! anything the deterministic fold can observe.

use crate::attributes::Attributes;
use crate::character::Character;
use crate::date::GameDate;
use crate::role::Role;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

macro_rules! id_newtype {
    ($name:ident, $inner:ty) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub $inner);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_newtype!(PlayerId, u32);
id_newtype!(ClubId, u16);
id_newtype!(StaffId, u32);
id_newtype!(FixtureId, u32);
id_newtype!(CompetitionId, u16);

/// Whole currency units — not cents (`TRANSFER_MODEL.md` §3). Nobody
/// negotiates a fee to the cent, and the extra two digits buy nothing but
/// overflow headroom we do not need. **Signed:** balances genuinely go
/// negative when a club overreaches, and the Phase-4 pathology harness must
/// *see* insolvency rather than have it clamped away. An integer, so the
/// domain stays float-free — `Contract`/`Finances` derive `Eq` and serialize
/// exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Money(pub i64);

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A player's employment terms (`TRANSFER_MODEL.md` §3): an annual `wage` and
/// the date the deal `expires`. A property of the employment, so it lives on
/// the `Player` (`Player.contract`), not in a `World`-level map that would
/// have to be kept in sync with `Club.players` — exactly the store-then-resync
/// bug the CA-is-derived rule (`ATTRIBUTE_SCHEMA.md` §1) exists to make
/// impossible. `None` on a player means a free agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    /// Annual wage.
    pub wage: Money,
    pub expires: GameDate,
}

/// A club's money (`TRANSFER_MODEL.md` §3): one cash `balance` plus one wage
/// commitment ceiling. **`wage_budget` is a constraint, not a second cash
/// pot** — committed annual wages (Σ over the squad's contracts) must stay
/// ≤ `wage_budget`; it is never spent from. Both resolved at worldgen from the
/// club quality anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finances {
    pub balance: Money,
    pub wage_budget: Money,
}

/// Per-player development trajectory parameters (DEVELOPMENT_MODEL.md §2.3),
/// resolved **once** at worldgen from `Character` + seeded noise and carried in
/// the `World` snapshot `GameStarted` records — never re-derived (the monthly
/// tick records resolved deltas, not inputs). Stored as fixed-point integers so
/// the domain stays float-free (exact serialization/hashing, `Eq` derivable),
/// converted to `f64` at use in `fforge-core`'s development engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevProfile {
    /// Growth efficiency E (§2.3) in thousandths: `723` = 0.723. ~200..1900.
    pub efficiency_milli: u16,
    /// Bloomer phase φ in hundredths of a year (§2.3): `183` = +1.83 yr (late
    /// bloomer), `-50` = −0.50 yr (early peaker).
    pub bloomer_phase_centi: i16,
}

impl DevProfile {
    #[inline]
    pub fn efficiency(&self) -> f64 {
        self.efficiency_milli as f64 / 1000.0
    }
    #[inline]
    pub fn bloomer_phase(&self) -> f64 {
        self.bloomer_phase_centi as f64 / 100.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub birth: GameDate,
    /// The role the player is naturally suited to. Headline CA = CA in this
    /// role; playing out of position is allowed and simply rates lower.
    pub natural_role: Role,
    pub attributes: Attributes,
    pub character: Character,
    /// Resolved-once development trajectory (§2.3). See `DevProfile`.
    pub development: DevProfile,
    /// Employment terms (`TRANSFER_MODEL.md` §3): `None` = free agent. The
    /// contract belongs with the employee, so a transfer moves the player
    /// between `Club.players` and there is exactly one place to update.
    pub contract: Option<Contract>,
    /// Set by `Event::PlayerRetired` (`TRANSFER_MODEL.md` §4, §8.2). Retired
    /// players stay in `World.players` (the log references them in historical
    /// `MatchPlayed` XIs) but leave every roster; this is the marker that lets
    /// a future consumer exclude them from development/the market without
    /// confusing them with an ordinary out-of-contract free agent.
    pub retired: bool,
    /// Fit again on this date; `None` (or a past date) = fit now. Set by the
    /// `MatchPlayed` fold arm from a recorded injury's resolved `days_out`
    /// (`MATCH_MODEL.md` §12, §14) — part of the sanctioned Phase-2e domain
    /// extension. `serde(default)` so pre-2e world snapshots load unchanged.
    #[serde(default)]
    pub injured_until: Option<GameDate>,
}

impl Player {
    pub fn age(&self, today: GameDate) -> i32 {
        today.years_since(self.birth)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StaffRole {
    Manager,
    // Scouts, coaches, directors arrive with later phases.
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Staff {
    pub id: StaffId,
    pub name: String,
    pub role: StaffRole,
    pub club: Option<ClubId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Club {
    pub id: ClubId,
    pub name: String,
    /// Squad membership — the **sole** club↔player index. There is no
    /// `Player.club` field (`TRANSFER_MODEL.md` §3): that denormalisation
    /// would give a transfer two places to update and one chance to disagree.
    /// Resolve the reverse direction with `World::club_of`.
    pub players: Vec<PlayerId>,
    /// Club coaching/academy quality (DEVELOPMENT_MODEL.md §3), a per-club
    /// growth multiplier resolved once at worldgen, in thousandths: `1050` =
    /// 1.05. The "good academy develops players faster" lever.
    pub coaching_milli: u16,
    /// Cash balance and wage commitment ceiling (`TRANSFER_MODEL.md` §3),
    /// resolved at worldgen from the quality anchor.
    pub finances: Finances,
    /// Club standing, 0–100 (`TRANSFER_MODEL.md` §3), resolved at worldgen
    /// from the quality anchor. Scales revenue, gates player willingness to
    /// sign, and seeds the Phase-5 board persona.
    pub reputation: u8,
}

impl Club {
    #[inline]
    pub fn coaching(&self) -> f64 {
        self.coaching_milli as f64 / 1000.0
    }
}

/// A single league competition. Cups and multi-league worlds are later
/// content; the shape stays the same.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Competition {
    pub id: CompetitionId,
    pub name: String,
    pub clubs: Vec<ClubId>,
}

/// One scheduled match. Results are events, not fields here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fixture {
    pub id: FixtureId,
    /// 1-based matchday within the season.
    pub matchday: u8,
    pub home: ClubId,
    pub away: ClubId,
}

/// The static world snapshot: who exists. Dynamic state (results, lineups,
/// dates) lives in the event-sourced `GameState` in fm-core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct World {
    pub players: BTreeMap<PlayerId, Player>,
    pub clubs: BTreeMap<ClubId, Club>,
    pub staff: BTreeMap<StaffId, Staff>,
    pub competition: Competition,
}

impl World {
    pub fn player(&self, id: PlayerId) -> &Player {
        &self.players[&id]
    }

    pub fn club(&self, id: ClubId) -> &Club {
        &self.clubs[&id]
    }

    pub fn club_players(&self, id: ClubId) -> impl Iterator<Item = &Player> {
        self.clubs[&id].players.iter().map(move |pid| self.player(*pid))
    }

    /// The club a player belongs to, or `None` if unattached (a free agent).
    /// `Club.players` is the sole index (`TRANSFER_MODEL.md` §3), so this is a
    /// linear scan over clubs — O(clubs), fine for a 20-club league; memoize
    /// outside the domain if it ever matters. Iteration is over the `clubs`
    /// `BTreeMap`, so the answer is deterministic even if two clubs somehow
    /// both listed the player (they must not).
    pub fn club_of(&self, player: PlayerId) -> Option<ClubId> {
        self.clubs
            .values()
            .find(|c| c.players.contains(&player))
            .map(|c| c.id)
    }

    pub fn manager_of(&self, club: ClubId) -> Option<&Staff> {
        self.staff
            .values()
            .find(|s| s.club == Some(club) && s.role == StaffRole::Manager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_and_contract_serialize_exactly() {
        // The float-free invariant (TRANSFER_MODEL.md §3): Money and GameDate
        // are integers, so Contract/Finances derive Eq and survive a JSON
        // round-trip byte-for-byte — no float rounding to lose. A negative
        // balance must round-trip too: insolvency is a real state, not clamped.
        let contract = Contract {
            wage: Money(1_250_000),
            expires: GameDate::from_year_day(2029, 200),
        };
        let back: Contract =
            serde_json::from_str(&serde_json::to_string(&contract).unwrap()).unwrap();
        assert_eq!(back, contract);

        let finances = Finances {
            balance: Money(-4_200_000),
            wage_budget: Money(30_000_000),
        };
        let back: Finances =
            serde_json::from_str(&serde_json::to_string(&finances).unwrap()).unwrap();
        assert_eq!(back, finances);
    }

    #[test]
    fn club_of_agrees_with_the_players_index() {
        // club_of is the reverse of Club.players (TRANSFER_MODEL.md §3): for
        // every listed player it returns that club, and for an unlisted id it
        // returns None. Club.players stays the sole index — no denormalised
        // Player.club to disagree with.
        let mk_club = |id: u16, players: Vec<PlayerId>| Club {
            id: ClubId(id),
            name: format!("Club {id}"),
            players,
            coaching_milli: 1000,
            finances: Finances {
                balance: Money(0),
                wage_budget: Money(0),
            },
            reputation: 50,
        };
        let mut clubs = BTreeMap::new();
        clubs.insert(ClubId(0), mk_club(0, vec![PlayerId(1), PlayerId(2)]));
        clubs.insert(ClubId(1), mk_club(1, vec![PlayerId(3)]));
        let world = World {
            players: BTreeMap::new(),
            clubs,
            staff: BTreeMap::new(),
            competition: Competition {
                id: CompetitionId(0),
                name: "L".to_string(),
                clubs: vec![ClubId(0), ClubId(1)],
            },
        };
        for (&cid, club) in &world.clubs {
            for &pid in &club.players {
                assert_eq!(world.club_of(pid), Some(cid));
            }
        }
        assert_eq!(world.club_of(PlayerId(999)), None);
    }
}