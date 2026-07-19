//! The match event stream (`MATCH_MODEL.md` §9): the action alphabet **is**
//! the stream's event-kind alphabet, designed for narratability (the humble
//! text match view, stats aggregation, the future journalist agent and
//! graphical viewer) — not merely final outcomes.
//!
//! This is a Trace, not a fold input (§7): `play_match` returns it, callers
//! are free to discard it, and nothing here is ever persisted through the
//! event-sourced `GameState` fold.

use super::zone::Zone;
use fforge_domain::PlayerId;

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

/// How the possession *reached* the shot (`MATCH_MODEL.md` §5's arrival
/// table) — finer-grained than `ShotKind`, which collapses through-ball,
/// dribble, and cutback finishes into `Finish`. This is what makes the
/// wide-origin-goal-share calibration target (cross + cutback,
/// `MATCH_MODEL.md` §8) actually computable from the stream, not just
/// headed-goal share. A rebound follow-up shot keeps the source of the shot
/// that created it — the rebound is a continuation of the same attack, not
/// a new arrival route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShotSource {
    Through,
    Dribble,
    Cutback,
    Cross,
    Long,
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
        source: ShotSource,
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
///
/// `actor` names the on-ball player the resolver already sampled from the
/// `side`-relative fielding XI's zone presence table (`MATCH_MODEL.md` §6) —
/// it is always a member of `side`'s eleven. `opponent` names the single
/// contesting player for the beats that resolve a two-player contest (a
/// take-on/tackle, the aerial duel inside a headed shot, a keeper's save);
/// it is `None` where there is genuinely no single opponent (a pass into
/// space, a cross that finds no one, a cleared ball, a shot with no
/// individual duel). This is `MATCH_MODEL.md` §9's identity enrichment
/// (`TRANSFER_MODEL.md` §12 item 1, P4.0): the stream was designed for
/// narratability but carried no player identity, blocking the journalist
/// agent that cannot write "*Rossi scored at 73'*" from an anonymous stream.
/// No new sampling — these are the actor/defender the resolver drew anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchEvent {
    pub minute: u8,
    pub side: Side,
    pub zone: Zone,
    pub kind: MatchEventKind,
    /// The on-ball player, always in `side`'s fielding XI.
    pub actor: PlayerId,
    /// The single contesting player, when the beat resolves one (take-on,
    /// tackle, aerial duel, save); `None` otherwise.
    pub opponent: Option<PlayerId>,
}

impl MatchEvent {
    /// A pure, presentation-agnostic rendering — the humble text match view
    /// (`DESIGN.md` §9) is just this, printed in order. No I/O here; callers
    /// (`fforge-game`, the only crate allowed to touch stdout) resolve the
    /// `side`/`actor` display names from the `World` and pass them in — this
    /// crate never looks a `PlayerId` up (it holds no `World`) and never
    /// touches stdout. `actor` is the resolved name of `self.actor`.
    pub fn commentary(&self, side_name: &str, actor: &str) -> String {
        let m = self.minute;
        let z = self.zone.label();
        match self.kind {
            MatchEventKind::Pass { success: true } => {
                format!("{m}' {actor} ({side_name}) picks a pass {z}.")
            }
            MatchEventKind::Pass { success: false } => {
                format!("{m}' {actor} ({side_name}) pass cut out {z}.")
            }
            MatchEventKind::TakeOn { success: true } => {
                format!("{m}' {actor} ({side_name}) beats their marker {z}.")
            }
            MatchEventKind::TakeOn { success: false } => {
                format!("{m}' {actor} ({side_name}) dispossessed {z}.")
            }
            MatchEventKind::Cross { success: true } => {
                format!("{m}' {actor} ({side_name}) whips in a cross.")
            }
            MatchEventKind::Cross { success: false } => {
                format!("{m}' {actor} ({side_name}) cross doesn't find a man.")
            }
            MatchEventKind::Clearance => format!("{m}' Cleared."),
            MatchEventKind::Turnover => format!("{m}' Turned over {z}."),
            MatchEventKind::Shot { kind, outcome, .. } => {
                let k = match kind {
                    ShotKind::Finish => "shot",
                    ShotKind::Header => "header",
                    ShotKind::LongShot => "effort from distance",
                };
                match outcome {
                    ShotOutcome::Goal => {
                        format!("{m}' GOAL! {actor} ({side_name}) scores with a {k}!")
                    }
                    ShotOutcome::Saved => format!("{m}' {actor} ({side_name}) {k} — saved!"),
                    ShotOutcome::Off => format!("{m}' {actor} ({side_name}) {k} — off target."),
                    ShotOutcome::Blocked => format!("{m}' {actor} ({side_name}) {k} — blocked."),
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
                source: ShotSource::Through,
                outcome: ShotOutcome::Goal,
            },
            MatchEventKind::Shot {
                kind: ShotKind::Header,
                source: ShotSource::Cross,
                outcome: ShotOutcome::Saved,
            },
            MatchEventKind::Shot {
                kind: ShotKind::LongShot,
                source: ShotSource::Long,
                outcome: ShotOutcome::Off,
            },
            MatchEventKind::Shot {
                kind: ShotKind::Finish,
                source: ShotSource::Cutback,
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
                actor: PlayerId(7),
                opponent: Some(PlayerId(3)),
            };
            assert!(!event.commentary("Home", "Rossi").is_empty());
        }
    }
}
