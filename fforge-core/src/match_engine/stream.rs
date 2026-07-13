//! The match event stream (`MATCH_MODEL.md` §9): the action alphabet **is**
//! the stream's event-kind alphabet, designed for narratability (the humble
//! text match view, stats aggregation, the future journalist agent and
//! graphical viewer) — not merely final outcomes.
//!
//! This is a Trace, not a fold input (§7): `play_match` returns it, callers
//! are free to discard it, and nothing here is ever persisted through the
//! event-sourced `GameState` fold.

use super::zone::Zone;

/// Which side an event belongs to. Distinct from `fforge_domain::ClubId` —
/// the stream is home/away-relative; a caller maps it to real clubs via the
/// `Fixture` it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Home,
    Away,
}

/// How the ball arrived at the shot (`MATCH_MODEL.md` §5) — the discriminant
/// that makes headed vs long-range goals countable for the goal-source-mix
/// metric. Through-ball, dribbled, and cutback finishes share `Finish`; only
/// the attacker-attribute selection and chance-quality knob differ between
/// them internally (§9's stream schema pins exactly these three variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShotKind {
    Finish,
    Header,
    LongShot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotOutcome {
    Goal,
    Saved,
    Off,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchEventKind {
    Pass {
        success: bool,
    },
    TakeOn {
        success: bool,
    },
    /// The delivery half of a cross (§5); a successful delivery is followed
    /// by a `Shot { kind: Header, .. }` event for the contested header — the
    /// aerial duel is that shot's defensive side, not a separately resolved
    /// step (§5's own text: "the aerial duel is the defensive half of stage
    /// two rather than a separate resolved step").
    Cross {
        success: bool,
    },
    /// A failed cross delivery, cleared by the defense (§3).
    Clearance,
    /// Possession changed hands (any failure other than a cleared cross).
    Turnover,
    Shot {
        kind: ShotKind,
        outcome: ShotOutcome,
    },
    /// A save that was parried into a scrappy rebound (a `Shot` follow-up
    /// event immediately follows in the stream) vs cleanly collected.
    Save {
        parried: bool,
    },
}

/// One beat in the minute-by-minute stream. `zone` is the zone-entry context
/// (`MATCH_MODEL.md` §9) so a beat can say *where* on the pitch it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchEvent {
    pub minute: u8,
    pub side: Side,
    pub zone: Zone,
    pub kind: MatchEventKind,
}

impl MatchEvent {
    /// A pure, presentation-agnostic rendering — the humble text match view
    /// (`DESIGN.md` §9) is just this, printed in order. No I/O here; callers
    /// (`fforge-game`, the only crate allowed to touch stdout) print it.
    pub fn commentary(&self, side_name: &str) -> String {
        let m = self.minute;
        let z = self.zone.label();
        match self.kind {
            MatchEventKind::Pass { success: true } => format!("{m}' {side_name} pick a pass {z}."),
            MatchEventKind::Pass { success: false } => {
                format!("{m}' {side_name} pass cut out {z}.")
            }
            MatchEventKind::TakeOn { success: true } => {
                format!("{m}' {side_name} beat their marker {z}.")
            }
            MatchEventKind::TakeOn { success: false } => {
                format!("{m}' {side_name} dispossessed {z}.")
            }
            MatchEventKind::Cross { success: true } => format!("{m}' {side_name} whip in a cross."),
            MatchEventKind::Cross { success: false } => {
                format!("{m}' {side_name} cross doesn't find a man.")
            }
            MatchEventKind::Clearance => format!("{m}' Cleared."),
            MatchEventKind::Turnover => format!("{m}' Turned over {z}."),
            MatchEventKind::Shot { kind, outcome } => {
                let k = match kind {
                    ShotKind::Finish => "shot",
                    ShotKind::Header => "header",
                    ShotKind::LongShot => "effort from distance",
                };
                match outcome {
                    ShotOutcome::Goal => format!("{m}' GOAL! {side_name} score with a {k}!"),
                    ShotOutcome::Saved => format!("{m}' {side_name} {k} — saved!"),
                    ShotOutcome::Off => format!("{m}' {side_name} {k} — off target."),
                    ShotOutcome::Blocked => format!("{m}' {side_name} {k} — blocked."),
                }
            }
            MatchEventKind::Save { parried: true } => {
                format!("{m}' Parried! Loose ball in the box.")
            }
            MatchEventKind::Save { parried: false } => format!("{m}' Keeper collects."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commentary_never_panics_across_the_whole_alphabet() {
        let kinds = [
            MatchEventKind::Pass { success: true },
            MatchEventKind::Pass { success: false },
            MatchEventKind::TakeOn { success: true },
            MatchEventKind::TakeOn { success: false },
            MatchEventKind::Cross { success: true },
            MatchEventKind::Cross { success: false },
            MatchEventKind::Clearance,
            MatchEventKind::Turnover,
            MatchEventKind::Shot {
                kind: ShotKind::Finish,
                outcome: ShotOutcome::Goal,
            },
            MatchEventKind::Shot {
                kind: ShotKind::Header,
                outcome: ShotOutcome::Saved,
            },
            MatchEventKind::Shot {
                kind: ShotKind::LongShot,
                outcome: ShotOutcome::Off,
            },
            MatchEventKind::Shot {
                kind: ShotKind::Finish,
                outcome: ShotOutcome::Blocked,
            },
            MatchEventKind::Save { parried: true },
            MatchEventKind::Save { parried: false },
        ];
        for kind in kinds {
            let event = MatchEvent {
                minute: 42,
                side: Side::Home,
                zone: Zone::AttW,
                kind,
            };
            assert!(!event.commentary("Home").is_empty());
        }
    }
}
