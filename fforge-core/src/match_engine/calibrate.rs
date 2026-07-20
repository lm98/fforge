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

/// Bin width, in CA points, for the strength-gap → expected-points
/// histogram (`MATCH_MODEL.md` §10 item 6). `~2` keeps bins narrow enough to
/// see slope while still accumulating enough matches per bin over a
/// multi-seed pool.
const STRENGTH_GAP_BIN_WIDTH: f64 = 2.0;

/// Bin index for a `home_strength - away_strength` gap: bin `i` covers
/// `[i * STRENGTH_GAP_BIN_WIDTH, (i + 1) * STRENGTH_GAP_BIN_WIDTH)`. Bins
/// are not clamped to a fixed range — sparse bins at the extremes just
/// accumulate fewer matches — but callers should expect the populated range
/// to run roughly ±20 given fforge's CA scale and squad-generation spread.
fn strength_gap_bin(gap: f64) -> i32 {
    (gap / STRENGTH_GAP_BIN_WIDTH).floor() as i32
}

/// Outcome counts for one strength-gap bin — the raw material for the
/// empirical **home expected-points share** `(wins + 0.5*draws)/matches`,
/// the quantity the Elo reference curve (`elo_expected`) is compared
/// against.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GapBinStats {
    pub matches: u32,
    pub home_wins: u32,
    pub draws: u32,
    pub away_wins: u32,
}

impl GapBinStats {
    /// Empirical home expected-points share for this bin: win = 1 point,
    /// draw = 0.5, loss = 0 — the standard points-share convention that
    /// handles draws without conflating them into a win probability.
    pub fn expected_points(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        (self.home_wins as f64 + 0.5 * self.draws as f64) / self.matches as f64
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
    /// Keyed by `strength_gap_bin(home_strength - away_strength)` — the
    /// bookmaker-baseline calibration axis (`MATCH_MODEL.md` §10 item 6):
    /// does the engine's favourite-vs-underdog discrimination look sane,
    /// scored against `elo_expected` in `score_against_reference`.
    pub by_strength_gap: BTreeMap<i32, GapBinStats>,
}

impl StreamTelemetry {
    /// Fold one match's trace in. `home_formation`/`away_formation` are the
    /// `Lineup::formation` index each side fielded, for the per-formation
    /// breakdown (`MATCH_MODEL.md` §10 item 1's diagnostic).
    /// `home_strength`/`away_strength` are each side's `lineup_strength` —
    /// mean best-role CA across the XI — for the strength-gap →
    /// expected-points bin (`MATCH_MODEL.md` §10 item 6).
    pub fn record(
        &mut self,
        outcome: &MatchOutcome,
        home_formation: u8,
        away_formation: u8,
        home_strength: f64,
        away_strength: f64,
    ) {
        self.matches += 1;
        self.goals += outcome.home_goals as u32 + outcome.away_goals as u32;
        match outcome.home_goals.cmp(&outcome.away_goals) {
            std::cmp::Ordering::Greater => self.home_wins += 1,
            std::cmp::Ordering::Equal => self.draws += 1,
            std::cmp::Ordering::Less => self.away_wins += 1,
        }

        let gap_bin = self
            .by_strength_gap
            .entry(strength_gap_bin(home_strength - away_strength))
            .or_default();
        gap_bin.matches += 1;
        match outcome.home_goals.cmp(&outcome.away_goals) {
            std::cmp::Ordering::Greater => gap_bin.home_wins += 1,
            std::cmp::Ordering::Equal => gap_bin.draws += 1,
            std::cmp::Ordering::Less => gap_bin.away_wins += 1,
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

    /// The empirical expected-points-vs-strength-gap curve: one row per
    /// populated bin, `(gap_bin_center, expected_points, matches)`, sorted
    /// by ascending gap. `gap_bin_center` is the midpoint of the
    /// `STRENGTH_GAP_BIN_WIDTH`-wide bin (`MATCH_MODEL.md` §10 item 6).
    pub fn expected_points_curve(&self) -> Vec<(f64, f64, u32)> {
        self.by_strength_gap
            .iter()
            .map(|(&bin, stats)| {
                let center = (bin as f64 + 0.5) * STRENGTH_GAP_BIN_WIDTH;
                (center, stats.expected_points(), stats.matches)
            })
            .collect()
    }

    /// Score the empirical expected-points curve (`expected_points_curve`)
    /// against `elo_expected(gap, s)` over the populated bins. Bins with
    /// zero matches never appear (see `expected_points_curve`), so every
    /// row here is measured, not extrapolated.
    pub fn score_against_reference(&self, s: f64) -> DeviationReport {
        let per_bin: Vec<GapDeviation> = self
            .expected_points_curve()
            .into_iter()
            .map(|(gap, empirical, matches)| {
                let reference = elo_expected(gap, s);
                GapDeviation {
                    gap,
                    matches,
                    empirical,
                    reference,
                    deviation: empirical - reference,
                }
            })
            .collect();

        let max_abs_deviation = per_bin
            .iter()
            .map(|b| b.deviation.abs())
            .fold(0.0_f64, f64::max);

        let total_matches: u32 = per_bin.iter().map(|b| b.matches).sum();
        let weighted_mean_abs_deviation = if total_matches == 0 {
            0.0
        } else {
            per_bin
                .iter()
                .map(|b| b.deviation.abs() * b.matches as f64)
                .sum::<f64>()
                / total_matches as f64
        };

        DeviationReport {
            per_bin,
            max_abs_deviation,
            weighted_mean_abs_deviation,
        }
    }
}

/// Elo scale constant (in CA points) for the bookmaker-baseline reference
/// curve (`MATCH_MODEL.md` §10 item 6). Chosen, not fitted: a ~10-CA-point
/// lineup-strength edge should read as a believable top-flight-ish
/// favourite, and `elo_expected(10.0, 40.0) ≈ 0.64` (0.5 + a ~14-point
/// expected-points edge) sits squarely in that ~0.6-0.65 band. This is a
/// documented modelling choice for the reference curve, not a fit target —
/// don't tune it to flatter the engine's own curve.
pub const ELO_SCALE_S: f64 = 40.0;

/// The Elo expected-score curve, reused here as an expected-*points*-share
/// reference: `1 / (1 + 10^(-gap/s))`. `gap` is a strength difference (here,
/// `home_strength - away_strength` in CA points) and `s` is the scale
/// (`ELO_SCALE_S`). Standard Elo treats this as P(win) for a no-draw game;
/// fforge's matches have draws, so it is compared against the empirical
/// **expected points share** `(wins + 0.5*draws)/matches`
/// (`GapBinStats::expected_points`), not P(home win) — see the module doc
/// for why equating the two would misread the draw mass as miscalibration.
pub fn elo_expected(gap: f64, s: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf(-gap / s))
}

/// Per-bin deviation of the empirical expected-points curve from
/// `elo_expected`. `deviation` is signed (`empirical - reference`); `score_against_reference`
/// reports both signed per-bin rows and unsigned (`abs`) summary stats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GapDeviation {
    pub gap: f64,
    pub matches: u32,
    pub empirical: f64,
    pub reference: f64,
    pub deviation: f64,
}

/// The result of scoring `StreamTelemetry`'s empirical curve against
/// `elo_expected`: this measures **slope/discrimination** against a
/// reference curve, not absolute correctness, and it is not a second
/// home-advantage test — the home-advantage *level* is validated by the
/// H/D/A aggregate elsewhere; this axis is new only in that it checks how
/// fast expected points moves with the strength gap.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviationReport {
    pub per_bin: Vec<GapDeviation>,
    pub max_abs_deviation: f64,
    pub weighted_mean_abs_deviation: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_engine::Zone;
    use crate::match_engine::stream::MatchEvent;
    use fforge_domain::PlayerId;

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
            actor: PlayerId(0),
            opponent: None,
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
                actor: PlayerId(0),
                opponent: None,
            },
            MatchEvent {
                minute: 21,
                side: Side::Away,
                zone: Zone::Mid,
                kind: MatchEventKind::Pass { success: true },
                actor: PlayerId(0),
                opponent: None,
            },
        ];
        let outcome = MatchOutcome {
            home_goals: 2,
            away_goals: 1,
            stream,
        };

        let mut telemetry = StreamTelemetry::default();
        telemetry.record(&outcome, 0, 2, 55.0, 50.0); // home 4-4-2 (stronger), away 4-2-3-1

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

        // gap = 55.0 - 50.0 = 5.0 -> bin 2 (covers [4.0, 6.0)), a home win.
        let gap_bin = telemetry.by_strength_gap.get(&2).unwrap();
        assert_eq!(gap_bin.matches, 1);
        assert_eq!(gap_bin.home_wins, 1);
        assert_eq!(gap_bin.draws, 0);
        assert_eq!(gap_bin.away_wins, 0);

        assert_eq!(telemetry.goals_per_match(), 3.0);
        assert_eq!(telemetry.shots_per_match(), 5.0);
        assert!((telemetry.shot_on_target_rate() - 0.8).abs() < 1e-9);
        assert!((telemetry.conversion_rate() - 0.6).abs() < 1e-9);
        assert!((telemetry.headed_goal_share() - (1.0 / 3.0)).abs() < 1e-9);
        assert!((telemetry.wide_origin_goal_share() - (1.0 / 3.0)).abs() < 1e-9); // the Cross goal only
        assert!((telemetry.home_possession_share() - (4.0 / 7.0)).abs() < 1e-9);
    }

    #[test]
    fn strength_gap_binning_and_expected_points_match_hand_counts() {
        fn bare_outcome(home_goals: u8, away_goals: u8) -> MatchOutcome {
            MatchOutcome {
                home_goals,
                away_goals,
                stream: Vec::new(),
            }
        }

        let mut telemetry = StreamTelemetry::default();
        // Bin 2 covers [4.0, 6.0): two home wins, one draw.
        telemetry.record(&bare_outcome(2, 0), 0, 0, 55.0, 50.0); // gap 5.0
        telemetry.record(&bare_outcome(3, 1), 0, 0, 54.0, 49.0); // gap 5.0
        telemetry.record(&bare_outcome(1, 1), 0, 0, 54.5, 50.0); // gap 4.5
        // Bin -3 covers [-6.0, -4.0): one away win.
        telemetry.record(&bare_outcome(0, 2), 0, 0, 48.0, 53.0); // gap -5.0

        let bin2 = telemetry.by_strength_gap.get(&2).unwrap();
        assert_eq!(bin2.matches, 3);
        assert_eq!(bin2.home_wins, 2);
        assert_eq!(bin2.draws, 1);
        assert_eq!(bin2.away_wins, 0);
        assert!((bin2.expected_points() - (2.5 / 3.0)).abs() < 1e-9);

        let bin_neg3 = telemetry.by_strength_gap.get(&-3).unwrap();
        assert_eq!(bin_neg3.matches, 1);
        assert_eq!(bin_neg3.away_wins, 1);
        assert_eq!(bin_neg3.expected_points(), 0.0);

        let curve = telemetry.expected_points_curve();
        assert_eq!(curve.len(), 2);
        // Sorted ascending by gap: bin -3 (center -5.0) before bin 2 (center 5.0).
        assert_eq!(curve[0], (-5.0, 0.0, 1));
        assert!((curve[1].0 - 5.0).abs() < 1e-9);
        assert!((curve[1].1 - (2.5 / 3.0)).abs() < 1e-9);
        assert_eq!(curve[1].2, 3);
    }

    #[test]
    fn elo_expected_is_the_standard_logistic_curve() {
        assert!((elo_expected(0.0, ELO_SCALE_S) - 0.5).abs() < 1e-9);
        // A ~10-CA-point edge should land in the documented ~0.6-0.65 band.
        let e10 = elo_expected(10.0, ELO_SCALE_S);
        assert!((0.6..=0.65).contains(&e10), "elo_expected(10, S) = {e10}");
        // Symmetric around the gap: favourite's edge mirrors underdog's deficit.
        assert!((elo_expected(-10.0, ELO_SCALE_S) - (1.0 - e10)).abs() < 1e-9);
    }

    /// Builds a `StreamTelemetry` whose `by_strength_gap` bins have exactly
    /// the given `(gap_bin_center, expected_points, matches)` rows, bypassing
    /// `record` so the deviation scorer can be tested against curves with
    /// known shapes rather than ones assembled from simulated matches.
    fn telemetry_with_curve(rows: &[(f64, f64, u32)]) -> StreamTelemetry {
        let mut telemetry = StreamTelemetry::default();
        for &(gap_center, expected_points, matches) in rows {
            let bin = strength_gap_bin(gap_center);
            // expected_points = (home_wins + 0.5*draws) / matches; represent
            // it as an all-draws mix so any fractional value is exact.
            let home_wins = (expected_points * matches as f64).round() as u32;
            telemetry.by_strength_gap.insert(
                bin,
                GapBinStats {
                    matches,
                    home_wins,
                    draws: 0,
                    away_wins: matches - home_wins,
                },
            );
        }
        telemetry
    }

    #[test]
    fn score_against_reference_reads_near_zero_for_a_matching_curve() {
        let rows: Vec<(f64, f64, u32)> = (-4..=4)
            .map(|i| {
                let gap = i as f64 * STRENGTH_GAP_BIN_WIDTH + STRENGTH_GAP_BIN_WIDTH / 2.0;
                (gap, elo_expected(gap, ELO_SCALE_S), 100)
            })
            .collect();
        let telemetry = telemetry_with_curve(&rows);
        let report = telemetry.score_against_reference(ELO_SCALE_S);
        assert!(
            report.max_abs_deviation < 0.02,
            "matching curve should score near-zero deviation, got {}",
            report.max_abs_deviation
        );
        assert!(report.weighted_mean_abs_deviation < 0.02);
    }

    #[test]
    fn score_against_reference_flags_a_deliberately_flat_curve() {
        // A coin-flip-football curve: expected points stuck at 0.5
        // regardless of strength gap — no discrimination at all.
        let rows: Vec<(f64, f64, u32)> = (-4..=4)
            .map(|i| {
                let gap = i as f64 * STRENGTH_GAP_BIN_WIDTH + STRENGTH_GAP_BIN_WIDTH / 2.0;
                (gap, 0.5, 100)
            })
            .collect();
        let telemetry = telemetry_with_curve(&rows);
        let report = telemetry.score_against_reference(ELO_SCALE_S);
        assert!(
            report.max_abs_deviation > 0.1,
            "a flat curve should show a large deviation at the extreme bins, got {}",
            report.max_abs_deviation
        );
    }

    /// The favourite-discrimination regression guard (`MATCH_MODEL.md` §10
    /// item 6): a sibling to `aggregates_are_in_a_believable_ballpark`
    /// (`lib.rs`), pooled over real `worldgen` + AI lineup selection + the
    /// match engine — bypassing the event fold entirely (`StreamTelemetry`
    /// consumes `MatchOutcome` directly, never `SeasonTelemetry`), per the
    /// same rationale `bin/calibrate.rs`'s doc comment gives. It checks two
    /// things, both wide sanity bands meant to catch gross regressions, not
    /// enforce a precise fit: (1) the empirical expected-points curve is
    /// monotonic non-decreasing in the strength gap, up to a noise
    /// tolerance sized from the per-bin sampling error at the match counts
    /// this pool produces; (2) the max deviation from `elo_expected` stays
    /// under a documented band. A run at seeds 0..8 (`cargo run --bin
    /// calibrate -- --seeds 8`) showed the empirical curve is markedly
    /// *steeper* than the S=40 reference (max |deviation| ~0.32) — the
    /// engine discriminates favourites more sharply than the reference
    /// curve, which is fine (§10 item 6 is a discrimination sanity check,
    /// not a fit target) but sets the deviation band well above zero.
    ///
    /// **Feature-gated behind `slow-tests`, ignored by default.** This is a
    /// knob-change regression tripwire, not a unit test: a commit that touches
    /// neither `*Knobs` nor a sim module can't trip it, so running it on every
    /// `cargo test` is wasted wall-clock. It runs in the PR-required fast
    /// suite's absence deliberately — CI instead runs it nightly and on any PR
    /// touching `*Knobs`, `development`, `match_engine`, `market`,
    /// `valuation`, `club_ai`, or `pool`. `#[ignore]` here is a scheduling
    /// choice, not neglect; run it locally with `cargo test --features
    /// slow-tests`.
    #[cfg_attr(not(feature = "slow-tests"), ignore)]
    #[test]
    fn favourite_discrimination_regression_guard() {
        let cfg = crate::WorldGenConfig::default();
        let mut telemetry = StreamTelemetry::default();
        for seed in 0..8u64 {
            let (world, schedule, _start) = crate::worldgen::generate(seed, &cfg);
            for fixture in &schedule {
                let home_lineup = crate::match_engine::ai_pick_lineup(&world, fixture.home);
                let away_lineup = crate::match_engine::ai_pick_lineup(&world, fixture.away);
                let home_strength = crate::match_engine::lineup_strength(&world, &home_lineup);
                let away_strength = crate::match_engine::lineup_strength(&world, &away_lineup);
                let mut rng =
                    crate::rng::derive_stream(seed, crate::FIXTURE_STREAM_NS | fixture.id.0 as u64);
                let outcome =
                    crate::match_engine::play_match(&world, &home_lineup, &away_lineup, &mut rng);
                telemetry.record(
                    &outcome,
                    home_lineup.formation,
                    away_lineup.formation,
                    home_strength,
                    away_strength,
                );
            }
        }

        let curve = telemetry.expected_points_curve(); // ascending by gap (BTreeMap order)
        const MONOTONIC_TOLERANCE: f64 = 0.05;
        for pair in curve.windows(2) {
            let (gap_a, ep_a, matches_a) = pair[0];
            let (gap_b, ep_b, matches_b) = pair[1];
            assert!(
                ep_b >= ep_a - MONOTONIC_TOLERANCE,
                "expected points dipped from {ep_a:.3} at gap {gap_a:.1} ({matches_a} matches) \
                 to {ep_b:.3} at gap {gap_b:.1} ({matches_b} matches), beyond the \
                 {MONOTONIC_TOLERANCE} noise tolerance"
            );
        }

        let report = telemetry.score_against_reference(ELO_SCALE_S);
        const MAX_DEVIATION_BAND: f64 = 0.5;
        assert!(
            report.max_abs_deviation <= MAX_DEVIATION_BAND,
            "max deviation from the Elo reference curve ({:.3}) exceeds the {MAX_DEVIATION_BAND} \
             sanity band",
            report.max_abs_deviation
        );
    }
}
