//! Calibration runner (`MATCH_MODEL.md` §8, `docs/MATCH_MODEL.md` §10 item 1
//! diagnosis): drives the *real* worldgen + AI lineup selection + match
//! engine pipeline pooled over many seeds, and reports the emergent
//! aggregates plus a per-formation breakdown.
//!
//! Deliberately bypasses the event fold: `commands::advance_matchday` only
//! ever records the score (`Event::MatchPlayed`) and discards
//! `MatchOutcome.stream` (`MATCH_MODEL.md` §7) — everything this binary
//! reports (shots, SoT%, conversion, goal-source mix, the formation table)
//! only exists in that stream, so this harness reproduces the same fixture
//! list, the same per-fixture RNG derivation, and the same AI lineup
//! selection `advance_matchday` uses, and feeds `MatchOutcome` straight into
//! `StreamTelemetry` instead.
//!
//! Run with: `cargo run --bin calibrate -- --seeds 8`

use fforge_core::match_engine::{StreamTelemetry, ai_pick_lineup, play_match};
use fforge_core::rng::derive_stream;
use fforge_core::{FIXTURE_STREAM_NS, WorldGenConfig, worldgen};
use fforge_domain::FORMATIONS;

struct CalibReport {
    per_seed_gpm: Vec<f64>,
    pooled: StreamTelemetry,
}

fn run_calibration(seeds: &[u64], cfg: &WorldGenConfig) -> CalibReport {
    let mut pooled = StreamTelemetry::default();
    let mut per_seed_gpm = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        let (world, schedule, _start) = worldgen::generate(seed, cfg);
        let mut seed_goals = 0u32;
        let mut seed_matches = 0u32;

        for fixture in &schedule {
            let home_lineup = ai_pick_lineup(&world, fixture.home);
            let away_lineup = ai_pick_lineup(&world, fixture.away);
            let mut rng = derive_stream(seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
            let outcome = play_match(&world, &home_lineup, &away_lineup, &mut rng);

            seed_goals += outcome.home_goals as u32 + outcome.away_goals as u32;
            seed_matches += 1;
            pooled.record(&outcome, home_lineup.formation, away_lineup.formation);
        }

        per_seed_gpm.push(seed_goals as f64 / seed_matches as f64);
    }

    CalibReport {
        per_seed_gpm,
        pooled,
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn stdev(xs: &[f64], mean: f64) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
    var.sqrt()
}

fn print_report(report: &CalibReport) {
    let p = &report.pooled;
    let gpm_mean = mean(&report.per_seed_gpm);
    let gpm_sd = stdev(&report.per_seed_gpm, gpm_mean);
    let gpm_min = report
        .per_seed_gpm
        .iter()
        .cloned()
        .fold(f64::INFINITY, f64::min);
    let gpm_max = report
        .per_seed_gpm
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);

    println!(
        "=== Calibration report ({} seeds pooled, {} matches) ===",
        report.per_seed_gpm.len(),
        p.matches
    );
    println!();
    println!(
        "goals/match      : {gpm_mean:.2}  (sd {gpm_sd:.2}, range {gpm_min:.2}-{gpm_max:.2} across seeds)"
    );
    println!(
        "H / D / A         : {:.1}% / {:.1}% / {:.1}%",
        p.home_win_rate() * 100.0,
        p.draw_rate() * 100.0,
        p.away_win_rate() * 100.0
    );
    println!("shots/match       : {:.2}", p.shots_per_match());
    println!(
        "shots on target   : {:.1}%",
        p.shot_on_target_rate() * 100.0
    );
    println!("conversion        : {:.1}%", p.conversion_rate() * 100.0);
    println!("headed goal share : {:.1}%", p.headed_goal_share() * 100.0);
    println!(
        "wide-origin share : {:.1}%",
        p.wide_origin_goal_share() * 100.0
    );
    println!(
        "home possession   : {:.1}%",
        p.home_possession_share() * 100.0
    );
    println!();
    println!("=== Per-formation breakdown ===");
    println!(
        "{:<10} {:>10} {:>10} {:>14}",
        "formation", "uses", "gpm", "shots/match"
    );
    for (idx, stats) in &p.by_formation {
        let name = FORMATIONS.get(*idx as usize).map(|f| f.name).unwrap_or("?");
        println!(
            "{:<10} {:>10} {:>10.2} {:>14.2}",
            name,
            stats.uses,
            stats.goals_per_match(),
            stats.shots_per_match()
        );
    }
    println!();
    println!("=== Formation usage histogram ===");
    let total_uses: u32 = p.by_formation.values().map(|s| s.uses).sum();
    for (idx, stats) in &p.by_formation {
        let name = FORMATIONS.get(*idx as usize).map(|f| f.name).unwrap_or("?");
        let share = if total_uses == 0 {
            0.0
        } else {
            stats.uses as f64 / total_uses as f64 * 100.0
        };
        println!("{name:<10} {:>6.1}%  ({} uses)", share, stats.uses);
    }
}

fn parse_seeds_arg(args: impl Iterator<Item = String>) -> u64 {
    const DEFAULT_SEEDS: u64 = 8;
    let args: Vec<String> = args.collect();
    for i in 0..args.len() {
        if args[i] == "--seeds"
            && let Some(v) = args.get(i + 1)
            && let Ok(n) = v.parse::<u64>()
        {
            return n;
        }
    }
    DEFAULT_SEEDS
}

fn main() {
    let num_seeds = parse_seeds_arg(std::env::args().skip(1));
    let seeds: Vec<u64> = (0..num_seeds).collect();
    let cfg = WorldGenConfig::default();

    let report = run_calibration(&seeds, &cfg);
    print_report(&report);
}
