//! The attribute schema (ATTRIBUTE_SCHEMA.md §1–2, §7, §8).
//!
//! Transcribed from the Phase 0.1 keystone artifact. The doc is the source of
//! truth; this module follows it.

use serde::{Deserialize, Serialize};

/// Fixed 0–100 integer scale for every rated value (§1).
/// Invariant: value ∈ 0..=100. Integers keep serialization/hashing exact —
/// no float edge-cases in the deterministic fold. Display scales (stars,
/// 1–20) are presentation-layer transforms.
pub type Rating = u8;

pub const MAX_RATING: Rating = 100;

/// Selects an attribute's age-curve family (§7).
///
/// Deviation from the §8 sketch, flagged: the sketch had three variants, but
/// §7 defines **four** qualitative curve families (goalkeeping attributes
/// "peak later and age gracefully"). `Goalkeeping` is promoted to its own
/// variant so Phase 3 can parameterize GK curves without a special case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DevCategory {
    Physical,
    Technical,
    Mental,
    Goalkeeping,
}

/// Performance attributes: contribute to CA (§4) and drive match actions (§6).
/// Order is canonical — it is the index into `Attributes` and the row index
/// of the role-weighting table.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Attribute {
    // Technical
    Finishing,
    Passing,
    BallControl,
    Dribbling,
    Tackling,
    Marking,
    Heading,
    Crossing,
    // Mental (performance)
    Vision,
    Decisions,
    DefPositioning,
    OffTheBall,
    Composure,
    Concentration,
    WorkRate,
    Aggression,
    // Physical
    Speed,
    Stamina,
    Strength,
    Agility,
    Jumping,
    // Goalkeeping
    Reflexes,
    Handling,
    CommandOfArea,
    Distribution,
}

pub const NUM_ATTRIBUTES: usize = 25;

impl Attribute {
    pub const ALL: [Attribute; NUM_ATTRIBUTES] = [
        Attribute::Finishing,
        Attribute::Passing,
        Attribute::BallControl,
        Attribute::Dribbling,
        Attribute::Tackling,
        Attribute::Marking,
        Attribute::Heading,
        Attribute::Crossing,
        Attribute::Vision,
        Attribute::Decisions,
        Attribute::DefPositioning,
        Attribute::OffTheBall,
        Attribute::Composure,
        Attribute::Concentration,
        Attribute::WorkRate,
        Attribute::Aggression,
        Attribute::Speed,
        Attribute::Stamina,
        Attribute::Strength,
        Attribute::Agility,
        Attribute::Jumping,
        Attribute::Reflexes,
        Attribute::Handling,
        Attribute::CommandOfArea,
        Attribute::Distribution,
    ];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Age-curve family (§7). Fixed here — a property of the attribute, not
    /// of Phase 3.
    pub fn dev_category(self) -> DevCategory {
        use Attribute::*;
        match self {
            Finishing | Passing | BallControl | Dribbling | Tackling | Marking | Heading
            | Crossing => DevCategory::Technical,
            Vision | Decisions | DefPositioning | OffTheBall | Composure | Concentration
            | WorkRate | Aggression => DevCategory::Mental,
            Speed | Stamina | Strength | Agility | Jumping => DevCategory::Physical,
            Reflexes | Handling | CommandOfArea | Distribution => DevCategory::Goalkeeping,
        }
    }

    pub fn is_goalkeeping(self) -> bool {
        self.dev_category() == DevCategory::Goalkeeping
    }

    pub fn name(self) -> &'static str {
        use Attribute::*;
        match self {
            Finishing => "Finishing",
            Passing => "Passing",
            BallControl => "Ball Control",
            Dribbling => "Dribbling",
            Tackling => "Tackling",
            Marking => "Marking",
            Heading => "Heading",
            Crossing => "Crossing",
            Vision => "Vision",
            Decisions => "Decisions",
            DefPositioning => "Def. Positioning",
            OffTheBall => "Off-the-ball",
            Composure => "Composure",
            Concentration => "Concentration",
            WorkRate => "Work Rate",
            Aggression => "Aggression",
            Speed => "Speed",
            Stamina => "Stamina",
            Strength => "Strength",
            Agility => "Agility",
            Jumping => "Jumping",
            Reflexes => "Reflexes",
            Handling => "Handling",
            CommandOfArea => "Command of Area",
            Distribution => "Distribution",
        }
    }
}

/// Dense array indexed by `Attribute as usize` (§8). Exact serialization;
/// trivial and allocation-free to fold over.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attributes([Rating; NUM_ATTRIBUTES]);

impl Attributes {
    /// Constructs from a raw array, clamping every value to the 0..=100
    /// invariant at the boundary.
    pub fn new(mut values: [Rating; NUM_ATTRIBUTES]) -> Self {
        for v in values.iter_mut() {
            *v = (*v).min(MAX_RATING);
        }
        Attributes(values)
    }

    #[inline]
    pub fn get(&self, a: Attribute) -> Rating {
        self.0[a.index()]
    }

    #[inline]
    pub fn set(&mut self, a: Attribute, v: Rating) {
        self.0[a.index()] = v.min(MAX_RATING);
    }

    pub fn as_array(&self) -> &[Rating; NUM_ATTRIBUTES] {
        &self.0
    }
}