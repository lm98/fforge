//! Career-arc runner (`DEVELOPMENT_MODEL.md` §6): the development sibling of
//! `bin/calibrate.rs`. Drives the real worldgen + development-fold pipeline
//! pooled over many world seeds, each across a decade-plus, and reports the §6
//! career-arc metrics (peak ages, PA attainment + tail, veteran decline slopes,
//! wonderkid hit/flop rates) against their targets — with per-seed spread, the
//! `MATCH_MODEL.md` §8 noisy-estimator readout, not just the pooled mean.
//!
//! Run with: `cargo run --release --bin career_arc -- --seeds 8 --seasons 16`

use fforge_core::WorldGenConfig;
use fforge_core::career_arc::{print_report, run_career_arc};

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
}
