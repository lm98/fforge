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
    CONSISTENCY_NS, ELO_SCALE_S, FOUL_NS, INJURY_NS, Knobs, MatchEventKind, PROFILE_SHIFT,
    SQUAD_PROFILES, ShotOutcome, StreamTelemetry, ai_pick_lineup_vs, lineup_strength, play_match,
    probe_tactics, run_head_to_head, run_head_to_head_detailed, run_squad_conditional_probe,
};
use fforge_core::rng::derive_stream;
use fforge_core::{FIXTURE_STREAM_NS, WorldGenConfig, worldgen};
use fforge_domain::{FORMATIONS, Mentality, Pressing, Tactics, Tempo};

struct CalibReport {
    per_seed: Vec<SeedReading>,
    pooled: StreamTelemetry,
}

/// One seed's derived readings — the existing pooled aggregates re-read per
/// seed (not just pooled), plus S1b's substitution-policy measurement
/// (`MATCH_MODEL.md` §16's prediction block). BACKLOG.md §7 item 5: "pool
/// over seeds and report per-seed spread, never just the pooled mean."
#[derive(Debug, Clone, Copy)]
struct SeedReading {
    gpm: f64,
    home_win_rate: f64,
    draw_rate: f64,
    away_win_rate: f64,
    fouls_per_match: f64,
    yellows_per_team_per_match: f64,
    reds_per_team_per_match: f64,
    /// S1b: substitutions per match, pooled both sides.
    subs_per_match: f64,
    /// S1b: share of goals scored at minute 75 or later.
    late_goal_share: f64,
    /// S1b: mean minutes played by a squad member who was *not* in the
    /// starting XI (i.e. entered as a substitute) — `DEVELOPMENT_MODEL.md`
    /// §3's minutes-share signal actually reaching non-XI players in
    /// AI-vs-AI play, previously always exactly zero appearances.
    non_xi_mean_minutes: f64,
}

/// Whatever `ai_pick_lineup_vs` actually does. Since T7-R2 flipped
/// `match_engine::AI_TACTICS_ENABLED` to `true`, that means real
/// `ai_pick_tactics` choices on both sides of every fixture — so this
/// harness now pools the tactics-live engine, and the numbers it reports are
/// the ones `TACTICS_MODEL.md` §8's re-bank records. Since S1b,
/// `ai_pick_lineup_vs` also fills a real bench/plan (`MATCH_MODEL.md` §16),
/// so every AI-controlled match here now substitutes too.
fn run_calibration(seeds: &[u64], cfg: &WorldGenConfig) -> CalibReport {
    let mut pooled = StreamTelemetry::default();
    let mut per_seed = Vec::with_capacity(seeds.len());

    for &seed in seeds {
        let (world, schedule, start) = worldgen::generate(seed, cfg);
        let mut seed_goals = 0u32;
        let mut seed_matches = 0u32;
        let mut seed_tel = StreamTelemetry::default();
        let mut seed_subs = 0u32;
        let mut seed_goals_75_plus = 0u32;
        let mut seed_non_xi_minutes_sum = 0u64;
        let mut seed_non_xi_apps = 0u32;

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
            seed_tel.record(
                &outcome,
                home_lineup.formation,
                away_lineup.formation,
                home_strength,
                away_strength,
            );

            for event in &outcome.stream {
                match event.kind {
                    MatchEventKind::Substitution { .. } => seed_subs += 1,
                    MatchEventKind::Shot {
                        outcome: ShotOutcome::Goal,
                        ..
                    } if event.minute >= 75 => seed_goals_75_plus += 1,
                    _ => {}
                }
            }
            let dressed_xi: std::collections::BTreeSet<_> = home_lineup
                .players
                .iter()
                .chain(away_lineup.players.iter())
                .copied()
                .collect();
            for &(pid, mins) in &outcome.minutes {
                if !dressed_xi.contains(&pid) {
                    seed_non_xi_minutes_sum += mins as u64;
                    seed_non_xi_apps += 1;
                }
            }
        }

        per_seed.push(SeedReading {
            gpm: seed_goals as f64 / seed_matches as f64,
            home_win_rate: seed_tel.home_win_rate(),
            draw_rate: seed_tel.draw_rate(),
            away_win_rate: seed_tel.away_win_rate(),
            fouls_per_match: seed_tel.fouls_per_match(),
            yellows_per_team_per_match: seed_tel.yellows_per_team_per_match(),
            reds_per_team_per_match: seed_tel.reds_per_team_per_match(),
            subs_per_match: seed_subs as f64 / seed_matches as f64,
            late_goal_share: if seed_goals == 0 {
                0.0
            } else {
                seed_goals_75_plus as f64 / seed_goals as f64
            },
            non_xi_mean_minutes: if seed_non_xi_apps == 0 {
                0.0
            } else {
                seed_non_xi_minutes_sum as f64 / seed_non_xi_apps as f64
            },
        });
    }

    CalibReport { per_seed, pooled }
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

/// Mean/sd/min/max of `f` across every seed's reading — the per-seed spread
/// BACKLOG.md §7 item 5 asks every pooled figure to carry.
fn spread(readings: &[SeedReading], f: impl Fn(&SeedReading) -> f64) -> (f64, f64, f64, f64) {
    let vals: Vec<f64> = readings.iter().map(f).collect();
    let m = mean(&vals);
    let sd = stdev(&vals, m);
    let lo = vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    (m, sd, lo, hi)
}

fn print_report(report: &CalibReport) {
    let p = &report.pooled;
    let (gpm_mean, gpm_sd, gpm_min, gpm_max) = spread(&report.per_seed, |s| s.gpm);

    println!(
        "=== Calibration report ({} seeds pooled, {} matches) ===",
        report.per_seed.len(),
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
    println!(
        "=== S1b: substitution-policy measurement (`MATCH_MODEL.md` §16 prediction block, {} seeds) ===",
        report.per_seed.len()
    );
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.subs_per_match);
    println!(
        "subs/match (pooled)  : {m:.2}  (sd {sd:.2}, range {lo:.2}-{hi:.2})  [predicted +3 to +5/match]"
    );
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.late_goal_share * 100.0);
    println!(
        "late goals (75'+)    : {m:.1}%  (sd {sd:.1}, range {lo:.1}-{hi:.1})  [predicted +1 to +3 pts]"
    );
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.non_xi_mean_minutes);
    println!(
        "non-XI mean minutes  : {m:.1}  (sd {sd:.1}, range {lo:.1}-{hi:.1})  [predicted ~10-20]"
    );
    let (hm, hsd, hlo, hhi) = spread(&report.per_seed, |s| s.home_win_rate * 100.0);
    let (dm, dsd, dlo, dhi) = spread(&report.per_seed, |s| s.draw_rate * 100.0);
    let (am, asd, alo, ahi) = spread(&report.per_seed, |s| s.away_win_rate * 100.0);
    println!(
        "H/D/A per-seed spread: {hm:.1}%(sd{hsd:.1},{hlo:.1}-{hhi:.1}) / \
         {dm:.1}%(sd{dsd:.1},{dlo:.1}-{dhi:.1}) / {am:.1}%(sd{asd:.1},{alo:.1}-{ahi:.1})"
    );
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.fouls_per_match);
    println!("fouls/match spread   : {m:.1}  (sd {sd:.1}, range {lo:.1}-{hi:.1})");
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.yellows_per_team_per_match);
    println!("yellows/team spread  : {m:.2}  (sd {sd:.2}, range {lo:.2}-{hi:.2})");
    let (m, sd, lo, hi) = spread(&report.per_seed, |s| s.reds_per_team_per_match);
    println!("reds/team spread     : {m:.3}  (sd {sd:.3}, range {lo:.3}-{hi:.3})");
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

/// S1b diagnostic: isolates whether a pooled-aggregate movement comes from
/// substitutions actually firing, or merely from a populated bench
/// consuming extra Consistency/ambient-injury draws in `build_bench`
/// (`MATCH_MODEL.md` §12/§16: an empty bench draws zero, so this stream
/// consumption is new) — a shift that can reach a match's outcome even when
/// no substitution in it ever fires, the same "symmetric perturbation ≠
/// preserved mean" shape `MATCH_MODEL.md` §17 already diagnosed for
/// Consistency's own landing. Three configurations, same fixtures/XIs, each
/// played with its own independently-derived streams: (A) identity — bench
/// and `sub_plan` both cleared, reproducing the pre-S1b engine exactly; (B)
/// bench populated, `sub_plan` cleared — the extra draws fire, but no
/// substitution ever can; (C) the real policy, unmodified.
fn run_bench_isolation_report(num_seeds: u64) {
    let seeds: Vec<u64> = (0..num_seeds).collect();
    let cfg = WorldGenConfig::default();

    let mut identity_pooled = StreamTelemetry::default();
    let (mut identity_goals, mut identity_matches) = (0u64, 0u64);
    let mut bench_only_pooled = StreamTelemetry::default();
    let (mut bench_only_goals, mut bench_only_matches) = (0u64, 0u64);
    let mut full_pooled = StreamTelemetry::default();
    let (mut full_goals, mut full_matches) = (0u64, 0u64);

    for &seed in &seeds {
        let (world, schedule, start) = worldgen::generate(seed, &cfg);
        let suspended = std::collections::BTreeSet::new();
        for fixture in &schedule {
            let home_full =
                ai_pick_lineup_vs(&world, fixture.home, fixture.away, true, start, &suspended);
            let away_full =
                ai_pick_lineup_vs(&world, fixture.away, fixture.home, false, start, &suspended);
            let home_strength = lineup_strength(&world, &home_full);
            let away_strength = lineup_strength(&world, &away_full);

            let mut home_identity = home_full.clone();
            home_identity.bench.clear();
            home_identity.sub_plan.clear();
            let mut away_identity = away_full.clone();
            away_identity.bench.clear();
            away_identity.sub_plan.clear();

            let mut home_bench_only = home_full.clone();
            home_bench_only.sub_plan.clear();
            let mut away_bench_only = away_full.clone();
            away_bench_only.sub_plan.clear();

            for (home, away, goals_acc, matches_acc, pooled_acc) in [
                (
                    &home_identity,
                    &away_identity,
                    &mut identity_goals,
                    &mut identity_matches,
                    &mut identity_pooled,
                ),
                (
                    &home_bench_only,
                    &away_bench_only,
                    &mut bench_only_goals,
                    &mut bench_only_matches,
                    &mut bench_only_pooled,
                ),
                (
                    &home_full,
                    &away_full,
                    &mut full_goals,
                    &mut full_matches,
                    &mut full_pooled,
                ),
            ] {
                let mut rng = derive_stream(seed, FIXTURE_STREAM_NS | fixture.id.0 as u64);
                let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS | fixture.id.0 as u64);
                let mut injury_rng = derive_stream(seed, INJURY_NS | fixture.id.0 as u64);
                let mut foul_rng = derive_stream(seed, FOUL_NS | fixture.id.0 as u64);
                let outcome = play_match(
                    &world,
                    home,
                    away,
                    &mut rng,
                    &mut consistency_rng,
                    &mut injury_rng,
                    &mut foul_rng,
                    &Knobs::default(),
                    &std::collections::BTreeMap::new(),
                    start,
                );
                *goals_acc += outcome.home_goals as u64 + outcome.away_goals as u64;
                *matches_acc += 1;
                pooled_acc.record(
                    &outcome,
                    home.formation,
                    away.formation,
                    home_strength,
                    away_strength,
                );
            }
        }
    }

    println!("=== S1b bench-isolation diagnostic ({num_seeds} seeds) ===");
    println!(
        "Same fixtures/XIs, three lineup configurations, each played with its own \
         independently-derived streams."
    );
    println!();
    for (label, goals, matches, pooled) in [
        (
            "A) identity (empty bench+plan)   ",
            identity_goals,
            identity_matches,
            &identity_pooled,
        ),
        (
            "B) bench populated, plan empty   ",
            bench_only_goals,
            bench_only_matches,
            &bench_only_pooled,
        ),
        (
            "C) full policy (bench+plan, live)",
            full_goals,
            full_matches,
            &full_pooled,
        ),
    ] {
        println!(
            "{label}: gpm {:.3}  H/D/A {:5.1}/{:4.1}/{:4.1}%  fouls/match {:5.1}  \
             yellows/team {:.2}  reds/team {:.3}",
            goals as f64 / matches as f64,
            pooled.home_win_rate() * 100.0,
            pooled.draw_rate() * 100.0,
            pooled.away_win_rate() * 100.0,
            pooled.fouls_per_match(),
            pooled.yellows_per_team_per_match(),
            pooled.reds_per_team_per_match(),
        );
    }
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
    if args.iter().any(|a| a == "--bench-isolation") {
        let num_seeds = parse_seeds_arg(args.into_iter());
        run_bench_isolation_report(num_seeds);
        return;
    }

    let num_seeds = parse_seeds_arg(args.into_iter());
    let seeds: Vec<u64> = (0..num_seeds).collect();
    let cfg = WorldGenConfig::default();

    let report = run_calibration(&seeds, &cfg);
    print_report(&report);
}
