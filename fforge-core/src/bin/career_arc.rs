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
    fit_pa_from_ca_age, print_maturity_ratios, print_report, run_career_arc,
    run_growth_disabled_probe,
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

    let seeds: Vec<u64> = (0..num_seeds as u64).collect();
    let cfg = WorldGenConfig::default();

    let report = run_career_arc(&seeds, seasons, &cfg);
    print_report(&report);

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
    println!();

    print_maturity_ratios(&DevKnobs::default());
}
