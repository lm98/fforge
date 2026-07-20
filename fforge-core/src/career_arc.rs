//! The career-arc harness (`DEVELOPMENT_MODEL.md` §6): the development sibling
//! of `match_engine::calibrate`. Where `calibrate` drives the real worldgen +
//! match pipeline and reports emergent *match* aggregates, this drives the real
//! worldgen + development fold across a **decade-plus, pooled over many world
//! seeds**, and reports the emergent *career* statistics §6 lists — peak age per
//! `DevCategory`, PA-attainment (mean + the sub-0.80 tail), veteran decline
//! slopes, and wonderkid hit/flop rates — each against its §6 target.
//!
//! **Why it drives a real `Session`, not a synthetic cohort.** Development is
//! *folded* (`DEVELOPMENT_MODEL.md` §5): a full multi-season run — worldgen's
//! real attribute distribution, aged monthly by `Event::DevelopmentTick` through
//! `Command::AdvanceMatchday` / `StartNextSeason`, with real AI lineups driving
//! the playing-time window — is the only faithful input. The scratchpad
//! `dev_shape` fitted its curves on a from-youth synthetic cohort; §6 is explicit
//! that those numbers are "the notebook's fitted point, expected to shift on the
//! real distribution exactly as `b_beat` did for the match engine", so the whole
//! point of this harness is to re-read the metrics off the *real* pipeline.
//!
//! **Why per-seed spread, not just the pooled mean** (`MATCH_MODEL.md` §8): a
//! single synthetic cohort is a noisy estimator. Every metric here is reported as
//! a **mean of per-seed means plus the spread across seeds** (sd + range), so a
//! believable pooled number that is actually a wide smear across seeds is visible
//! as such, not hidden.
//!
//! This module is harness plumbing, not simulation logic — like
//! `match_engine::calibrate` it never feeds back into `DevKnobs` by itself; the
//! re-fit is a human reading these numbers and editing `DevKnobs::default`.

use crate::{Command, Session, WorldGenConfig, new_game};
use fforge_domain::{
    Attribute, ClubId, DevCategory, ROLE_WEIGHTS, Role, World, best_role, date::DAYS_PER_YEAR,
};

// --- observation-window filters (§6 "Peak-age metric note") --------------
//
// A peak/plateau/slope can only be *measured* on a career whose sampled age
// range actually brackets it — a player first seen at 30 has no observable
// physical peak, and folding their monotone decline into the estimate biases it
// late. Each metric therefore admits only arcs whose [min_age, max_age] window
// contains the feature. The bounds are deliberately loose (they gate *whose*
// career is measurable, not what counts as a good number).

/// Physical peak (a genuine argmax, §6): need the rise *and* the fall around the
/// ~25 peak.
const PHYS_PEAK_MIN_AGE: f64 = 22.0;
const PHYS_PEAK_MAX_AGE: f64 = 28.0;
/// Post-peak physical slope is measured over this many years past the peak.
const PHYS_POSTPEAK_SPAN: f64 = 5.0;

/// Technical plateau onset (§6): still climbing at the low end, into the plateau
/// at the high end.
const TECH_ONSET_MIN_AGE: f64 = 24.0;
const TECH_ONSET_MAX_AGE: f64 = 31.0;
/// Mental plateau onset (§6): the latest-maturing category.
const MENT_ONSET_MIN_AGE: f64 = 25.0;
const MENT_ONSET_MAX_AGE: f64 = 33.0;

/// Overall best-role CA peak (mid–late 20s, §6): physical decline pulls it down,
/// so it is a real argmax; still needs the window to bracket it.
const CA_PEAK_MIN_AGE: f64 = 22.0;
const CA_PEAK_MAX_AGE: f64 = 31.0;

/// Fraction of career max at which a flat category is deemed to have "arrived"
/// — the plateau-onset threshold (§6: "first age reaching 98% of its career
/// maximum").
const PLATEAU_FRACTION: f64 = 0.98;

/// Veteran decline slopes (§6) are read as the CA-scale change per year across
/// the 30→35 band, so an arc must span both ends.
const VET_LO_AGE: f64 = 30.0;
const VET_HI_AGE: f64 = 35.0;

// --- development cohort (attainment + wonderkids) ------------------------
//
// PA-attainment and wonderkid outcomes are about *prospects realizing potential*
// — only meaningful for players who start with real headroom (`worldgen` grants
// it below age 24, §worldgen `gen_player`) and whom we then trace through their
// peak. Veterans start at PA≈CA by construction, so their attainment is ~1.0 and
// uninformative; folding them in would wash the tail out.

/// Max world-start age to count as a development prospect.
const COHORT_MAX_START_AGE: f64 = 21.0;
/// Min age a prospect must be traced to, so their peak CA is actually observed.
const COHORT_MIN_END_AGE: f64 = 26.0;

/// PA floor for the wonderkid sub-population (§6: "Wonderkid (PA ≥ 80)").
const WONDERKID_PA: f64 = 80.0;
/// Attainment at/above which a wonderkid "hit"; below which they "flopped" (§6).
const WONDERKID_HIT: f64 = 0.90;
const WONDERKID_FLOP: f64 = 0.75;
/// The PA-attainment underperformance tail §6 tracks.
const ATTAINMENT_TAIL: f64 = 0.80;

/// One sampled point on a player's career: age (years) and the three outfield
/// category composites plus best-role CA at that date. Composites are the
/// role-weighted mean of the category's attributes (the CA aggregation restricted
/// to one `DevCategory`) using the player's `natural_role`, so the attribute set
/// is stable across the arc and each composite reads on the same 0–100 scale as
/// CA. `NaN` marks a category the role weights at zero (never happens for the
/// three outfield categories, but kept honest).
#[derive(Clone, Copy)]
struct Sample {
    age: f64,
    phys: f64,
    tech: f64,
    ment: f64,
    ca: f64,
}

/// A single player's traced career: their (hidden) PA and the time-ordered
/// samples. `natural_role` is fixed, so composite trajectories are stable.
struct Arc {
    pa: f64,
    start_age: f64,
    samples: Vec<Sample>,
}

impl Arc {
    fn min_age(&self) -> f64 {
        self.samples.first().map(|s| s.age).unwrap_or(f64::NAN)
    }
    fn max_age(&self) -> f64 {
        self.samples.last().map(|s| s.age).unwrap_or(f64::NAN)
    }

    /// The sample nearest a target age (careers are sampled densely — weekly in
    /// season — so "nearest" is within a few days of the ask).
    fn nearest(&self, age: f64) -> Option<&Sample> {
        self.samples
            .iter()
            .min_by(|a, b| (a.age - age).abs().total_cmp(&(b.age - age).abs()))
    }

    /// Argmax age of a per-sample field — the raw peak age. Only valid for a
    /// genuinely-declining series (physical, overall CA); a flat plateau drifts
    /// late under argmax, which is why the flat categories use `plateau_onset`.
    fn peak_age(&self, field: impl Fn(&Sample) -> f64) -> f64 {
        self.samples
            .iter()
            .max_by(|a, b| field(a).total_cmp(&field(b)))
            .map(|s| s.age)
            .unwrap_or(f64::NAN)
    }

    /// The first age at which a field reaches `PLATEAU_FRACTION` of its career
    /// maximum (§6 plateau-onset — stable for flat categories where argmax
    /// drifts). Scans ascending age (samples are time-ordered).
    fn plateau_onset(&self, field: impl Fn(&Sample) -> f64) -> f64 {
        let max = self
            .samples
            .iter()
            .map(&field)
            .fold(f64::NEG_INFINITY, f64::max);
        let threshold = PLATEAU_FRACTION * max;
        self.samples
            .iter()
            .find(|s| field(s) >= threshold)
            .map(|s| s.age)
            .unwrap_or(f64::NAN)
    }

    /// Peak (max) best-role CA reached over the whole career.
    fn peak_ca(&self) -> f64 {
        self.samples
            .iter()
            .map(|s| s.ca)
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

/// The role-weighted mean of one `DevCategory`'s attributes — the CA aggregation
/// (`ability::current_ability`) restricted to a single category, using `role`'s
/// weights. `NaN` iff the role weights every attribute in the category at zero
/// (never happens for the three outfield categories, but kept honest).
fn category_composite(role: Role, attrs: &fforge_domain::Attributes, cat: DevCategory) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for attr in Attribute::ALL {
        if attr.dev_category() != cat {
            continue;
        }
        let w = ROLE_WEIGHTS.weight(role, attr) as f64;
        num += w * attrs.get(attr) as f64;
        den += w;
    }
    if den == 0.0 { f64::NAN } else { num / den }
}

/// Snapshot every player of `world` at `date` into the growing per-player arcs.
fn sample_world(world: &World, date: fforge_domain::GameDate, arcs: &mut Vec<(u32, Arc)>) {
    for (idx, (&pid, player)) in world.players.iter().enumerate() {
        let age = (date.days - player.birth.days) as f64 / DAYS_PER_YEAR as f64;
        let role = player.natural_role;
        let sample = Sample {
            age,
            phys: category_composite(role, &player.attributes, DevCategory::Physical),
            tech: category_composite(role, &player.attributes, DevCategory::Technical),
            ment: category_composite(role, &player.attributes, DevCategory::Mental),
            ca: best_role(&player.attributes, &ROLE_WEIGHTS).1 as f64,
        };
        // `world.players` is a stable BTreeMap, so player `idx` is stable across
        // ticks within a seed — we index arcs positionally to avoid a per-sample
        // map lookup.
        if idx == arcs.len() {
            arcs.push((
                pid.0,
                Arc {
                    pa: player.character.potential as f64,
                    start_age: age,
                    samples: Vec::new(),
                },
            ));
        }
        arcs[idx].1.samples.push(sample);
    }
}

/// Trace one world seed across `seasons` full seasons, returning every player's
/// career arc. Drives the *real* command pipeline (worldgen → AI lineups → match
/// engine → monthly development fold), sampling the developed world after every
/// matchday and every season roll-over.
fn trace_seed(seed: u64, seasons: usize, cfg: &WorldGenConfig) -> Vec<Arc> {
    let log = new_game(seed, cfg, ClubId(0));
    let mut session = Session::from_events(log, &mut []);
    let mut arcs: Vec<(u32, Arc)> = Vec::new();

    sample_world(&session.state.world, session.state.date, &mut arcs);
    for s in 0..seasons {
        while !session.state.season_over() {
            session
                .execute(Command::AdvanceMatchday, &mut [])
                .expect("advance matchday");
            sample_world(&session.state.world, session.state.date, &mut arcs);
        }
        if s + 1 < seasons {
            session
                .execute(Command::StartNextSeason, &mut [])
                .expect("start next season");
            sample_world(&session.state.world, session.state.date, &mut arcs);
        }
    }

    arcs.into_iter().map(|(_, a)| a).collect()
}

/// Every §6 metric reduced to one number per seed (a per-seed mean over that
/// seed's qualifying players), plus the pooled raw attainment values for the
/// distribution tail. Per-seed vectors are the raw material for the spread
/// (`SeedSpread`) the report prints.
#[derive(Default)]
pub struct CareerArcReport {
    pub seeds: usize,
    pub seasons: usize,

    // Per-seed means (one entry per seed) — §6 metrics.
    phys_peak_age: Vec<f64>,
    phys_postpeak_slope: Vec<f64>,
    tech_onset_age: Vec<f64>,
    ment_onset_age: Vec<f64>,
    ca_peak_age: Vec<f64>,
    attainment_mean: Vec<f64>,
    attainment_tail_frac: Vec<f64>,
    vet_phys_slope: Vec<f64>,
    vet_ment_slope: Vec<f64>,
    wonderkid_hit: Vec<f64>,
    wonderkid_flop: Vec<f64>,

    // Per-seed qualifying sample sizes (so a tight-looking number backed by a
    // handful of careers is visible as thin).
    n_phys_peak: Vec<usize>,
    n_tech_onset: Vec<usize>,
    n_ment_onset: Vec<usize>,
    n_ca_peak: Vec<usize>,
    n_cohort: Vec<usize>,
    n_vet: Vec<usize>,
    n_wonderkid: Vec<usize>,

    /// Pooled attainment values across all seeds — for the pooled p10 / tail.
    all_attainment: Vec<f64>,
}

/// Mean, sd, and range of a per-seed metric — the `MATCH_MODEL.md` §8
/// noisy-estimator readout.
pub struct SeedSpread {
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
    pub n: usize,
}

fn seed_spread(xs: &[f64]) -> SeedSpread {
    let valid: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    let n = valid.len();
    if n == 0 {
        return SeedSpread {
            mean: f64::NAN,
            sd: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
            n: 0,
        };
    }
    let mean = valid.iter().sum::<f64>() / n as f64;
    let sd = if n < 2 {
        0.0
    } else {
        (valid.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64).sqrt()
    };
    let min = valid.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = valid.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    SeedSpread {
        mean,
        sd,
        min,
        max,
        n,
    }
}

/// Mean of the finite entries of a slice (used for per-seed reduction).
fn mean_finite(xs: &[f64]) -> f64 {
    let valid: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if valid.is_empty() {
        f64::NAN
    } else {
        valid.iter().sum::<f64>() / valid.len() as f64
    }
}

/// The p-quantile (0..1) of a copy-sorted sample, linear on ranks.
fn percentile(xs: &[f64], p: f64) -> f64 {
    let mut v: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let rank = p * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        v[lo]
    } else {
        v[lo] + (rank - lo as f64) * (v[hi] - v[lo])
    }
}

impl CareerArcReport {
    /// Fold one seed's traced arcs into the report as a fresh per-seed row.
    fn record_seed(&mut self, arcs: &[Arc]) {
        // --- peak / plateau ages, each over its own admissible sub-population.
        let mut phys_peaks = Vec::new();
        let mut phys_slopes = Vec::new();
        let mut tech_onsets = Vec::new();
        let mut ment_onsets = Vec::new();
        let mut ca_peaks = Vec::new();
        // --- attainment / wonderkids over the development cohort.
        let mut attainments = Vec::new();
        let mut wk_hits = Vec::new();
        let mut wk_flops = Vec::new();
        // --- veteran 30→35 slopes.
        let mut vet_phys = Vec::new();
        let mut vet_ment = Vec::new();

        for arc in arcs {
            let (lo, hi) = (arc.min_age(), arc.max_age());

            if lo <= PHYS_PEAK_MIN_AGE && hi >= PHYS_PEAK_MAX_AGE {
                let peak = arc.peak_age(|s| s.phys);
                phys_peaks.push(peak);
                if hi >= peak + PHYS_POSTPEAK_SPAN
                    && let (Some(a), Some(b)) =
                        (arc.nearest(peak), arc.nearest(peak + PHYS_POSTPEAK_SPAN))
                {
                    phys_slopes.push((b.phys - a.phys) / (b.age - a.age));
                }
            }
            if lo <= TECH_ONSET_MIN_AGE && hi >= TECH_ONSET_MAX_AGE {
                tech_onsets.push(arc.plateau_onset(|s| s.tech));
            }
            if lo <= MENT_ONSET_MIN_AGE && hi >= MENT_ONSET_MAX_AGE {
                ment_onsets.push(arc.plateau_onset(|s| s.ment));
            }
            if lo <= CA_PEAK_MIN_AGE && hi >= CA_PEAK_MAX_AGE {
                ca_peaks.push(arc.peak_age(|s| s.ca));
            }

            // Veteran decline: slope of the category composite over 30→35.
            if lo <= VET_LO_AGE
                && hi >= VET_HI_AGE
                && let (Some(a), Some(b)) = (arc.nearest(VET_LO_AGE), arc.nearest(VET_HI_AGE))
            {
                let dy = b.age - a.age;
                vet_phys.push((b.phys - a.phys) / dy);
                vet_ment.push((b.ment - a.ment) / dy);
            }

            // Development cohort: a headroom-bearing prospect traced past its peak.
            if arc.start_age <= COHORT_MAX_START_AGE && hi >= COHORT_MIN_END_AGE && arc.pa > 0.0 {
                let attainment = arc.peak_ca() / arc.pa;
                attainments.push(attainment);
                self.all_attainment.push(attainment);
                if arc.pa >= WONDERKID_PA {
                    wk_hits.push(if attainment >= WONDERKID_HIT {
                        1.0
                    } else {
                        0.0
                    });
                    wk_flops.push(if attainment < WONDERKID_FLOP {
                        1.0
                    } else {
                        0.0
                    });
                }
            }
        }

        let tail_frac = if attainments.is_empty() {
            f64::NAN
        } else {
            attainments.iter().filter(|&&a| a < ATTAINMENT_TAIL).count() as f64
                / attainments.len() as f64
        };

        self.phys_peak_age.push(mean_finite(&phys_peaks));
        self.phys_postpeak_slope.push(mean_finite(&phys_slopes));
        self.tech_onset_age.push(mean_finite(&tech_onsets));
        self.ment_onset_age.push(mean_finite(&ment_onsets));
        self.ca_peak_age.push(mean_finite(&ca_peaks));
        self.attainment_mean.push(mean_finite(&attainments));
        self.attainment_tail_frac.push(tail_frac);
        self.vet_phys_slope.push(mean_finite(&vet_phys));
        self.vet_ment_slope.push(mean_finite(&vet_ment));
        self.wonderkid_hit.push(mean_finite(&wk_hits));
        self.wonderkid_flop.push(mean_finite(&wk_flops));

        self.n_phys_peak.push(phys_peaks.len());
        self.n_tech_onset.push(tech_onsets.len());
        self.n_ment_onset.push(ment_onsets.len());
        self.n_ca_peak.push(ca_peaks.len());
        self.n_cohort.push(attainments.len());
        self.n_vet.push(vet_phys.len());
        self.n_wonderkid.push(wk_hits.len());
    }

    // Public accessors for the regression test / bin (spreads over per-seed means).
    pub fn phys_peak_age(&self) -> SeedSpread {
        seed_spread(&self.phys_peak_age)
    }
    pub fn tech_onset_age(&self) -> SeedSpread {
        seed_spread(&self.tech_onset_age)
    }
    pub fn ment_onset_age(&self) -> SeedSpread {
        seed_spread(&self.ment_onset_age)
    }
    pub fn ca_peak_age(&self) -> SeedSpread {
        seed_spread(&self.ca_peak_age)
    }
    pub fn attainment_mean(&self) -> SeedSpread {
        seed_spread(&self.attainment_mean)
    }
    pub fn attainment_tail_frac(&self) -> SeedSpread {
        seed_spread(&self.attainment_tail_frac)
    }
    pub fn vet_phys_slope(&self) -> SeedSpread {
        seed_spread(&self.vet_phys_slope)
    }
    pub fn vet_ment_slope(&self) -> SeedSpread {
        seed_spread(&self.vet_ment_slope)
    }
    pub fn phys_postpeak_slope(&self) -> SeedSpread {
        seed_spread(&self.phys_postpeak_slope)
    }
    pub fn wonderkid_hit(&self) -> SeedSpread {
        seed_spread(&self.wonderkid_hit)
    }
    pub fn wonderkid_flop(&self) -> SeedSpread {
        seed_spread(&self.wonderkid_flop)
    }
    /// Pooled p-quantile of attainment across every seed's prospects.
    pub fn attainment_percentile(&self, p: f64) -> f64 {
        percentile(&self.all_attainment, p)
    }
}

/// Run the career-arc harness over `seeds` world seeds, each traced `seasons`
/// full seasons, and return the pooled §6 report.
pub fn run_career_arc(seeds: &[u64], seasons: usize, cfg: &WorldGenConfig) -> CareerArcReport {
    let mut report = CareerArcReport {
        seeds: seeds.len(),
        seasons,
        ..Default::default()
    };
    for &seed in seeds {
        let arcs = trace_seed(seed, seasons, cfg);
        report.record_seed(&arcs);
    }
    report
}

/// Pretty-print the report to stdout (the `bin/career_arc.rs` payload). Each row
/// is `mean (sd, range across seeds) [n careers] | target`, so the pooled number
/// and its per-seed spread sit side by side with the §6 target it answers to.
pub fn print_report(report: &CareerArcReport) {
    fn row(label: &str, s: &SeedSpread, target: &str) {
        println!(
            "{label:<32}: {:>6.2}  (sd {:>5.2}, range {:>6.2}-{:>6.2}, {} seeds)   target: {target}",
            s.mean, s.sd, s.min, s.max, s.n
        );
    }

    println!(
        "=== Career-arc report ({} seeds pooled, {} seasons each) ===",
        report.seeds, report.seasons
    );
    println!();
    println!("--- Peak age per DevCategory (DEVELOPMENT_MODEL.md §6) ---");
    row("Physical peak age", &report.phys_peak_age(), "24-27");
    row(
        "Physical post-peak slope (CA/yr)",
        &report.phys_postpeak_slope(),
        "clearly negative",
    );
    row(
        "Technical plateau onset age",
        &report.tech_onset_age(),
        "late 20s",
    );
    row(
        "Mental plateau onset age",
        &report.ment_onset_age(),
        "early 30s",
    );
    row(
        "Overall best-role CA peak age",
        &report.ca_peak_age(),
        "mid-late 20s",
    );
    println!();
    println!("--- PA attainment (peak CA / PA) ---");
    row("Attainment mean", &report.attainment_mean(), "0.85-0.92");
    row(
        "Attainment tail (frac < 0.80)",
        &report.attainment_tail_frac(),
        "a real tail (~0.13)",
    );
    println!(
        "{:<32}: {:>6.3}                                              target: ~0.78",
        "Attainment p10 (pooled)",
        report.attainment_percentile(0.10)
    );
    println!();
    println!("--- Veteran decline, 30->35 composite slope (CA/yr) ---");
    row(
        "Physical",
        &report.vet_phys_slope(),
        "clearly negative (~-2.7)",
    );
    row("Mental", &report.vet_ment_slope(), "~ flat (~+0.3)");
    println!();
    println!("--- Wonderkids (PA >= 80) ---");
    row(
        "Hit rate (attainment >= 0.90)",
        &report.wonderkid_hit(),
        "most (~0.56)",
    );
    row(
        "Flop rate (attainment < 0.75)",
        &report.wonderkid_flop(),
        "small but real (~0.04)",
    );
    println!();
    println!("--- Qualifying career counts per seed (mean) ---");
    let m = |xs: &[usize]| -> f64 {
        if xs.is_empty() {
            0.0
        } else {
            xs.iter().sum::<usize>() as f64 / xs.len() as f64
        }
    };
    println!(
        "phys-peak {:.0}, tech-onset {:.0}, ment-onset {:.0}, ca-peak {:.0}, cohort {:.0}, veteran {:.0}, wonderkid {:.0}",
        m(&report.n_phys_peak),
        m(&report.n_tech_onset),
        m(&report.n_ment_onset),
        m(&report.n_ca_peak),
        m(&report.n_cohort),
        m(&report.n_vet),
        m(&report.n_wonderkid)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The career-arc regression guard (`DEVELOPMENT_MODEL.md` §6): the
    /// development sibling of `aggregates_are_in_a_believable_ballpark`
    /// (`lib.rs`) and of `match_engine::calibrate`'s
    /// `favourite_discrimination_regression_guard`. It pools a small
    /// multi-season run over a couple of real-`worldgen` seeds and asserts the
    /// §6 headline metrics — peak ages, PA attainment, and the aging character —
    /// sit in *wide* believable bands. Like its siblings this is a
    /// gross-regression tripwire, not a fit gate: the bands are deliberately
    /// loose, sized to catch a curve that has come loose from the schema
    /// (physicals peaking at 19 or 33, prospects realizing 40% or 130% of PA,
    /// physicals that no longer decline), not to pin the fitted numbers.
    ///
    /// **Not asserted: a phys < tech < ment age ordering.** The scratchpad's
    /// from-youth cohort climbed the whole envelope, so its category peaks
    /// ordered cleanly. Real `worldgen` seeds players *near* their plateau
    /// (attributes shaped around club quality, not placed on `env_c(15)`), so
    /// for the flat categories the plateau-onset metric (§6) fires early and
    /// close together — technical and mental onset are a mid-20s wash, not
    /// separable in age. The schema commitment that *survives* on the real
    /// distribution is the **aging character**, not the maturation ordering:
    /// physicals peak and then decline hard while mental holds. That is what
    /// this guard pins (the veteran-slope split below), and it is the §7
    /// property that actually matters for squad-building.
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
    fn career_arcs_are_in_a_believable_ballpark() {
        let cfg = WorldGenConfig::default();
        let seeds: Vec<u64> = (0..2).collect();
        // A decade-plus so youth traced from ~16 reach their 30s and veterans
        // span the 30→35 decline band. (The bin runs more seeds × more seasons;
        // per-seed spread is tiny, so a 2×12 pool is a faithful tripwire.)
        let report = run_career_arc(&seeds, 12, &cfg);

        // --- believable age bands (loose; catch gross drift only) ---
        let phys = report.phys_peak_age();
        assert!(
            (23.0..=28.0).contains(&phys.mean),
            "physical peak age {:.2} outside believable band",
            phys.mean
        );
        let tech = report.tech_onset_age();
        assert!(
            (24.0..=32.0).contains(&tech.mean),
            "technical plateau onset {:.2} outside believable band",
            tech.mean
        );
        let ment = report.ment_onset_age();
        assert!(
            (24.0..=34.0).contains(&ment.mean),
            "mental plateau onset {:.2} outside believable band",
            ment.mean
        );
        let ca_peak = report.ca_peak_age();
        assert!(
            (25.0..=32.0).contains(&ca_peak.mean),
            "overall CA peak age {:.2} outside believable band",
            ca_peak.mean
        );

        // --- PA attainment: a believable central level and a real tail ---
        let attain = report.attainment_mean();
        assert!(
            (0.80..=0.95).contains(&attain.mean),
            "PA attainment mean {:.3} outside believable band",
            attain.mean
        );
        // Not everyone reaches PA — the underperforming tail must be non-empty.
        assert!(
            report.attainment_tail_frac().mean > 0.0,
            "no sub-0.80 attainment tail at all — the shortfall mechanism is dead"
        );

        // --- the aging character (§7), the ordering that survives real worldgen:
        // physicals decline clearly; mental barely moves; the gap between them is
        // wide. This is the squad-building-relevant fact the schema commits to.
        let vp = report.vet_phys_slope().mean;
        let vm = report.vet_ment_slope().mean;
        assert!(
            vp < -1.0,
            "veteran physical slope {vp:.2} not clearly declining"
        );
        assert!(
            vm > -0.6,
            "veteran mental slope {vm:.2} declines like a physical"
        );
        assert!(
            vm - vp > 1.5,
            "physical vs mental aging barely differ (phys {vp:.2}, ment {vm:.2}) — \
             the DevCategory curves have collapsed together"
        );
    }
}
