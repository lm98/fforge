//! Career-arc runner (`DEVELOPMENT_MODEL.md` §6): the development sibling of
//! `bin/calibrate.rs`. Drives the real worldgen + development-fold pipeline
//! pooled over many world seeds, each across a decade-plus, and reports the §6
//! career-arc metrics (peak ages, PA attainment + tail, veteran decline slopes,
//! wonderkid hit/flop rates) against their targets — with per-seed spread, the
//! `MATCH_MODEL.md` §8 noisy-estimator readout, not just the pooled mean.
//!
//! Run with: `cargo run --release --bin career_arc -- --seeds 8 --seasons 16`

use fforge_core::DevKnobs;
use fforge_core::WorldGenConfig;
use fforge_core::career_arc::{
    fit_pa_from_ca_age, fit_pa_from_ca_age_band, fit_pa_from_ca_age_youth,
    fit_pa_from_composites_age, fit_pa_from_composites_age_band, fit_pa_from_composites_age_youth,
    max_step_saturation_16_band, print_maturity_ratios, print_report, print_seeding_projection,
    run_career_arc_with_projection, run_growth_disabled_probe,
};

fn parse_usize_arg(args: &[String], flag: &str, default: usize) -> usize {
    for i in 0..args.len() {
        if args[i] == flag
            && let Some(v) = args.get(i + 1)
            && let Ok(n) = v.parse::<usize>()
        {
            return n;
        }
    }
    default
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num_seeds = parse_usize_arg(&args, "--seeds", 8);
    let seasons = parse_usize_arg(&args, "--seasons", 16);

    if args.iter().any(|a| a == "--max-step-saturation") {
        let cfg = WorldGenConfig::default();
        let seeds: Vec<u64> = (0..num_seeds as u64).collect();
        let (attempted, clipped, frac) = max_step_saturation_16_band(&seeds, seasons, &cfg);
        println!(
            "max_step clip fraction, [16,17) start-age cohort: {clipped}/{attempted} = {frac:.4} \
             ({num_seeds} seeds x {seasons} seasons, DevKnobs::default() as currently compiled)"
        );
        return;
    }

    let seeds: Vec<u64> = (0..num_seeds as u64).collect();
    let cfg = WorldGenConfig::default();

    let (report, projection) = run_career_arc_with_projection(&seeds, seasons, &cfg);
    print_report(&report);

    println!();
    print_seeding_projection(&projection);

    // --- Wonderkid-flop-analysis (measurement-only) ---
    println!();
    println!("=== Wonderkid-flop-analysis probes ===");
    println!();
    println!("--- Task 2: growth-disabled probe (k=0, e_base/e_min at floor) ---");
    // Larger pool than the normal report: this probe skips match simulation
    // entirely (see `trace_seed_growth_disabled`'s doc comment), so it is
    // cheap, and the flop rate is a rare-event count that needs a wider pool
    // to read reliably (`MATCH_MODEL.md` §8's noisy-estimator lesson).
    let probe_seeds: Vec<u64> = (0..64u64).collect();
    let disabled = run_growth_disabled_probe(&probe_seeds, seasons as f64, &cfg);
    let flop = disabled.wonderkid_flop();
    println!(
        "Flop rate (attainment < 0.75)   : {:.4}  (sd {:.4}, range {:.4}-{:.4}, {} seeds)  target if hypothesis holds: still 0.00",
        flop.mean, flop.sd, flop.min, flop.max, flop.n
    );
    println!(
        "Attainment mean                 : {:.4}  (target: approx r0)",
        disabled.attainment_mean().mean
    );
    let r0 = disabled.r0_wonderkid();
    println!(
        "r0 (this probe)                 : mean {:.3}, sd {:.3}, min {:.3}, p10 {:.3} ({} arcs)",
        r0.mean, r0.sd, r0.min, r0.p10, r0.n
    );
    let gap = disabled.attainment_minus_r0_wonderkid();
    println!(
        "attainment - r0 (this probe)    : mean {:.4}, sd {:.4}, min {:.4}, p10 {:.4} ({} arcs)",
        gap.mean, gap.sd, gap.min, gap.p10, gap.n
    );
    println!();

    println!("--- Task 3: PA ~ a*CA + b*age + c (all worldgen players, pooled) ---");
    let fit = fit_pa_from_ca_age(&seeds, &cfg);
    println!(
        "a={:.4}  b={:.4}  c={:.4}  residual_sd={:.4}  n={}  (target residual_sd approx 2.3)",
        fit.a, fit.b, fit.c, fit.residual_sd, fit.n
    );
    // W1b amendment §4: the same fit restricted to age < 24, isolating the
    // youth band from the headroom formula's kink at 24 (see
    // `fit_pa_from_ca_age_youth`'s doc comment).
    let fit_youth = fit_pa_from_ca_age_youth(&seeds, &cfg);
    println!(
        "youth-only (age<24): a={:.4}  b={:.4}  c={:.4}  residual_sd={:.4}  n={}  (predicted approx 2.31 = 8/sqrt(12))",
        fit_youth.a, fit_youth.b, fit_youth.c, fit_youth.residual_sd, fit_youth.n
    );
    println!();

    print_maturity_ratios(&DevKnobs::default());
    println!();

    // --- DEVELOPMENT_MODEL.md §8: post-seeding-invert measurement ---
    println!("=== §8 seeding-fix measurement: NAIVE vs COMPETENT PA recoverability ===");
    println!();
    println!(
        "{:>6} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "band", "naive_sd", "n", "compt_sd", "n", "gap"
    );
    for band in 16..=21i64 {
        let (lo, hi) = (band as f64, band as f64 + 1.0);
        let naive = fit_pa_from_ca_age_band(&seeds, &cfg, lo, hi);
        let compt = fit_pa_from_composites_age_band(&seeds, &cfg, lo, hi);
        println!(
            "{:>6} {:>8.3} {:>8} {:>8.3} {:>8} {:>8.3}",
            band,
            naive.residual_sd,
            naive.n,
            compt.residual_sd,
            compt.n,
            naive.residual_sd - compt.residual_sd
        );
    }
    println!();
    let naive_youth = fit_pa_from_ca_age_youth(&seeds, &cfg);
    let compt_youth = fit_pa_from_composites_age_youth(&seeds, &cfg);
    println!(
        "youth (age<24) pooled: naive_sd={:.3} (n={})  competent_sd={:.3} (n={})  gap={:.3}",
        naive_youth.residual_sd,
        naive_youth.n,
        compt_youth.residual_sd,
        compt_youth.n,
        naive_youth.residual_sd - compt_youth.residual_sd
    );
    let naive_all = fit_pa_from_ca_age(&seeds, &cfg);
    let compt_all = fit_pa_from_composites_age(&seeds, &cfg);
    println!(
        "all ages pooled:       naive_sd={:.3} (n={})  competent_sd={:.3} (n={})  gap={:.3}",
        naive_all.residual_sd, naive_all.n, compt_all.residual_sd, compt_all.n,
        naive_all.residual_sd - compt_all.residual_sd
    );
    println!();

    println!("--- §8.3 headline: start_age<=18 wonderkid hit/flop, real post-fix arcs ---");
    for b in projection.bands() {
        if b.start_age_band <= 18 {
            println!(
                "band {:>3}: n={:>4} n_wk={:>4}  attainment {:.3}  sub80 {:.3}  hit {:.3}  flop {:.3}",
                b.start_age_band, b.n, b.n_wonderkid, b.attainment_mean, b.sub80_frac, b.hit_rate,
                b.flop_rate
            );
        }
    }
    let le18 = projection.le18();
    println!(
        "<=18 pooled: n={} n_wk={}  attainment {:.3}  sub80 {:.3}  hit {:.3}  flop {:.3}  (target flop 0.02-0.08, hit >=0.45)",
        le18.n, le18.n_wonderkid, le18.attainment_mean, le18.sub80_frac, le18.hit_rate,
        le18.flop_rate
    );
    let overall = projection.overall();
    println!(
        "<=21 pooled (attainment mean/sub80 tail stay on this cohort per §8.3): n={} n_wk={}  attainment {:.3} (target 0.85-0.92)  sub80 {:.3} (target 0.10-0.20)",
        overall.n, overall.n_wonderkid, overall.attainment_mean, overall.sub80_frac
    );
    println!();
}
