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
    /// Squad membership. Finances/budget arrive with Phase 4.
    pub players: Vec<PlayerId>,
    /// Club coaching/academy quality (DEVELOPMENT_MODEL.md §3), a per-club
    /// growth multiplier resolved once at worldgen, in thousandths: `1050` =
    /// 1.05. The "good academy develops players faster" lever.
    pub coaching_milli: u16,
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

    pub fn manager_of(&self, club: ClubId) -> Option<&Staff> {
        self.staff
            .values()
            .find(|s| s.club == Some(club) && s.role == StaffRole::Manager)
    }
}