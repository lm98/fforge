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

use fforge_core::match_engine::{
    CONSISTENCY_NS, ELO_SCALE_S, FOUL_NS, INJURY_NS, Knobs, PROFILE_SHIFT, SQUAD_PROFILES,
    StreamTelemetry, ai_pick_lineup_vs, lineup_strength, play_match, probe_tactics,
    run_head_to_head, run_head_to_head_detailed, run_squad_conditional_probe,
};
use fforge_core::rng::derive_stream;
use fforge_core::{FIXTURE_STREAM_NS, WorldGenConfig, worldgen};
use fforge_domain::{FORMATIONS, Mentality, Pressing, Tactics, Tempo};

struct CalibReport {
    per_seed_gpm: Vec<f64>,
    pooled: StreamTelemetry,
}

/// Whatever `ai_pick_lineup_vs` actually does. Since T7-R2 flipped
/// `match_engine::AI_TACTICS_ENABLED` to `true`, that means real
/// `ai_pick_tactics` choices on both sides of every fixture — so this
/// harness now pools the tactics-live engine, and the numbers it reports are
/// the ones `TACTICS_MODEL.md` §8's re-bank records.
fn run_calibration(seeds: &[u64], cfg: &WorldGenConfig) -> CalibReport {
    let mut pooled = StreamTelemetry::default();
    let mut per_seed_gpm = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        let (world, schedule, start) = worldgen::generate(seed, cfg);
        let mut seed_goals = 0u32;
        let mut seed_matches = 0u32;

        let suspended = std::collections::BTreeSet::new();
        for fixture in &schedule {
            let home_lineup =
                ai_pick_lineup_vs(&world, fixture.home, fixture.away, true, start, &suspended);
            let away_lineup =
                ai_pick_lineup_vs(&world, fixture.away, fixture.home, false, start, &suspended);
            let home_strength = lineup_strength(&world, &home_lineup);
            let away_strength = lineup_strength(&world, &away_lineup);
            let mut rng = derive_stream(seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
            let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS | fixture.id.0 as u64);
            let mut injury_rng = derive_stream(seed, INJURY_NS | fixture.id.0 as u64);
            let mut foul_rng = derive_stream(seed, FOUL_NS | fixture.id.0 as u64);
            let outcome = play_match(
                &world,
                &home_lineup,
                &away_lineup,
                &mut rng,
                &mut consistency_rng,
                &mut injury_rng,
                &mut foul_rng,
                &Knobs::default(),
                &std::collections::BTreeMap::new(),
                start,
            );

            seed_goals += outcome.home_goals as u32 + outcome.away_goals as u32;
            seed_matches += 1;
            pooled.record(
                &outcome,
                home_lineup.formation,
                away_lineup.formation,
                home_strength,
                away_strength,
            );
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
    println!("fouls/match       : {:.1}", p.fouls_per_match());
    println!(
        "yellows/team/match: {:.2}  (target ~2-3)",
        p.yellows_per_team_per_match()
    );
    println!(
        "reds/team/match   : {:.3}  (target well under 0.1)",
        p.reds_per_team_per_match()
    );
    println!();
    println!("=== Expected points vs strength gap (bookmaker-baseline axis) ===");
    println!(
        "reference: Elo expected-score curve, S = {ELO_SCALE_S:.0} (MATCH_MODEL.md §10 item 6) \
         — a discrimination check, not a fit target; home-advantage level is validated by H/D/A above."
    );
    let deviation = p.score_against_reference(ELO_SCALE_S);
    println!(
        "{:>8} {:>8} {:>10} {:>10} {:>10}",
        "gap", "matches", "empirical", "reference", "deviation"
    );
    for bin in &deviation.per_bin {
        println!(
            "{:>8.1} {:>8} {:>10.3} {:>10.3} {:>+10.3}",
            bin.gap, bin.matches, bin.empirical, bin.reference, bin.deviation
        );
    }
    println!(
        "max |deviation| : {:.3}   match-weighted mean |deviation| : {:.3}",
        deviation.max_abs_deviation, deviation.weighted_mean_abs_deviation
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

fn parse_u64_arg(args: &[String], flag: &str, default: u64) -> u64 {
    for i in 0..args.len() {
        if args[i] == flag
            && let Some(v) = args.get(i + 1)
            && let Ok(n) = v.parse::<u64>()
        {
            return n;
        }
    }
    default
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

/// `TACTICS_MODEL.md` §7's head-to-head mode: the v1 AI never counter-picks
/// (§7's opponent-blindness), so the §5 triangle is never exercised in
/// ordinary league play (`run_calibration`, above) — only forcing both
/// sides' tactics directly, on an equal-strength squad pooled over many
/// seeds, can test it.
fn run_head_to_head_report(num_seeds: u64) {
    let cfg = WorldGenConfig {
        num_clubs: 2,
        ..Default::default()
    };
    let (world, _schedule, _start) = worldgen::generate(7, &cfg);
    let club = world.competition.clubs[0];
    let seeds: Vec<u64> = (0..num_seeds).collect();

    let high = Tactics {
        pressing: Pressing::High,
        ..Tactics::neutral()
    };
    let patient = Tactics {
        tempo: Tempo::Patient,
        ..Tactics::neutral()
    };
    let direct = Tactics {
        tempo: Tempo::Direct,
        ..Tactics::neutral()
    };
    let attacking = Tactics {
        mentality: Mentality::Attacking,
        ..Tactics::neutral()
    };
    let defensive_direct = Tactics {
        mentality: Mentality::Defensive,
        tempo: Tempo::Direct,
        ..Tactics::neutral()
    };

    println!("=== Head-to-head (equal-strength squad, {num_seeds} seeds x2) ===");
    println!("TACTICS_MODEL.md §5's triangle — jointly cyclic if the model is sound:");
    let high_vs_patient = run_head_to_head(&world, club, high, patient, &seeds);
    println!(
        "  High press vs Patient    : {:.3} / {:.3}",
        high_vs_patient,
        1.0 - high_vs_patient
    );
    let direct_vs_high = run_head_to_head(&world, club, direct, high, &seeds);
    println!(
        "  Direct vs High press     : {:.3} / {:.3}",
        direct_vs_high,
        1.0 - direct_vs_high
    );
    let patient_vs_direct = run_head_to_head(&world, club, patient, direct, &seeds);
    println!(
        "  Patient vs Direct        : {:.3} / {:.3}",
        patient_vs_direct,
        1.0 - patient_vs_direct
    );
    println!();
    println!("Mentality (off-triangle risk axis):");
    let attacking_vs_neutral =
        run_head_to_head(&world, club, attacking, Tactics::neutral(), &seeds);
    println!(
        "  Attacking vs Balanced    : {:.3} / {:.3}",
        attacking_vs_neutral,
        1.0 - attacking_vs_neutral
    );
    let counter_vs_attacking = run_head_to_head(&world, club, defensive_direct, attacking, &seeds);
    println!(
        "  Defensive+Direct vs Attacking : {:.3} / {:.3}",
        counter_vs_attacking,
        1.0 - counter_vs_attacking
    );
}

/// `TACTICS_MODEL.md` §9 item 6's probe: the same forced-tactics head-to-head
/// as above, but swept across *squad shapes* rather than only across tactic
/// pairs. The question it answers: is non-dominance cyclic (§5's original
/// claim, unconfirmed) or squad-conditional (the T7 addendum's alternative)?
/// Squad-conditional non-dominance holds iff the best tactic **rotates**
/// across profiles — no single tactic best for every squad shape.
fn run_squad_conditional_report(num_seeds: u64, num_worlds: u64) {
    let cfg = WorldGenConfig {
        num_clubs: 2,
        ..Default::default()
    };
    let worlds: Vec<_> = (0..num_worlds)
        .map(|w| {
            let (world, _schedule, _start) = worldgen::generate(w * 7 + 7, &cfg);
            let club = world.competition.clubs[0];
            (world, club)
        })
        .collect();
    let seeds: Vec<u64> = (0..num_seeds).collect();

    println!(
        "=== Squad-conditional probe ({num_worlds} worlds x {num_seeds} seeds x2 per cell, shift ±{PROFILE_SHIFT}) ==="
    );
    println!("TACTICS_MODEL.md §9 item 6. Each cell: that tactic's expected-points share against");
    println!(
        "Tactics::neutral(), both sides fielding the same profiled squad (equal strength, both"
    );
    println!("legs), pooled across worlds — sd is the spread *across worlds*, the axis a");
    println!("single-world read hides.");
    println!();

    let rows = run_squad_conditional_probe(&worlds, &seeds, &SQUAD_PROFILES);
    let labels: Vec<&str> = probe_tactics().iter().map(|&(n, _)| n).collect();

    println!("--- vs neutral (>0.500 = the tactic helps this squad) ---");
    print!("{:<12}", "profile");
    for l in &labels {
        print!("{l:>20}");
    }
    println!("{:>14}", "pooled best");
    for row in &rows {
        print!("{:<12}", row.profile);
        for &(_, v, sd) in &row.vs_neutral {
            print!("{:>20}", format!("{v:.4} ±{sd:.4}"));
        }
        println!("{:>14}", row.best_tactic());
    }

    println!();
    println!("--- per-world argmax (rotation is only real if it survives the world draw) ---");
    for row in &rows {
        println!("{:<12} {}", row.profile, row.per_world_best.join(", "));
    }

    println!();
    println!("--- the §5 triangle, per squad (>0.500 = first-named side wins the edge) ---");
    println!(
        "{:<12}{:>18}{:>18}{:>18}{:>10}",
        "profile", "High v Patient", "Direct v High", "Patient v Direct", "cyclic?"
    );
    for row in &rows {
        println!(
            "{:<12}{:>18.4}{:>18.4}{:>18.4}{:>10}",
            row.profile,
            row.triangle[0],
            row.triangle[1],
            row.triangle[2],
            if row.triangle.iter().all(|&e| e > 0.5) {
                "yes"
            } else {
                "no"
            }
        );
    }

    println!();
    let bests: std::collections::BTreeSet<&str> = rows.iter().map(|r| r.best_tactic()).collect();
    println!(
        "Distinct pooled best-tactics across {} profiles: {} ({})",
        rows.len(),
        bests.len(),
        bests.iter().cloned().collect::<Vec<_>>().join(", ")
    );

    // The gradient §9 item 6 actually turns on, stated directly: Pressing's
    // value is mechanically increasing in squad Stamina (`contest::fatigue_mult`
    // scales its drop by `(1 - stamina)`, while `def_bias_by_zone` is
    // attribute-independent), so `physical` minus `technical` on the press
    // column is the one cell-difference the model *predicts the sign of*.
    let press_of = |name: &str| -> (f64, f64) {
        let r = rows
            .iter()
            .find(|r| r.profile == name)
            .expect("profile row");
        let c = r.vs_neutral.iter().find(|c| c.0 == "High press").unwrap();
        (c.1, c.2)
    };
    let (phys, phys_sd) = press_of("physical");
    let (tech, tech_sd) = press_of("technical");
    println!(
        "Press gradient (physical - technical): {:+.4}  (world sds {phys_sd:.4} / {tech_sd:.4})",
        phys - tech
    );
}

/// `TACTICS_MODEL.md` §9 item 7: the Mentality axis, pooled across worlds.
///
/// Mentality is §5's declared *risk* axis, deliberately off the triangle, so
/// it is judged on two properties at once and a harness that reads only one
/// of them cannot tell a balanced axis from a broken one:
///
/// - **expected points ≈ 0.500** against `Balanced` — a risk setting that
///   also wins is not a risk setting, it is a better setting.
/// - **goals/match moves** — `Attacking` up, `Defensive` down (§8 predicts
///   ±0.2–0.4), *for both sides*, which is §5's own stated intent: committing
///   men forward should open the game up, not just improve your own odds.
fn run_mentality_report(num_seeds: u64, num_worlds: u64) {
    let cfg = WorldGenConfig {
        num_clubs: 2,
        ..Default::default()
    };
    let worlds: Vec<_> = (0..num_worlds)
        .map(|w| {
            let (world, _s, _d) = worldgen::generate(w * 7 + 7, &cfg);
            let club = world.competition.clubs[0];
            (world, club)
        })
        .collect();
    let seeds: Vec<u64> = (0..num_seeds).collect();

    let attacking = Tactics {
        mentality: Mentality::Attacking,
        ..Tactics::neutral()
    };
    let defensive = Tactics {
        mentality: Mentality::Defensive,
        ..Tactics::neutral()
    };
    let defensive_direct = Tactics {
        mentality: Mentality::Defensive,
        tempo: Tempo::Direct,
        ..Tactics::neutral()
    };
    let balanced = Tactics::neutral();

    let matchups: [(&str, Tactics, Tactics); 5] = [
        ("Attacking v Balanced", attacking, balanced),
        ("Defensive v Balanced", defensive, balanced),
        ("Def+Direct v Attacking", defensive_direct, attacking),
        ("both Attacking", attacking, attacking),
        ("both Defensive", defensive, defensive),
    ];

    println!("=== Mentality probe ({num_worlds} worlds x {num_seeds} seeds x2 per cell) ===");
    println!("TACTICS_MODEL.md §9 item 7. Points ≈ 0.500 means the axis costs what it buys;");
    println!("goals/match is what the axis is *for* (§8: Attacking +0.2-0.4, Defensive -0.2-0.4).");
    println!();
    println!(
        "{:<26}{:>16}{:>10}{:>18}",
        "matchup", "points (first)", "sd", "goals/match"
    );

    for (label, a, b) in matchups {
        let mut pts = Vec::new();
        let mut goals = Vec::new();
        for (world, club) in &worlds {
            let (p, g) = run_head_to_head_detailed(world, *club, a, b, &seeds);
            pts.push(p);
            goals.push(g);
        }
        let pm = pts.iter().sum::<f64>() / pts.len() as f64;
        let psd = (pts.iter().map(|x| (x - pm).powi(2)).sum::<f64>()
            / (pts.len() as f64 - 1.0).max(1.0))
        .sqrt();
        let gm = goals.iter().sum::<f64>() / goals.len() as f64;
        println!("{label:<26}{pm:>16.4}{psd:>10.4}{gm:>18.3}");
    }

    let mut both_balanced = Vec::new();
    for (world, club) in &worlds {
        both_balanced.push(run_head_to_head_detailed(world, *club, balanced, balanced, &seeds).1);
    }
    let bb = both_balanced.iter().sum::<f64>() / both_balanced.len() as f64;
    println!(
        "{:<26}{:>16}{:>10}{bb:>18.3}",
        "both Balanced", "0.5000", "-"
    );
    println!();
    println!("Reference: both-Balanced goals/match = {bb:.3}. §8 wants both-Attacking clearly");
    println!("above it and both-Defensive clearly below, with every points column near 0.500.");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--mentality") {
        let num_worlds = parse_u64_arg(&args, "--worlds", 6);
        let num_seeds = parse_seeds_arg(args.into_iter());
        run_mentality_report(num_seeds.max(50), num_worlds.max(1));
        return;
    }
    if args.iter().any(|a| a == "--squad-conditional") {
        let num_worlds = parse_u64_arg(&args, "--worlds", 8);
        let num_seeds = parse_seeds_arg(args.into_iter());
        run_squad_conditional_report(num_seeds.max(50), num_worlds.max(1));
        return;
    }
    if args.iter().any(|a| a == "--head-to-head") {
        let num_seeds = parse_seeds_arg(args.into_iter());
        run_head_to_head_report(num_seeds.max(50));
        return;
    }

    let num_seeds = parse_seeds_arg(args.into_iter());
    let seeds: Vec<u64> = (0..num_seeds).collect();
    let cfg = WorldGenConfig::default();

    let report = run_calibration(&seeds, &cfg);
    print_report(&report);
}
