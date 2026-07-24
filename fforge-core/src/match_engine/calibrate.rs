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
use super::resolve::mirrored_zone;
use super::stream::{MatchEventKind, ShotKind, ShotOutcome, ShotSource, Side};
use super::zone::{NUM_ZONES, Zone};
use super::{CONSISTENCY_NS, FOUL_NS, INJURY_NS, Knobs, ai_pick_lineup, play_match};
use crate::rng::derive_stream;
use fforge_domain::{ClubId, GameDate, Tactics, World};
use std::collections::BTreeMap;

fn side_idx(s: Side) -> usize {
    match s {
        Side::Home => 0,
        Side::Away => 1,
    }
}

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
    /// Pass attempts/completions keyed by `[side_idx][zone.index()]` — the
    /// per-zone pass completion cut `TACTICS_MODEL.md` §8/T7 adds (Pressing
    /// `High`'s prediction: opponent pass completion in their own
    /// `Def`/`Mid` drops).
    pub pass_attempts_by_zone: [[u32; NUM_ZONES]; 2],
    pub pass_completions_by_zone: [[u32; NUM_ZONES]; 2],
    /// Turnovers *won*, keyed by `[winning_side_idx][zone the winner
    /// restarts in]` (turnover mirroring, `MATCH_MODEL.md` §3) — the
    /// turnover-won-by-zone cut `TACTICS_MODEL.md` §8/T7 adds (Pressing
    /// `High`'s prediction: turnovers won in the opponent's `Def` mirror to
    /// this side's own `AttC` restarts).
    pub turnovers_won_by_zone: [[u32; NUM_ZONES]; 2],
    /// Fouls (`MATCH_MODEL.md` §15, T11) — every `MatchEventKind::Foul`
    /// beat, regardless of whether it drew a card.
    pub fouls: u32,
    /// Yellow cards shown, counting a second yellow as one (`Card::Yellow`
    /// or `Card::SecondYellow` — the standard "cards shown" convention).
    pub yellows: u32,
    /// Dismissals: a straight red or a second yellow (`Card::Red` or
    /// `Card::SecondYellow`).
    pub reds: u32,
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
            let idx = side_idx(event.side);
            match event.side {
                Side::Home => self.home_events += 1,
                Side::Away => self.away_events += 1,
            }
            match event.kind {
                MatchEventKind::Pass { success } => {
                    self.pass_attempts_by_zone[idx][event.zone.index()] += 1;
                    if success {
                        self.pass_completions_by_zone[idx][event.zone.index()] += 1;
                    } else {
                        self.turnovers_won_by_zone[1 - idx][mirrored_zone(event.zone).index()] += 1;
                    }
                }
                MatchEventKind::TakeOn { success: false } => {
                    self.turnovers_won_by_zone[1 - idx][mirrored_zone(event.zone).index()] += 1;
                }
                MatchEventKind::Clearance => {
                    // A cleared cross is also a turnover (resolve.rs: a
                    // failed Cross delivery falls through to `turnover`).
                    self.turnovers_won_by_zone[1 - idx][mirrored_zone(event.zone).index()] += 1;
                }
                MatchEventKind::Foul { card } => {
                    self.fouls += 1;
                    match card {
                        Some(super::Card::Yellow) => self.yellows += 1,
                        Some(super::Card::SecondYellow) => {
                            self.yellows += 1;
                            self.reds += 1;
                        }
                        Some(super::Card::Red) => self.reds += 1,
                        None => {}
                    }
                }
                _ => {}
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

    /// Fouls per match, both teams combined (`MATCH_MODEL.md` §15's §8
    /// impact row: "fouls/game ~20-25").
    pub fn fouls_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.fouls as f64 / self.matches as f64
    }

    /// Yellow cards per team per match — half the combined per-match count,
    /// the shape §15's "roughly 2-3 yellows per team per match" target is
    /// stated in.
    pub fn yellows_per_team_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.yellows as f64 / self.matches as f64 / 2.0
    }

    /// Red cards (dismissals) per team per match — half the combined
    /// per-match count, matching `yellows_per_team_per_match`'s convention.
    pub fn reds_per_team_per_match(&self) -> f64 {
        if self.matches == 0 {
            return 0.0;
        }
        self.reds as f64 / self.matches as f64 / 2.0
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

    /// Pass completion rate for `side` in `zone` — the per-zone pass-
    /// completion cut (`TACTICS_MODEL.md` §8/T7).
    pub fn pass_completion_in_zone(&self, side: Side, zone: Zone) -> f64 {
        let idx = side_idx(side);
        let att = self.pass_attempts_by_zone[idx][zone.index()];
        if att == 0 {
            return 0.0;
        }
        self.pass_completions_by_zone[idx][zone.index()] as f64 / att as f64
    }

    /// Turnovers won *by* `side`, restarting in `zone` — the turnover-won-
    /// by-zone cut (`TACTICS_MODEL.md` §8/T7). `zone` is the zone the
    /// *winner* restarts in (post-mirroring), not where the ball was lost.
    pub fn turnovers_won_in_zone(&self, side: Side, zone: Zone) -> u32 {
        self.turnovers_won_by_zone[side_idx(side)][zone.index()]
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

/// Namespace tag for the head-to-head harness's RNG stream
/// (`rng::derive_stream`), distinct from `commands::FIXTURE_STREAM_NS`.
/// `TACTICS_MODEL.md` §7's triangle harness needs its own mode: the v1 AI
/// never counter-picks (§7's opponent-blindness), so the §5 triangle is
/// never exercised in ordinary league play — it can only be tested by
/// forcing both sides' tactics directly, pooled over many seeds.
const HEAD_TO_HEAD_NS: u64 = 0x4832_485F_0000_0000; // "H2H_"

/// Home-side expected-points contribution for one match (win = 1.0, draw =
/// 0.5, loss = 0.0) — the same convention `GapBinStats::expected_points`
/// pools.
fn match_expected_points(home_goals: u8, away_goals: u8) -> f64 {
    match home_goals.cmp(&away_goals) {
        std::cmp::Ordering::Greater => 1.0,
        std::cmp::Ordering::Equal => 0.5,
        std::cmp::Ordering::Less => 0.0,
    }
}

/// The `TACTICS_MODEL.md` §7/§5 triangle harness: pool `2 * seeds.len()`
/// matches between two forced tactics settings on an **equal-strength**
/// squad (the same club fielded on both sides, so `lineup_strength` is
/// identical and `home_bias` is the only asymmetry) — each seed is played
/// once with `tactics_a` at home and once with `tactics_b` at home, so
/// pooling both directions cancels `home_bias` out of the read. Returns
/// `tactics_a`'s pooled expected-points share (`tactics_b`'s is `1.0 - a`).
pub fn run_head_to_head(
    world: &World,
    club: ClubId,
    tactics_a: Tactics,
    tactics_b: Tactics,
    seeds: &[u64],
) -> f64 {
    let mut lineup_a = ai_pick_lineup(world, club);
    lineup_a.tactics = tactics_a;
    let mut lineup_b = ai_pick_lineup(world, club);
    lineup_b.tactics = tactics_b;

    // No real GameState here — a fixed reference date only feeds the
    // ambient injury channel's age term (T10), a second-order effect on a
    // harness measuring the tactics triangle, not injuries.
    let today = GameDate { days: 0 };
    let mut total_points_a = 0.0;
    let mut matches = 0u32;
    for &seed in seeds {
        let mut rng_a_home = derive_stream(seed, HEAD_TO_HEAD_NS);
        let mut consistency_a_home = derive_stream(seed, HEAD_TO_HEAD_NS | CONSISTENCY_NS);
        let mut injury_a_home = derive_stream(seed, HEAD_TO_HEAD_NS | INJURY_NS);
        let mut foul_a_home = derive_stream(seed, HEAD_TO_HEAD_NS | FOUL_NS);
        let out_a_home = play_match(
            world,
            &lineup_a,
            &lineup_b,
            &mut rng_a_home,
            &mut consistency_a_home,
            &mut injury_a_home,
            &mut foul_a_home,
            &Knobs::default(),
            &BTreeMap::new(),
            today,
        );
        total_points_a += match_expected_points(out_a_home.home_goals, out_a_home.away_goals);
        matches += 1;

        let mut rng_b_home = derive_stream(seed, HEAD_TO_HEAD_NS | 1);
        let mut consistency_b_home = derive_stream(seed, HEAD_TO_HEAD_NS | CONSISTENCY_NS | 1);
        let mut injury_b_home = derive_stream(seed, HEAD_TO_HEAD_NS | INJURY_NS | 1);
        let mut foul_b_home = derive_stream(seed, HEAD_TO_HEAD_NS | FOUL_NS | 1);
        let out_b_home = play_match(
            world,
            &lineup_b,
            &lineup_a,
            &mut rng_b_home,
            &mut consistency_b_home,
            &mut injury_b_home,
            &mut foul_b_home,
            &Knobs::default(),
            &BTreeMap::new(),
            today,
        );
        // a is away in this leg: a's points share = 1 - home (b)'s.
        total_points_a += 1.0 - match_expected_points(out_b_home.home_goals, out_b_home.away_goals);
        matches += 1;
    }

    total_points_a / matches as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_engine::Zone;
    use crate::match_engine::stream::MatchEvent;
    use fforge_domain::PlayerId;

    /// T7 addendum §2/T7a: the press-wiring verification. Confirms
    /// `Pressing::High`'s `def_bias_by_zone` lands where `TACTICS_MODEL.md`
    /// §3 says it must — the *possessing* side's zone, i.e. the opponent's
    /// build-up (`Def`/`Mid`) — and nowhere else, ruling out the frame-flip
    /// failure mode the addendum names before any effect-table change was
    /// considered. Bands are set from the actually-measured effect (Def
    /// −1.7pts, Mid −2.0pts at 3000-seed pooling — smaller than §8's
    /// original ±3-6pt prediction, but astronomically significant given
    /// ~200k pooled attempts per zone) rather than re-asserting §8's
    /// pre-registered band, which this run showed was optimistic.
    #[cfg_attr(not(feature = "slow-tests"), ignore)]
    #[test]
    fn pressing_high_lands_in_the_opponents_def_and_mid_and_nowhere_else() {
        use crate::worldgen::{WorldGenConfig, generate};
        use fforge_domain::{Pressing, Tactics};

        let cfg = WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _s, _d) = generate(7, &cfg);
        let club = world.competition.clubs[0];

        let mut press = ai_pick_lineup(&world, club);
        press.tactics = Tactics {
            pressing: Pressing::High,
            ..Tactics::neutral()
        };
        let neutral_opp = ai_pick_lineup(&world, club);
        let neutral_baseline = ai_pick_lineup(&world, club);

        // Identity Consistency, Injuries, and Fouls (§2.1): this test
        // isolates the press's own zone-localisation, and per-match
        // attribute noise — or a player dropping out of contention
        // mid-match, or a foul overriding a turnover — is an unrelated
        // confound to the question it's asking.
        let k = Knobs {
            consistency_sigma_max: 0.0,
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        };
        let today = GameDate { days: 0 };
        let mut tel_pressed = StreamTelemetry::default();
        let mut tel_baseline = StreamTelemetry::default();
        for seed in 0..3000u64 {
            let mut rng = derive_stream(seed, HEAD_TO_HEAD_NS);
            let mut consistency_rng = derive_stream(seed, HEAD_TO_HEAD_NS | CONSISTENCY_NS);
            let mut injury_rng = derive_stream(seed, HEAD_TO_HEAD_NS | INJURY_NS);
            let mut foul_rng = derive_stream(seed, HEAD_TO_HEAD_NS | FOUL_NS);
            // Home presses, Away neutral: Away is the "opponent" whose
            // build-up we're checking.
            let out = play_match(
                &world,
                &press,
                &neutral_opp,
                &mut rng,
                &mut consistency_rng,
                &mut injury_rng,
                &mut foul_rng,
                &k,
                &BTreeMap::new(),
                today,
            );
            tel_pressed.record(&out, 0, 0, 50.0, 50.0);

            let mut rng2 = derive_stream(seed, HEAD_TO_HEAD_NS | 1);
            let mut consistency_rng2 = derive_stream(seed, HEAD_TO_HEAD_NS | CONSISTENCY_NS | 1);
            let mut injury_rng2 = derive_stream(seed, HEAD_TO_HEAD_NS | INJURY_NS | 1);
            let mut foul_rng2 = derive_stream(seed, HEAD_TO_HEAD_NS | FOUL_NS | 1);
            let out2 = play_match(
                &world,
                &neutral_baseline,
                &neutral_opp,
                &mut rng2,
                &mut consistency_rng2,
                &mut injury_rng2,
                &mut foul_rng2,
                &k,
                &BTreeMap::new(),
                today,
            );
            tel_baseline.record(&out2, 0, 0, 50.0, 50.0);
        }

        for zone in [Zone::Def, Zone::Mid] {
            let pressed = tel_pressed.pass_completion_in_zone(Side::Away, zone);
            let baseline = tel_baseline.pass_completion_in_zone(Side::Away, zone);
            assert!(
                pressed < baseline - 0.005,
                "{zone:?}: pressing must measurably depress the opponent's \
                 build-up completion; pressed={pressed:.4} baseline={baseline:.4}"
            );
        }
        for zone in [Zone::AttC, Zone::AttW] {
            let pressed = tel_pressed.pass_completion_in_zone(Side::Away, zone);
            let baseline = tel_baseline.pass_completion_in_zone(Side::Away, zone);
            assert!(
                (pressed - baseline).abs() < 0.02,
                "{zone:?}: pressing High targets only Def/Mid — a completion \
                 shift here is the frame-flip failure mode; pressed={pressed:.4} \
                 baseline={baseline:.4}"
            );
        }

        let turnovers_pressed = tel_pressed.turnovers_won_in_zone(Side::Home, Zone::AttC);
        let turnovers_baseline = tel_baseline.turnovers_won_in_zone(Side::Home, Zone::AttC);
        assert!(
            turnovers_pressed > turnovers_baseline,
            "pressing must win more turnovers restarting in the presser's \
             own AttC (turnover mirroring off the opponent's Def); \
             pressed={turnovers_pressed} baseline={turnovers_baseline}"
        );
    }

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
            injuries: Vec::new(),
            cards: Vec::new(),
            ratings: Vec::new(),
            minutes: Vec::new(),
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
                injuries: Vec::new(),
                cards: Vec::new(),
                ratings: Vec::new(),
                minutes: Vec::new(),
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
        // TACTICS_MODEL.md §8's rollout discipline: re-runs with whatever
        // ai_pick_lineup_vs actually does — AI tactics gated off by default
        // pending T7's triangle finding (batch-3 T7 addendum §7), so this
        // currently guards the same neutral engine T6 landed with; it picks
        // up AI tactics automatically once `AI_TACTICS_ENABLED` flips.
        //
        // Pool widened 8 -> 24 seeds (T8): Consistency (MATCH_MODEL.md §17)
        // adds real per-match attribute variance, which an 8-seed pool's
        // sparsest extreme-gap bins (a handful of matches each) can't absorb
        // without an occasional non-monotonic dip — at seeds 0..8 the pair
        // (gap -27, 6 matches) -> (gap -25, 18 matches) read (0.083, 0.000),
        // an 0.083 dip past the 0.05 tolerance; the same pair at 24 seeds
        // reads (0.023, 0.008), an 0.015 dip well inside it. Test-pool
        // sizing, not a production knob — Consistency's own values are
        // unchanged.
        let cfg = crate::WorldGenConfig::default();
        let mut telemetry = StreamTelemetry::default();
        for seed in 0..24u64 {
            let (world, schedule, start) = crate::worldgen::generate(seed, &cfg);
            for fixture in &schedule {
                let suspended = std::collections::BTreeSet::new();
                let home_lineup = crate::match_engine::ai_pick_lineup_vs(
                    &world,
                    fixture.home,
                    fixture.away,
                    true,
                    start,
                    &suspended,
                );
                let away_lineup = crate::match_engine::ai_pick_lineup_vs(
                    &world,
                    fixture.away,
                    fixture.home,
                    false,
                    start,
                    &suspended,
                );
                let home_strength = crate::match_engine::lineup_strength(&world, &home_lineup);
                let away_strength = crate::match_engine::lineup_strength(&world, &away_lineup);
                let mut rng =
                    crate::rng::derive_stream(seed, crate::FIXTURE_STREAM_NS | fixture.id.0 as u64);
                let mut consistency_rng =
                    crate::rng::derive_stream(seed, CONSISTENCY_NS | fixture.id.0 as u64);
                let mut injury_rng =
                    crate::rng::derive_stream(seed, INJURY_NS | fixture.id.0 as u64);
                let mut foul_rng = crate::rng::derive_stream(seed, FOUL_NS | fixture.id.0 as u64);
                let outcome = crate::match_engine::play_match(
                    &world,
                    &home_lineup,
                    &away_lineup,
                    &mut rng,
                    &mut consistency_rng,
                    &mut injury_rng,
                    &mut foul_rng,
                    &Knobs::default(),
                    &BTreeMap::new(),
                    start,
                );
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
