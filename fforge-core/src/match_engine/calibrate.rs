//! Calibration telemetry (`MATCH_MODEL.md` §7-8): a passive accumulator over
//! `MatchOutcome` traces, deliberately **not** wired into the fold — the
//! event-sourced `GameState` only ever folds the score
//! (`Event::MatchPlayed`), and `SeasonTelemetry` (`observer.rs`) is the
//! consumer that sees that folded stream. Shots, on-target rate, conversion,
//! and goal-source mix live only in the discarded `MatchOutcome.stream`, so
//! the calibration harness must consume it directly — exactly what §7
//! sanctions ("calibration re-runs the engine freely").
//!
//! This module is exploratory-harness plumbing, not simulation logic: it
//! never feeds back into `Knobs` or the presence tables by itself.

use super::MatchOutcome;
use super::stream::{MatchEventKind, ShotKind, ShotOutcome, ShotSource, Side};
use std::collections::BTreeMap;

/// Per-formation usage seen by `StreamTelemetry::record` — one increment per
/// side per match (a formation used by both home and away in the same match
/// counts twice), keyed by `Lineup::formation` in the caller.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FormationStats {
    /// Number of side-uses of this formation (not matches).
    pub uses: u32,
    pub goals: u32,
    pub shots: u32,
}

impl FormationStats {
    pub fn goals_per_match(&self) -> f64 {
        if self.uses == 0 {
            return 0.0;
        }
        self.goals as f64 / self.uses as f64
    }

    pub fn shots_per_match(&self) -> f64 {
        if self.uses == 0 {
            return 0.0;
        }
        self.shots as f64 / self.uses as f64
    }
}

/// Accumulates match aggregates directly from `MatchOutcome` traces —
/// everything `SeasonTelemetry` can't see because the fold discards the
/// stream. Mirrors the notebook's `report()` fields.
#[derive(Debug, Default, Clone)]
pub struct StreamTelemetry {
    pub matches: u32,
    pub home_wins: u32,
    pub draws: u32,
    pub away_wins: u32,
    pub goals: u32,
    pub shots: u32,
    pub shots_on_target: u32,
    pub goals_by_kind: BTreeMap<ShotKind, u32>,
    /// Goals keyed by arrival route (`MATCH_MODEL.md` §5) — what makes the
    /// wide-origin-goal-share target (cross + cutback, §8) computable,
    /// distinct from `goals_by_kind`'s coarser Finish/Header/LongShot split.
    pub goals_by_source: BTreeMap<ShotSource, u32>,
    /// A crude possession proxy: total stream events attributed to each
    /// side (more events ⇒ more time on the ball / advancing it).
    pub home_events: u32,
    pub away_events: u32,
    /// Keyed by `Lineup::formation` index into `FORMATIONS`.
    pub by_formation: BTreeMap<u8, FormationStats>,
}

impl StreamTelemetry {
    /// Fold one match's trace in. `home_formation`/`away_formation` are the
    /// `Lineup::formation` index each side fielded, for the per-formation
    /// breakdown (`MATCH_MODEL.md` §10 item 1's diagnostic).
    pub fn record(&mut self, outcome: &MatchOutcome, home_formation: u8, away_formation: u8) {
        self.matches += 1;
        self.goals += outcome.home_goals as u32 + outcome.away_goals as u32;
        match outcome.home_goals.cmp(&outcome.away_goals) {
            std::cmp::Ordering::Greater => self.home_wins += 1,
            std::cmp::Ordering::Equal => self.draws += 1,
            std::cmp::Ordering::Less => self.away_wins += 1,
        }

        let mut home_shots = 0u32;
        let mut away_shots = 0u32;
        let mut home_goals = 0u32;
        let mut away_goals = 0u32;

        for event in &outcome.stream {
            match event.side {
                Side::Home => self.home_events += 1,
                Side::Away => self.away_events += 1,
            }
            if let MatchEventKind::Shot {
                kind,
                source,
                outcome: shot_outcome,
            } = event.kind
            {
                self.shots += 1;
                match event.side {
                    Side::Home => home_shots += 1,
                    Side::Away => away_shots += 1,
                }
                if matches!(shot_outcome, ShotOutcome::Goal | ShotOutcome::Saved) {
                    self.shots_on_target += 1;
                }
                if shot_outcome == ShotOutcome::Goal {
                    *self.goals_by_kind.entry(kind).or_default() += 1;
                    *self.goals_by_source.entry(source).or_default() += 1;
                    match event.side {
                        Side::Home => home_goals += 1,
                        Side::Away => away_goals += 1,
                    }
                }
            }
        }

        let home_stats = self.by_formation.entry(home_formation).or_default();
        home_stats.uses += 1;
        home_stats.goals += home_goals;
        home_stats.shots += home_shots;

        let away_stats = self.by_formation.entry(away_formation).or_default();
        away_stats.uses += 1;
        away_stats.goals += away_goals;
        away_stats.shots += away_shots;
    }

    pub fn goals_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.goals as f64 / self.matches as f64
    }

    pub fn shots_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.shots as f64 / self.matches as f64
    }

    pub fn home_win_rate(&self) -> f64 {
        self.rate(self.home_wins)
    }

    pub fn draw_rate(&self) -> f64 {
        self.rate(self.draws)
    }

    pub fn away_win_rate(&self) -> f64 {
        self.rate(self.away_wins)
    }

    fn rate(&self, n: u32) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        n as f64 / self.matches as f64
    }

    pub fn shot_on_target_rate(&self) -> f64 {
        if self.shots == 0 {
            return 0.0;
        }
        self.shots_on_target as f64 / self.shots as f64
    }

    pub fn conversion_rate(&self) -> f64 {
        if self.shots == 0 {
            return 0.0;
        }
        self.goals as f64 / self.shots as f64
    }

    /// Share of goals scored via `ShotKind::Header` — the headed-goal-share
    /// calibration target (`MATCH_MODEL.md` §8).
    pub fn headed_goal_share(&self) -> f64 {
        if self.goals == 0 {
            return 0.0;
        }
        *self.goals_by_kind.get(&ShotKind::Header).unwrap_or(&0) as f64 / self.goals as f64
    }

    /// Share of goals arriving via `ShotSource::Cross` or `Cutback` — the
    /// wide-origin-goal-share calibration target (`MATCH_MODEL.md` §8:
    /// "cross + cutback"), 25-35%.
    pub fn wide_origin_goal_share(&self) -> f64 {
        if self.goals == 0 {
            return 0.0;
        }
        let cross = *self.goals_by_source.get(&ShotSource::Cross).unwrap_or(&0);
        let cutback = *self.goals_by_source.get(&ShotSource::Cutback).unwrap_or(&0);
        (cross + cutback) as f64 / self.goals as f64
    }

    /// Home share of the possession-proxy event count.
    pub fn home_possession_share(&self) -> f64 {
        let total = self.home_events + self.away_events;
        if total == 0 {
            return 0.0;
        }
        self.home_events as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_engine::Zone;
    use crate::match_engine::stream::MatchEvent;

    fn shot(side: Side, kind: ShotKind, source: ShotSource, outcome: ShotOutcome) -> MatchEvent {
        MatchEvent {
            minute: 10,
            side,
            zone: Zone::Box,
            kind: MatchEventKind::Shot {
                kind,
                source,
                outcome,
            },
        }
    }

    #[test]
    fn record_reproduces_hand_counted_aggregates() {
        // Hand-built trace: home score once (through-ball Finish), miss
        // once (Off), home scores a Header (from a Cross); away score once
        // (LongShot), get one Saved.
        let stream = vec![
            shot(
                Side::Home,
                ShotKind::Finish,
                ShotSource::Through,
                ShotOutcome::Goal,
            ),
            shot(
                Side::Home,
                ShotKind::Finish,
                ShotSource::Cutback,
                ShotOutcome::Off,
            ),
            shot(
                Side::Home,
                ShotKind::Header,
                ShotSource::Cross,
                ShotOutcome::Goal,
            ),
            shot(
                Side::Away,
                ShotKind::LongShot,
                ShotSource::Long,
                ShotOutcome::Goal,
            ),
            shot(
                Side::Away,
                ShotKind::LongShot,
                ShotSource::Long,
                ShotOutcome::Saved,
            ),
            MatchEvent {
                minute: 20,
                side: Side::Home,
                zone: Zone::Mid,
                kind: MatchEventKind::Pass { success: true },
            },
            MatchEvent {
                minute: 21,
                side: Side::Away,
                zone: Zone::Mid,
                kind: MatchEventKind::Pass { success: true },
            },
        ];
        let outcome = MatchOutcome {
            home_goals: 2,
            away_goals: 1,
            stream,
        };

        let mut telemetry = StreamTelemetry::default();
        telemetry.record(&outcome, 0, 2); // home 4-4-2, away 4-2-3-1

        assert_eq!(telemetry.matches, 1);
        assert_eq!(telemetry.goals, 3);
        assert_eq!(telemetry.home_wins, 1);
        assert_eq!(telemetry.draws, 0);
        assert_eq!(telemetry.away_wins, 0);
        assert_eq!(telemetry.shots, 5);
        assert_eq!(telemetry.shots_on_target, 4); // 3 goals + 1 saved
        assert_eq!(telemetry.goals_by_kind.get(&ShotKind::Finish), Some(&1));
        assert_eq!(telemetry.goals_by_kind.get(&ShotKind::Header), Some(&1));
        assert_eq!(telemetry.goals_by_kind.get(&ShotKind::LongShot), Some(&1));
        assert_eq!(
            telemetry.goals_by_source.get(&ShotSource::Through),
            Some(&1)
        );
        assert_eq!(telemetry.goals_by_source.get(&ShotSource::Cross), Some(&1));
        assert_eq!(telemetry.goals_by_source.get(&ShotSource::Long), Some(&1));
        assert_eq!(telemetry.goals_by_source.get(&ShotSource::Cutback), None); // the Cutback shot was Off, not a goal
        assert_eq!(telemetry.home_events, 4); // 3 home shots + 1 home pass
        assert_eq!(telemetry.away_events, 3); // 2 away shots + 1 away pass

        let home_formation = telemetry.by_formation.get(&0).unwrap();
        assert_eq!(home_formation.uses, 1);
        assert_eq!(home_formation.goals, 2);
        assert_eq!(home_formation.shots, 3);

        let away_formation = telemetry.by_formation.get(&2).unwrap();
        assert_eq!(away_formation.uses, 1);
        assert_eq!(away_formation.goals, 1);
        assert_eq!(away_formation.shots, 2);

        assert_eq!(telemetry.goals_per_match(), 3.0);
        assert_eq!(telemetry.shots_per_match(), 5.0);
        assert!((telemetry.shot_on_target_rate() - 0.8).abs() < 1e-9);
        assert!((telemetry.conversion_rate() - 0.6).abs() < 1e-9);
        assert!((telemetry.headed_goal_share() - (1.0 / 3.0)).abs() < 1e-9);
        assert!((telemetry.wide_origin_goal_share() - (1.0 / 3.0)).abs() < 1e-9); // the Cross goal only
        assert!((telemetry.home_possession_share() - (4.0 / 7.0)).abs() < 1e-9);
    }
}
