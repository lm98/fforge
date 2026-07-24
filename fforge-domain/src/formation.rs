//! Formations: an ordered assignment of the eight archetypal roles to the
//! eleven slots. A `Lineup` is the *decision value* a manager (human or AI)
//! submits — the resolved, validated form that gets recorded as an event.

use crate::entities::PlayerId;
use crate::role::Role;
use crate::substitution::SubRule;
use crate::tactics::Tactics;
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
/// per slot, in slot order, plus the tactical instruction set
/// (`TACTICS_MODEL.md` §2, §6) — a team sheet, its shape, and its
/// contingencies are one decision, validated and recorded together.
/// `#[serde(default)]` so old logs deserialize to `Tactics::neutral()`,
/// which the §4 invariant makes bit-identical on replay. `bench`/`sub_plan`
/// (`MATCH_MODEL.md` §16, R7) are the same sanctioned-extension shape:
/// `#[serde(default)]` to empty, which is the substitution identity — no
/// bench, no rules, no mid-match decision point can ever act, so an old log
/// (or any lineup that never sets them) replays bit-identical to the
/// pre-substitution engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineup {
    pub formation: u8,
    pub players: [PlayerId; XI],
    #[serde(default)]
    pub tactics: Tactics,
    /// Up to 7 bench players, validated like starters (in-squad, no
    /// duplicates with `players` or each other).
    #[serde(default)]
    pub bench: Vec<PlayerId>,
    /// The pre-committed substitution/tactic-change plan (§2.7's small
    /// declarative vocabulary), evaluated deterministically at fixed
    /// decision points — never mid-match I/O (`TACTICS_MODEL.md` §7's
    /// pre-commitment pattern applied to the match clock).
    #[serde(default)]
    pub sub_plan: Vec<SubRule>,
}

/// Bench floor (§16): "a bench of 7 (squad floor 18 = XI + 7, so every legal
/// squad can fill it)".
pub const BENCH_SIZE: usize = 7;

/// Substitution cap pinned by the batch-3 T12 task spec (§2.7/T12's own
/// deliverable and test list: "Three substitutions maximum" — the real
/// law's 5, as `MATCH_MODEL.md` §16 first drafted at T2 before this task
/// narrowed it, is *not* what v1 implements; see §16's T12 finding).
pub const MAX_SUBSTITUTIONS: usize = 3;

impl Lineup {
    pub fn formation_def(&self) -> &'static FormationDef {
        &FORMATIONS[self.formation as usize]
    }
}
