//! Formations: an ordered assignment of the eight archetypal roles to the
//! eleven slots. A `Lineup` is the *decision value* a manager (human or AI)
//! submits — the resolved, validated form that gets recorded as an event.

use crate::entities::PlayerId;
use crate::role::Role;
use serde::{Deserialize, Serialize};

pub const XI: usize = 11;

#[derive(Debug, Clone, Copy)]
pub struct FormationDef {
    pub name: &'static str,
    /// Slot roles, GK first, then back-to-front, left-to-right.
    pub slots: [Role; XI],
}

use Role::*;

/// The starting set. Tactics proper (pressing, tempo, width) are Phase 2;
/// here a formation is purely which roles the XI is judged in.
pub const FORMATIONS: [FormationDef; 4] = [
    FormationDef {
        name: "4-4-2",
        slots: [Gk, Fb, Cb, Cb, Fb, W, Cm, Cm, W, St, St],
    },
    FormationDef {
        name: "4-3-3",
        slots: [Gk, Fb, Cb, Cb, Fb, Dm, Cm, Cm, W, St, W],
    },
    FormationDef {
        name: "4-2-3-1",
        slots: [Gk, Fb, Cb, Cb, Fb, Dm, Dm, W, Am, W, St],
    },
    FormationDef {
        name: "3-5-2",
        slots: [Gk, Cb, Cb, Cb, Fb, Dm, Cm, Am, Fb, St, St],
    },
];

/// A submitted team sheet: formation index into `FORMATIONS` + one player
/// per slot, in slot order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineup {
    pub formation: u8,
    pub players: [PlayerId; XI],
}

impl Lineup {
    pub fn formation_def(&self) -> &'static FormationDef {
        &FORMATIONS[self.formation as usize]
    }
}