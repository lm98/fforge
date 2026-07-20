//! Market runner (`TRANSFER_MODEL.md` §11): the transfer-market sibling of
//! `bin/calibrate.rs` and `bin/career_arc.rs`. Drives the real worldgen +
//! full command pipeline (matches, development, finance, pool, market
//! clearing) pooled over many world seeds, each traced across ~15 seasons,
//! and reports the §11 metric table against its believable bands — with
//! per-seed spread, not just the pooled mean.
//!
//! Run with: `cargo run --release --bin market -- --seeds 8 --seasons 15`

use fforge_core::WorldGenConfig;
use fforge_core::market::{print_report, run_market_calibration};

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
    let seasons = parse_usize_arg(&args, "--seasons", 15);

    let seeds: Vec<u64> = (0..num_seeds as u64).collect();
    let cfg = WorldGenConfig::default();

    let report = run_market_calibration(&seeds, seasons, &cfg);
    print_report(&report);
}
