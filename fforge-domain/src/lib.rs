//! fm-domain — layer 1: the domain model.
//!
//! Pure data and pure functions. No I/O, no RNG, no clock, no dependencies on
//! anything above it. The attribute schema here is a transcription of
//! ATTRIBUTE_SCHEMA.md (Phase 0.1); the entity model is the Phase 0.2
//! deliverable.

pub mod ability;
pub mod attributes;
pub mod character;
pub mod date;
pub mod entities;
pub mod formation;
pub mod role;
pub mod substitution;
pub mod tactics;

pub use ability::{best_role, current_ability};
pub use attributes::{Attribute, Attributes, DevCategory, Rating, MAX_RATING, NUM_ATTRIBUTES};
pub use character::Character;
pub use date::GameDate;
pub use entities::{
    Club, ClubId, Competition, CompetitionId, Contract, DevProfile, Finances, Fixture, FixtureId,
    Money, Player, PlayerId, Staff, StaffId, StaffRole, World,
};
pub use formation::{BENCH_SIZE, FORMATIONS, FormationDef, Lineup, MAX_SUBSTITUTIONS, XI};
pub use role::{Role, RoleWeights, NUM_ROLES, ROLE_WEIGHTS};
pub use substitution::{ScoreState, SubAction, SubCondition, SubRule};
pub use tactics::{Mentality, Pressing, Tactics, Tempo, Width};