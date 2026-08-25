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

use crate::development::{self, DevKnobs, EnvTables, norms_by_role};
use crate::event::Event;
use crate::{Command, Session, WorldGenConfig, new_game, worldgen};
use fforge_domain::{
    Attribute, ClubId, DevCategory, Fixture, GameDate, PlayerId, ROLE_WEIGHTS, Role, World,
    best_role, date::DAYS_PER_YEAR,
};
use std::collections::BTreeMap;

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
    /// Best-role CA at the first sample (worldgen's output, before any
    /// development tick) — the wonderkid-flop-analysis task's `r0 = start_ca /
    /// pa`, the attainment a prospect starts at before any growth runs.
    start_ca: f64,
    /// Fixed for the whole arc (`natural_role`) — the W1b projection
    /// (`maturity(start_age - phi)`) needs it to read the right role's
    /// envelope blend, the same `role_maturity_ratio` the maturity-ratio
    /// report (task 4) already computes.
    role: Role,
    /// The player's once-resolved bloomer phase (`DevProfile::bloomer_phase`,
    /// years) — recorded, not re-drawn, so the W1b projection reads the same
    /// per-player noise draw the traced career actually used.
    phi: f64,
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
                    start_ca: sample.ca,
                    role,
                    phi: player.development.bloomer_phase(),
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

/// Record the age of every player seen for the first time — `Arc`'s "start
/// age" definition (age at worldgen or, later, youth intake) applied to a
/// plain `PlayerId -> age` map instead of a full traced arc, for the
/// clip-stats measurement below.
fn record_new_start_ages(world: &World, date: GameDate, start_age: &mut BTreeMap<PlayerId, f64>) {
    for (&pid, player) in &world.players {
        start_age
            .entry(pid)
            .or_insert_with(|| (date.days - player.birth.days) as f64 / DAYS_PER_YEAR as f64);
    }
}

/// The `[16, 17)` start-age cohort's `max_step`-clamp exposure
/// (`DEVELOPMENT_MODEL.md` §8.6's max-step-saturation escalation clause):
/// pools `(attempted, clipped)` weighted-attribute steps across every such
/// player's whole traced career.
///
/// Reconstructs, from the real per-matchday `MatchPlayed`/`DevelopmentTick`
/// events a real `Session` produces, the exact `(world, minutes,
/// club_matches)` triple `commands::dev_ticks_between` fed to
/// `development::tick_changes` for that matchday's boundary — then re-derives
/// the identical changes via `tick_changes_with_clip_stats` and
/// `debug_assert_eq!`s the result against the real recorded
/// `DevelopmentTick.changes`, so a reconstruction bug fails loudly rather
/// than silently mismeasuring. Limited to the single-tick case
/// `commands::advance_matchday`'s own doc comment describes ("at most one
/// [boundary], since a matchday step is 7 days") — a multi-tick
/// `StartNextSeason` offseason span is skipped for this diagnostic only (the
/// large majority of ticks are the single-tick case: ~37 matchdays/season
/// vs. one season transition).
fn trace_seed_clip_stats_16_band(seed: u64, seasons: usize, cfg: &WorldGenConfig) -> (u64, u64) {
    // The real `Session`/`commands::dev_ticks_between` pipeline this function
    // reconstructs against hardcodes `DevKnobs::default()` internally (it has
    // no seam to take a substituted table) — so the reconstruction must use
    // the identical table, not a caller-supplied one, or the
    // `debug_assert_eq!` below would fail on a currently-committed knob
    // table it was never actually measuring against.
    let knobs = DevKnobs::default();
    let log = new_game(seed, cfg, ClubId(0));
    let mut session = Session::from_events(log, &mut []);

    let mut start_age: BTreeMap<PlayerId, f64> = BTreeMap::new();
    record_new_start_ages(&session.state.world, session.state.date, &mut start_age);

    let mut attempted_16 = 0u64;
    let mut clipped_16 = 0u64;

    for s in 0..seasons {
        while !session.state.season_over() {
            let fixtures: Vec<Fixture> = session
                .state
                .fixtures_of_matchday(session.state.current_matchday)
                .cloned()
                .collect();
            let pre_world = session.state.world.clone();
            let pre_apps = session.state.appearances_since_tick.clone();
            let pre_club_matches = session.state.club_matches_since_tick.clone();
            let log_before = session.log.len();

            session
                .execute(Command::AdvanceMatchday, &mut [])
                .expect("advance matchday");

            record_new_start_ages(&session.state.world, session.state.date, &mut start_age);

            let mut this_apps: BTreeMap<PlayerId, u32> = BTreeMap::new();
            let mut tick: Option<(GameDate, Vec<crate::event::AttrStep>)> = None;
            for ev in &session.log[log_before..] {
                match ev {
                    Event::MatchPlayed { minutes, .. } => {
                        for &(pid, mins) in minutes {
                            *this_apps.entry(pid).or_default() += mins as u32;
                        }
                    }
                    Event::DevelopmentTick { date, changes } => {
                        tick = Some((*date, changes.clone()));
                    }
                    _ => {}
                }
            }

            let Some((tick_date, real_changes)) = tick else {
                continue; // no 30-day boundary crossed this matchday
            };

            let mut window_apps = pre_apps;
            for (pid, mins) in this_apps {
                *window_apps.entry(pid).or_default() += mins;
            }
            let mut window_club_matches = pre_club_matches;
            for f in &fixtures {
                *window_club_matches.entry(f.home).or_default() += 1;
                *window_club_matches.entry(f.away).or_default() += 1;
            }

            let period = development::period_index(tick_date);
            let (recomputed, clip_map) = development::tick_changes_with_clip_stats(
                &pre_world,
                session.state.seed,
                period,
                tick_date,
                &window_apps,
                &window_club_matches,
                &knobs,
            );
            debug_assert_eq!(
                recomputed, real_changes,
                "clip-stats reconstruction diverged from the real recorded tick \
                 — the window/world reconstruction above is not faithful"
            );

            for (pid, stats) in clip_map {
                if let Some(&sa) = start_age.get(&pid)
                    && (16.0..17.0).contains(&sa)
                {
                    attempted_16 += stats.attempted as u64;
                    clipped_16 += stats.clipped as u64;
                }
            }
        }
        if s + 1 < seasons {
            session
                .execute(Command::StartNextSeason, &mut [])
                .expect("start next season");
            record_new_start_ages(&session.state.world, session.state.date, &mut start_age);
        }
    }

    (attempted_16, clipped_16)
}

/// Pool `trace_seed_clip_stats_16_band` over `seeds`: the max-step-saturation
/// escalation clause's own reading (`DEVELOPMENT_MODEL.md` §8.6) — the
/// clipped fraction of monthly attribute steps for the `[16, 17)` start-age
/// cohort, read before fitting and again after each re-fit stage. Returns
/// `(attempted, clipped, fraction)`.
pub fn max_step_saturation_16_band(
    seeds: &[u64],
    seasons: usize,
    cfg: &WorldGenConfig,
) -> (u64, u64, f64) {
    let mut attempted = 0u64;
    let mut clipped = 0u64;
    for &seed in seeds {
        let (a, c) = trace_seed_clip_stats_16_band(seed, seasons, cfg);
        attempted += a;
        clipped += c;
    }
    let frac = if attempted > 0 {
        clipped as f64 / attempted as f64
    } else {
        f64::NAN
    };
    (attempted, clipped, frac)
}

/// Trace one world seed with growth **effectively disabled** — the
/// wonderkid-flop-analysis decisive test (task 2): if the flop rate and
/// attainment distribution are unchanged from the normal run even with no
/// growth mechanism running at all, the floor comes from worldgen's initial
/// state, not from anything the growth knobs can reach.
///
/// Deliberately does **not** drive `Session`/`Command::AdvanceMatchday` (that
/// pipeline hardcodes `DevKnobs::default()` for the monthly tick, so it can't
/// take a substituted knob table without changing `commands.rs`, which is out
/// of scope for a measurement-only pass). Instead it calls the same
/// `development::tick_changes`/`apply_attr_step` the fold uses, directly,
/// against a `World` built once via the real `worldgen::generate` — no
/// production file changes, no committed knob-default change. Playing no
/// real matches is safe for this probe specifically: with `knobs.k == 0` the
/// growth term is zero regardless of the playing-time multiplier, and the
/// aging (decline) term never reads minutes at all (`attr_rate`), so an empty
/// minutes map changes nothing either branch would otherwise do.
fn trace_seed_growth_disabled(
    seed: u64,
    years: f64,
    cfg: &WorldGenConfig,
    knobs: &DevKnobs,
) -> Vec<Arc> {
    let (mut world, _fixtures, start_date) = worldgen::generate(seed, cfg);
    let mut arcs: Vec<(u32, Arc)> = Vec::new();
    sample_world(&world, start_date, &mut arcs);

    let empty_apps: BTreeMap<PlayerId, u32> = BTreeMap::new();
    let empty_matches: BTreeMap<ClubId, u32> = BTreeMap::new();

    let start_idx = development::period_index(start_date);
    let end_idx =
        development::period_index(start_date.add_days((years * DAYS_PER_YEAR as f64) as i64));

    for period in (start_idx + 1)..=end_idx {
        let tick_date = development::period_date(period);
        let changes = development::tick_changes(
            &world,
            seed,
            period,
            tick_date,
            &empty_apps,
            &empty_matches,
            knobs,
        );
        for step in &changes {
            development::apply_attr_step(&mut world, step);
        }
        sample_world(&world, tick_date, &mut arcs);
    }

    arcs.into_iter().map(|(_, a)| a).collect()
}

/// Run the growth-disabled probe (task 2) over `seeds`, pooling into the same
/// `CareerArcReport` shape as the normal harness so the flop rate and
/// attainment distribution are directly comparable. `k` near zero kills the
/// proportional-growth term outright; `e_base`/`e_min` are also pinned at
/// their floor as the task specifies, though with `k == 0` they are already
/// inert (the growth branch multiplies by `k` regardless of `e`) — included
/// for faithfulness to the ask, not because they add a second lever here.
pub fn run_growth_disabled_probe(
    seeds: &[u64],
    years: f64,
    cfg: &WorldGenConfig,
) -> CareerArcReport {
    let knobs = DevKnobs {
        k: 0.0,
        e_base: 0.0,
        e_min: 0.0,
        ..DevKnobs::default()
    };
    let mut report = CareerArcReport {
        seeds: seeds.len(),
        seasons: 0,
        ..Default::default()
    };
    for &seed in seeds {
        let arcs = trace_seed_growth_disabled(seed, years, cfg, &knobs);
        report.record_seed(&arcs);
    }
    report
}

/// Result of task 3's `PA ~ a*CA + b*age + c` ordinary-least-squares fit
/// across every `worldgen`-generated player (not just the development
/// cohort) — the baseline for "is PA recoverable from (CA, age)".
pub struct PaFit {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub residual_sd: f64,
    pub n: usize,
}

/// Solve the 3x3 linear system `m * x = rhs` via Gaussian elimination with
/// partial pivoting. Local to this one-off measurement fit — no need for a
/// linear-algebra dependency for a single 3x3 solve.
#[allow(clippy::needless_range_loop)]
fn solve3(mut m: [[f64; 3]; 3], mut rhs: [f64; 3]) -> [f64; 3] {
    for col in 0..3 {
        let pivot = (col..3)
            .max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))
            .unwrap();
        m.swap(col, pivot);
        rhs.swap(col, pivot);
        for row in (col + 1)..3 {
            let factor = m[row][col] / m[col][col];
            for k in col..3 {
                m[row][k] -= factor * m[col][k];
            }
            rhs[row] -= factor * rhs[col];
        }
    }
    let mut x = [0.0; 3];
    for row in (0..3).rev() {
        let sum: f64 = (row + 1..3).map(|k| m[row][k] * x[k]).sum();
        x[row] = (rhs[row] - sum) / m[row][row];
    }
    x
}

/// Task 3: fit `PA ~ a*CA + b*age + c` by ordinary least squares over every
/// player `worldgen` generates (pooled across `seeds`), and report the
/// residual standard deviation — how tightly PA is determined by (CA, age)
/// alone at generation time.
pub fn fit_pa_from_ca_age(seeds: &[u64], cfg: &WorldGenConfig) -> PaFit {
    fit_pa_from_ca_age_filtered(seeds, cfg, |_age| true)
}

/// W1b amendment §4: the same fit restricted to `age < 24` — worldgen's
/// `headroom` formula is piecewise (`2*(24-age) + U(0,8)` below 24, `U(0,3)`
/// at/above), so a single linear-in-age fit over the whole population is
/// misspecified across that kink. Restricting to the youth band before the
/// kink removes that source of residual and isolates the genuine conditional
/// uncertainty for the population scouting actually cares about.
pub fn fit_pa_from_ca_age_youth(seeds: &[u64], cfg: &WorldGenConfig) -> PaFit {
    fit_pa_from_ca_age_filtered(seeds, cfg, |age| age < 24.0)
}

/// The same fit restricted to one single-year age band `[lo, hi)`
/// (`DEVELOPMENT_MODEL.md` §8.4: the `residual_sd` rise is predicted
/// strongest at 16, weakest at 21) — the per-band decomposition `fit_youth`'s
/// pooled 16-23 read cannot show on its own.
pub fn fit_pa_from_ca_age_band(seeds: &[u64], cfg: &WorldGenConfig, lo: f64, hi: f64) -> PaFit {
    fit_pa_from_ca_age_filtered(seeds, cfg, move |age| age >= lo && age < hi)
}

fn fit_pa_from_ca_age_filtered(
    seeds: &[u64],
    cfg: &WorldGenConfig,
    age_filter: impl Fn(f64) -> bool,
) -> PaFit {
    let mut rows: Vec<(f64, f64, f64)> = Vec::new(); // (ca, age, pa)
    for &seed in seeds {
        let (world, _fixtures, start_date) = worldgen::generate(seed, cfg);
        for player in world.players.values() {
            let age = (start_date.days - player.birth.days) as f64 / DAYS_PER_YEAR as f64;
            if !age_filter(age) {
                continue;
            }
            let ca = best_role(&player.attributes, &ROLE_WEIGHTS).1 as f64;
            let pa = player.character.potential as f64;
            rows.push((ca, age, pa));
        }
    }

    let n = rows.len() as f64;
    let (mut s_ca, mut s_age, mut s_pa) = (0.0, 0.0, 0.0);
    let (mut s_ca2, mut s_age2, mut s_ca_age) = (0.0, 0.0, 0.0);
    let (mut s_ca_pa, mut s_age_pa) = (0.0, 0.0);
    for &(ca, age, pa) in &rows {
        s_ca += ca;
        s_age += age;
        s_pa += pa;
        s_ca2 += ca * ca;
        s_age2 += age * age;
        s_ca_age += ca * age;
        s_ca_pa += ca * pa;
        s_age_pa += age * pa;
    }

    let m = [
        [s_ca2, s_ca_age, s_ca],
        [s_ca_age, s_age2, s_age],
        [s_ca, s_age, n],
    ];
    let rhs = [s_ca_pa, s_age_pa, s_pa];
    let [a, b, c] = solve3(m, rhs);

    let sse: f64 = rows
        .iter()
        .map(|&(ca, age, pa)| {
            let resid = pa - (a * ca + b * age + c);
            resid * resid
        })
        .sum();
    let residual_sd = (sse / (n - 3.0)).sqrt();

    PaFit {
        a,
        b,
        c,
        residual_sd,
        n: rows.len(),
    }
}

/// Solve an `n x n` linear system by Gaussian elimination with partial
/// pivoting — `solve3`'s generalization for the COMPETENT fit's 5 unknowns
/// (`fit_pa_from_composites_age_filtered`). One-off measurement code; not
/// worth a linear-algebra dependency for a handful of small, dense solves.
#[allow(clippy::needless_range_loop)]
fn solve_n(mut m: Vec<Vec<f64>>, mut rhs: Vec<f64>) -> Vec<f64> {
    let n = rhs.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))
            .unwrap();
        m.swap(col, pivot);
        rhs.swap(col, pivot);
        for row in (col + 1)..n {
            let factor = m[row][col] / m[col][col];
            for k in col..n {
                m[row][k] -= factor * m[col][k];
            }
            rhs[row] -= factor * rhs[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let sum: f64 = (row + 1..n).map(|k| m[row][k] * x[k]).sum();
        x[row] = (rhs[row] - sum) / m[row][row];
    }
    x
}

/// Result of the COMPETENT attack's `PA ~ a*phys + b*tech + c*ment + d*age +
/// e` ordinary-least-squares fit (`DEVELOPMENT_MODEL.md` §8.4's two-attack
/// measurement): `coeffs` is `[phys, tech, ment, age, intercept]`.
pub struct PaFitMulti {
    pub coeffs: [f64; 5],
    pub residual_sd: f64,
    pub n: usize,
}

/// The COMPETENT attack on PA: fit `PA` on the per-`DevCategory` composites
/// (physical/technical/mental — `category_composite`, the same aggregation
/// `career_arc`'s own arc-tracing already uses) plus age, instead of on raw
/// best-role CA alone. Under envelope-consistent seeding the composite
/// *ratios* partially decode the hidden bloomer phase φ (φ shifts the whole
/// envelope, but each category's envelope has a different shape, so a
/// φ-shifted player's phys/tech/ment mix differs from an on-schedule player's
/// even at matched CA) — this is the skilled-observer attack the NAIVE
/// `fit_pa_from_ca_age*` fits cannot make. The gap between the two residuals
/// is the headroom a real scouting signal could close.
fn fit_pa_from_composites_age_filtered(
    seeds: &[u64],
    cfg: &WorldGenConfig,
    age_filter: impl Fn(f64) -> bool,
) -> PaFitMulti {
    // rows: (phys, tech, ment, age, pa)
    let mut rows: Vec<(f64, f64, f64, f64, f64)> = Vec::new();
    for &seed in seeds {
        let (world, _fixtures, start_date) = worldgen::generate(seed, cfg);
        for player in world.players.values() {
            let age = (start_date.days - player.birth.days) as f64 / DAYS_PER_YEAR as f64;
            if !age_filter(age) {
                continue;
            }
            let role = player.natural_role;
            let phys = category_composite(role, &player.attributes, DevCategory::Physical);
            let tech = category_composite(role, &player.attributes, DevCategory::Technical);
            let ment = category_composite(role, &player.attributes, DevCategory::Mental);
            let pa = player.character.potential as f64;
            rows.push((phys, tech, ment, age, pa));
        }
    }

    // Design-matrix predictors, in `coeffs`' order, plus a constant 1 for the
    // intercept — build the 5x5 normal-equations system X'X x = X'y directly
    // rather than materializing X.
    let n = rows.len();
    let mut xtx = vec![vec![0.0; 5]; 5];
    let mut xty = vec![0.0; 5];
    for &(phys, tech, ment, age, pa) in &rows {
        let x = [phys, tech, ment, age, 1.0];
        for i in 0..5 {
            xty[i] += x[i] * pa;
            for j in 0..5 {
                xtx[i][j] += x[i] * x[j];
            }
        }
    }
    let coeffs_vec = solve_n(xtx, xty);
    let coeffs: [f64; 5] = coeffs_vec.try_into().unwrap();

    let sse: f64 = rows
        .iter()
        .map(|&(phys, tech, ment, age, pa)| {
            let pred =
                coeffs[0] * phys + coeffs[1] * tech + coeffs[2] * ment + coeffs[3] * age + coeffs[4];
            (pa - pred).powi(2)
        })
        .sum();
    let residual_sd = (sse / (n as f64 - 5.0)).sqrt();

    PaFitMulti {
        coeffs,
        residual_sd,
        n,
    }
}

/// The COMPETENT fit over every `worldgen`-generated player, all ages —
/// `fit_pa_from_ca_age`'s sibling.
pub fn fit_pa_from_composites_age(seeds: &[u64], cfg: &WorldGenConfig) -> PaFitMulti {
    fit_pa_from_composites_age_filtered(seeds, cfg, |_age| true)
}

/// The COMPETENT fit restricted to `age < 24` — `fit_pa_from_ca_age_youth`'s
/// sibling, same population.
pub fn fit_pa_from_composites_age_youth(seeds: &[u64], cfg: &WorldGenConfig) -> PaFitMulti {
    fit_pa_from_composites_age_filtered(seeds, cfg, |age| age < 24.0)
}

/// The COMPETENT fit restricted to one single-year age band `[lo, hi)` —
/// `fit_pa_from_ca_age_band`'s sibling, so the NAIVE/COMPETENT gap can be
/// read per band, not just pooled.
pub fn fit_pa_from_composites_age_band(
    seeds: &[u64],
    cfg: &WorldGenConfig,
    lo: f64,
    hi: f64,
) -> PaFitMulti {
    fit_pa_from_composites_age_filtered(seeds, cfg, move |age| age >= lo && age < hi)
}

/// Task 4: the maturity ratio `env_c(y) / NORM` at a given age, for a given
/// `Role` — the role-weighted blend of each category's already-built envelope
/// (`EnvTables::env_at`, reusing the identical inner loop `norms_by_role`
/// uses, just at a fixed age instead of scanning for the age-maximum) divided
/// by that role's `NORM`. This is `target_i`'s scaling factor with the `PA`
/// term stripped out — "what fraction of this role's ultimate ceiling does
/// the envelope license at age `y`."
fn role_maturity_ratio(envs: &EnvTables, norms: &[f64], role: Role, y: f64) -> f64 {
    let mut num = 0.0;
    let mut den = 0.0;
    for attr in Attribute::ALL {
        let w = ROLE_WEIGHTS.weight(role, attr) as f64;
        if w > 0.0 {
            num += w * envs.env_at(attr.dev_category(), y);
            den += w;
        }
    }
    (num / den) / norms[role.index()]
}

/// Pretty-print task 4's maturity-ratio table for a `DevKnobs` table (the
/// production default unless a caller is probing a variant).
pub fn print_maturity_ratios(knobs: &DevKnobs) {
    let envs = EnvTables::new(knobs);
    let norms = norms_by_role(&envs);
    println!("--- Maturity ratio env_c(y)/NORM by Role (DevTables machinery) ---");
    println!(
        "{:<4} {:>8} {:>8} {:>8} {:>8}",
        "Role", "age16", "age18", "age20", "age22"
    );
    for role in Role::ALL {
        let ratios: Vec<f64> = [16.0, 18.0, 20.0, 22.0]
            .iter()
            .map(|&y| role_maturity_ratio(&envs, &norms, role, y))
            .collect();
        println!(
            "{:<4} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
            format!("{role:?}"),
            ratios[0],
            ratios[1],
            ratios[2],
            ratios[3]
        );
    }
}

// --- W1b: arithmetic projection of the env-consistent seeding fix ---------
//
// `WONDERKID_FLOP_DIAGNOSIS.md`'s amendment, §5: before touching `worldgen`,
// predict what an env-consistent reseed (`r0' = maturity(start_age - phi)`,
// task 4's table) would do to the flop/hit rates, using the *already-traced*
// arcs' own gap-closure fraction `f = (attainment - r0) / (1 - r0)` — no new
// simulation, since `f` is (approximately) scale-invariant under the
// proportional growth law (§2 of the amendment). Two honest limitations,
// reported alongside every number rather than worked around:
// - `f` is only exactly scale-invariant for a pure proportional law;
//   `max_step` quantization and additive jitter both reduce `f` at larger
//   gaps, so `attainment'` here is an upper bound and the projected flop
//   rate a lower bound.
// - `phi` is read from each arc's own recorded `DevProfile`, never re-drawn,
//   so the projection stays over the same population it is predicting for.

/// One cohort arc's contribution to the projection: the actual `r0`/
/// `attainment` this arc measured, and the hypothetical `r0'`/`attainment'`
/// an env-consistent reseed would have produced for the *same* player (same
/// `f`, same `phi`, different starting point).
struct ProjRow {
    /// `floor(start_age)` — the age-band split the pooled arithmetic in the
    /// amendment's §3 couldn't do.
    start_age_band: i64,
    is_wonderkid: bool,
    r0: f64,
    r0_proj: f64,
    attainment: f64,
    attainment_proj: f64,
}

/// Pooled hit/flop/attainment/tail stats for one group of `ProjRow`s — either
/// one start-age band or the `overall()` pool across all bands.
pub struct ProjectionBandStats {
    /// The band's `floor(start_age)`, or `-1` for `overall()`'s all-bands pool.
    pub start_age_band: i64,
    pub n: usize,
    pub n_wonderkid: usize,
    pub r0_mean: f64,
    pub r0_proj_mean: f64,
    pub attainment_mean: f64,
    pub attainment_proj_mean: f64,
    pub sub80_frac: f64,
    pub sub80_proj_frac: f64,
    pub hit_rate: f64,
    pub hit_rate_proj: f64,
    pub flop_rate: f64,
    pub flop_rate_proj: f64,
}

/// Fraction of `xs` satisfying `pred` — `NaN` for an empty slice, matching
/// `mean_finite`'s convention.
fn frac(xs: &[f64], pred: impl Fn(f64) -> bool) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    xs.iter().filter(|&&x| pred(x)).count() as f64 / xs.len() as f64
}

fn summarize_band(band: i64, rows: &[&ProjRow]) -> ProjectionBandStats {
    let r0: Vec<f64> = rows.iter().map(|r| r.r0).collect();
    let r0_proj: Vec<f64> = rows.iter().map(|r| r.r0_proj).collect();
    let attainment: Vec<f64> = rows.iter().map(|r| r.attainment).collect();
    let attainment_proj: Vec<f64> = rows.iter().map(|r| r.attainment_proj).collect();
    let wk_attainment: Vec<f64> = rows
        .iter()
        .filter(|r| r.is_wonderkid)
        .map(|r| r.attainment)
        .collect();
    let wk_attainment_proj: Vec<f64> = rows
        .iter()
        .filter(|r| r.is_wonderkid)
        .map(|r| r.attainment_proj)
        .collect();

    ProjectionBandStats {
        start_age_band: band,
        n: rows.len(),
        n_wonderkid: wk_attainment.len(),
        r0_mean: mean_finite(&r0),
        r0_proj_mean: mean_finite(&r0_proj),
        attainment_mean: mean_finite(&attainment),
        attainment_proj_mean: mean_finite(&attainment_proj),
        sub80_frac: frac(&attainment, |a| a < ATTAINMENT_TAIL),
        sub80_proj_frac: frac(&attainment_proj, |a| a < ATTAINMENT_TAIL),
        hit_rate: frac(&wk_attainment, |a| a >= WONDERKID_HIT),
        hit_rate_proj: frac(&wk_attainment_proj, |a| a >= WONDERKID_HIT),
        flop_rate: frac(&wk_attainment, |a| a < WONDERKID_FLOP),
        flop_rate_proj: frac(&wk_attainment_proj, |a| a < WONDERKID_FLOP),
    }
}

/// Pooled W1b projection rows across every traced seed.
#[derive(Default)]
pub struct SeedingProjectionReport {
    rows: Vec<ProjRow>,
}

impl SeedingProjectionReport {
    /// Fold one seed's already-traced arcs into the projection — the same
    /// cohort filter `CareerArcReport::record_seed` uses, so the two reports
    /// are reading the same population.
    fn record_seed(&mut self, arcs: &[Arc], envs: &EnvTables, norms: &[f64]) {
        for arc in arcs {
            let hi = arc.max_age();
            if !(arc.start_age <= COHORT_MAX_START_AGE && hi >= COHORT_MIN_END_AGE && arc.pa > 0.0)
            {
                continue;
            }
            let r0 = arc.start_ca / arc.pa;
            let gap = 1.0 - r0;
            if gap <= 1e-9 {
                continue; // no headroom to measure a gap-closure fraction from
            }
            let attainment = arc.peak_ca() / arc.pa;
            let f = (attainment - r0) / gap;
            let y = arc.start_age - arc.phi; // bloomer-shifted seeding age (§2's convention)
            let r0_proj = role_maturity_ratio(envs, norms, arc.role, y).clamp(0.0, 1.0);
            let attainment_proj = r0_proj + f * (1.0 - r0_proj);
            self.rows.push(ProjRow {
                start_age_band: arc.start_age.floor() as i64,
                is_wonderkid: arc.pa >= WONDERKID_PA,
                r0,
                r0_proj,
                attainment,
                attainment_proj,
            });
        }
    }

    /// Per-start-age-band stats, ascending age — the split the pooled
    /// arithmetic in the amendment's §3 couldn't produce.
    pub fn bands(&self) -> Vec<ProjectionBandStats> {
        let mut by_band: BTreeMap<i64, Vec<&ProjRow>> = BTreeMap::new();
        for row in &self.rows {
            by_band.entry(row.start_age_band).or_default().push(row);
        }
        by_band
            .into_iter()
            .map(|(band, rows)| summarize_band(band, &rows))
            .collect()
    }

    /// All bands pooled — the top-line number the amendment's §5 decision
    /// rule reads.
    pub fn overall(&self) -> ProjectionBandStats {
        let rows: Vec<&ProjRow> = self.rows.iter().collect();
        summarize_band(-1, &rows)
    }

    /// `start_age_band <= 18` pooled (§8.3: the headline wonderkid hit/flop
    /// population, narrower than `overall()`'s full `<= 21` pool) —
    /// `start_age_band` is `-2` as a distinct marker from `overall()`'s `-1`.
    pub fn le18(&self) -> ProjectionBandStats {
        let rows: Vec<&ProjRow> = self.rows.iter().filter(|r| r.start_age_band <= 18).collect();
        summarize_band(-2, &rows)
    }
}

/// Run the career-arc harness and the W1b projection together off the same
/// traced arcs — the projection needs no extra tracing (it's pure arithmetic
/// on `attainment`/`r0`/`phi` the normal trace already produces), so both
/// reports come from one pass over `seeds` rather than re-running the real
/// worldgen+match+development pipeline a second time.
pub fn run_career_arc_with_projection(
    seeds: &[u64],
    seasons: usize,
    cfg: &WorldGenConfig,
) -> (CareerArcReport, SeedingProjectionReport) {
    let dev_knobs = DevKnobs::default();
    let envs = EnvTables::new(&dev_knobs);
    let norms = norms_by_role(&envs);

    let mut report = CareerArcReport {
        seeds: seeds.len(),
        seasons,
        ..Default::default()
    };
    let mut projection = SeedingProjectionReport::default();
    for &seed in seeds {
        let arcs = trace_seed(seed, seasons, cfg);
        report.record_seed(&arcs);
        projection.record_seed(&arcs, &envs, &norms);
    }
    (report, projection)
}

/// Pretty-print the W1b projection: per-band and overall actual-vs-projected
/// stats, plus the amendment's §5 decision-rule read on the pooled projected
/// flop rate.
pub fn print_seeding_projection(report: &SeedingProjectionReport) {
    println!("--- W1b: arithmetic projection of the env-consistent seeding fix ---");
    println!("(f = (attainment - r0)/(1 - r0) from the arcs already traced above, applied to");
    println!(" r0' = maturity(start_age - phi); NOT a re-simulation of worldgen or DevKnobs.");
    println!(" attainment' is an upper bound / flop' a lower bound — see this fn's doc comment.)");
    println!();
    println!(
        "{:>4} {:>5} {:>5} {:>6} {:>6} {:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "band", "n", "n_wk", "r0", "r0'", "attain", "attain'", "sub80", "sub80'", "hit", "hit'",
        "flop", "flop'"
    );
    for b in report.bands() {
        println!(
            "{:>4} {:>5} {:>5} {:>6.3} {:>6.3} {:>7.3} {:>7.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
            b.start_age_band,
            b.n,
            b.n_wonderkid,
            b.r0_mean,
            b.r0_proj_mean,
            b.attainment_mean,
            b.attainment_proj_mean,
            b.sub80_frac,
            b.sub80_proj_frac,
            b.hit_rate,
            b.hit_rate_proj,
            b.flop_rate,
            b.flop_rate_proj
        );
    }
    println!();
    let o = report.overall();
    println!(
        "OVERALL: n={} n_wk={}  r0 {:.3}->{:.3}  attainment {:.3}->{:.3}  sub80 {:.3}->{:.3}  hit {:.3}->{:.3}  flop {:.3}->{:.3}",
        o.n,
        o.n_wonderkid,
        o.r0_mean,
        o.r0_proj_mean,
        o.attainment_mean,
        o.attainment_proj_mean,
        o.sub80_frac,
        o.sub80_proj_frac,
        o.hit_rate,
        o.hit_rate_proj,
        o.flop_rate,
        o.flop_rate_proj
    );
    println!();
    let verdict = if o.flop_rate_proj <= 0.10 {
        "<=10%: close to drop-in — proceed with W3 as planned, W4 as a normal re-fit"
    } else if o.flop_rate_proj <= 0.30 {
        "10-30%: proceed, but land W3 and W4 together in one PR"
    } else {
        ">=30%: STOP — escalate as design, not fit (amendment §5's decision rule)"
    };
    println!(
        "Decision rule on projected flop rate {:.3}: {}",
        o.flop_rate_proj, verdict
    );
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

    // --- wonderkid-flop-analysis additions: r0 = start_ca / pa (§2.2's floor
    // hypothesis) pooled over the wonderkid (PA >= 80) cohort, and the pooled
    // (attainment - r0) gap it leaves for growth to have actually produced.
    all_r0_wonderkid: Vec<f64>,
    all_attain_minus_r0_wonderkid: Vec<f64>,
    /// Arcs (over the whole development cohort, not just wonderkids) where
    /// `attainment < r0` — should never happen for a pre-peak-only decline
    /// model (§2.1: downward pull only acts past the category's envelope
    /// peak, and `peak_ca()` is a max over the whole traced arc). Non-empty
    /// means pre-peak decline is reachable and this analysis is incomplete.
    r0_violations: Vec<(f64, f64)>, // (attainment, r0) pairs that violated
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

/// Mean, sd, min of a pooled (not per-seed) raw sample — the distribution
/// readout for the wonderkid-flop-analysis r0/attainment-gap report, which
/// pools individual arcs rather than per-seed means (unlike `SeedSpread`).
pub struct PooledStats {
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub p10: f64,
    pub n: usize,
}

fn pooled_stats(xs: &[f64]) -> PooledStats {
    let valid: Vec<f64> = xs.iter().copied().filter(|x| x.is_finite()).collect();
    let n = valid.len();
    if n == 0 {
        return PooledStats {
            mean: f64::NAN,
            sd: f64::NAN,
            min: f64::NAN,
            p10: f64::NAN,
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
    PooledStats {
        mean,
        sd,
        min,
        p10: percentile(&valid, 0.10),
        n,
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

                let r0 = arc.start_ca / arc.pa;
                // peak_ca() is a max over the whole traced arc and includes the
                // starting sample itself, so attainment >= r0 always *unless*
                // a pre-peak player can decline — which §2.1 says cannot
                // happen (the downward pull only acts past the category's
                // envelope peak). Recorded as a violation, not a panic, so one
                // bad arc doesn't blank the rest of this report.
                if attainment < r0 - 1e-9 {
                    self.r0_violations.push((attainment, r0));
                }

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
                    self.all_r0_wonderkid.push(r0);
                    self.all_attain_minus_r0_wonderkid.push(attainment - r0);
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

    /// `r0 = start_ca / pa` distribution over the wonderkid (PA >= 80) cohort
    /// — the worldgen-floor hypothesis's central object.
    pub fn r0_wonderkid(&self) -> PooledStats {
        pooled_stats(&self.all_r0_wonderkid)
    }
    /// `attainment - r0` distribution over the same cohort — how much of
    /// final attainment growth actually added, on top of the worldgen floor.
    pub fn attainment_minus_r0_wonderkid(&self) -> PooledStats {
        pooled_stats(&self.all_attain_minus_r0_wonderkid)
    }
    /// Arcs where `attainment < r0` — see the field doc; should be empty.
    pub fn r0_violations(&self) -> &[(f64, f64)] {
        &self.r0_violations
    }
    /// Fraction of the wonderkid cohort born with `r0 < 0.75` already — i.e.
    /// already a flop by worldgen's own headroom draw, before any growth (or
    /// lack of it) runs at all. Directly explains the growth-disabled probe's
    /// nonzero flop rate without needing that probe's own sampling noise.
    pub fn r0_below_flop_frac(&self) -> f64 {
        if self.all_r0_wonderkid.is_empty() {
            return f64::NAN;
        }
        self.all_r0_wonderkid
            .iter()
            .filter(|&&r| r < WONDERKID_FLOP)
            .count() as f64
            / self.all_r0_wonderkid.len() as f64
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
    println!("--- Wonderkid-flop-analysis: r0 = start_ca / PA (PA >= 80 cohort) ---");
    let r0 = report.r0_wonderkid();
    println!(
        "r0                              : mean {:.3}, sd {:.3}, min {:.3}, p10 {:.3} ({} arcs)",
        r0.mean, r0.sd, r0.min, r0.p10, r0.n
    );
    let gap = report.attainment_minus_r0_wonderkid();
    println!(
        "attainment - r0                 : mean {:.3}, sd {:.3}, min {:.3}, p10 {:.3} ({} arcs)",
        gap.mean, gap.sd, gap.min, gap.p10, gap.n
    );
    println!(
        "fraction of wonderkid cohort born with r0 < 0.75 : {:.4}  (already a flop at birth, before any growth)",
        report.r0_below_flop_frac()
    );
    let violations = report.r0_violations();
    if violations.is_empty() {
        println!("attainment >= r0 holds for every cohort arc (no violations)");
    } else {
        println!(
            "WARNING: {} arc(s) with attainment < r0 — pre-peak decline is reachable, analysis incomplete:",
            violations.len()
        );
        for (attainment, r0) in violations.iter().take(10) {
            println!("  attainment {attainment:.3} < r0 {r0:.3}");
        }
    }
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

    /// `max_step_saturation_16_band`'s reconstruction is validated by its own
    /// internal `debug_assert_eq!` against the real recorded
    /// `DevelopmentTick.changes` on every single-tick boundary it processes
    /// (`DEVELOPMENT_MODEL.md` §8.6) — this test just needs to run that path,
    /// in a debug build, across enough matchdays to cross at least one
    /// 30-day boundary, and confirm it returns a sane (non-NaN, in-range)
    /// fraction rather than panicking.
    #[test]
    fn max_step_saturation_reconstruction_matches_the_real_recorded_ticks() {
        let cfg = WorldGenConfig {
            num_clubs: 4,
            ..Default::default()
        };
        let (attempted, clipped, frac) = max_step_saturation_16_band(&[1, 2], 2, &cfg);
        assert!(attempted > 0, "expected at least one attempted step");
        assert!(clipped <= attempted);
        assert!((0.0..=1.0).contains(&frac), "frac {frac} out of range");
    }

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
