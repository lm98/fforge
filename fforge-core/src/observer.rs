//! The trace/telemetry spine, instrumented from Phase 1 (DESIGN.md §9).
//!
//! An `EventObserver` is a **passive consumer of the event stream — it never
//! writes to the world.** This is the seam the reusable evaluation kernel
//! grows along: trace capture, scenario replay, and scoring all subscribe
//! here. `SeasonTelemetry` is the first concrete consumer, and doubles as the
//! embryo of the Phase-2 calibration harness (goals/game, home advantage —
//! the exact aggregates the harness will check against reality).

use crate::event::Event;

pub trait EventObserver {
    fn on_event(&mut self, event: &Event);
}

#[derive(Debug, Default, Clone)]
pub struct SeasonTelemetry {
    pub matches: u32,
    pub goals: u32,
    pub home_wins: u32,
    pub draws: u32,
    pub away_wins: u32,
    pub scoreline_counts: std::collections::BTreeMap<(u8, u8), u32>,
}

impl SeasonTelemetry {
    pub fn goals_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.goals as f64 / self.matches as f64
    }

    pub fn home_win_rate(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.home_wins as f64 / self.matches as f64
    }
}

impl EventObserver for SeasonTelemetry {
    fn on_event(&mut self, event: &Event) {
        if let Event::MatchPlayed {
            home_goals,
            away_goals,
            ..
        } = event
        {
            self.matches += 1;
            self.goals += (*home_goals + *away_goals) as u32;
            match home_goals.cmp(away_goals) {
                std::cmp::Ordering::Greater => self.home_wins += 1,
                std::cmp::Ordering::Equal => self.draws += 1,
                std::cmp::Ordering::Less => self.away_wins += 1,
            }
            *self
                .scoreline_counts
                .entry((*home_goals, *away_goals))
                .or_default() += 1;
        }
    }
}