//! The Phase-3 player-development engine (`DEVELOPMENT_MODEL.md` §2–§5).
//!
//! A PA-scaled, age-shaped **target** each attribute tracks: growth is
//! proportional approach to that target (diminishing returns near PA, §2.2),
//! aging is the category envelope turning down (§2.1), and a plasticity window
//! plus the once-resolved per-player noise (`E`, `φ`) give wonderkids-who-flop
//! and late bloomers (§2.3). The constants live in `DevKnobs` — the sibling of
//! `match_engine::Knobs`, and likewise a *fitted starting point* to be re-fit by
//! a calibration harness, not a finished calibration.
//!
//! **Purity boundary (fforge-core invariant 2).** This module is only ever
//! called from `commands::step`, which *produces* the recorded
//! `Event::DevelopmentTick { changes }`. All RNG and growth math is here; the
//! fold (`GameState::apply`) only *adds* the recorded integer deltas. So the
//! growth model can evolve freely and never rewrite a recorded career — the same
//! record-outcomes guarantee `MatchPlayed` gives scores (`event.rs`).
//!
//! **Divergence from `DEVELOPMENT_MODEL.md` §2 (filed, per the workspace
//! CLAUDE.md).** The doc's `target_i = (PA/NORM)·env_c` scales every attribute
//! of a category to the *same* level, which — validated on a 3-composite model —
//! flattens the role shape across 25 real attributes (a centre-back's Finishing
//! would grow to his Tackling). We keep the doc's role-weighted `NORM` for the
//! level/age scaling exactly as specified, but multiply by a **role-shaped
//! per-attribute ceiling** (mirroring `worldgen`'s own weight shaping) so
//! position-relative CA — a hard schema property — survives development. This is
//! a faithful realization of §2.2's stated intent ("growth steered toward the
//! attributes the role values"); the doc's §2 pseudocode should be updated to
//! match.

use crate::event::AttrStep;
use crate::rng::{Rng, derive_stream};
use fforge_domain::{
    Attribute, ClubId, DevCategory, GameDate, PlayerId, ROLE_WEIGHTS, Role, World, best_role,
};
use std::collections::BTreeMap;

/// Tag namespace for the per-tick development RNG stream (`rng::derive_stream`),
/// distinct from `commands::FIXTURE_STREAM_NS`. The tick's period index is
/// OR'd into the low bits, so every month draws an independent stream.
pub const DEV_STREAM_NS: u64 = 0x4445_5645_0000_0000; // "DEVE"

/// One development tick covers this many sim-days. ~monthly (`DESIGN.md` §4.2):
/// 365/30 ≈ 12.2 ticks/year. A tick fires whenever a calendar advance crosses a
/// new 30-day period boundary (`commands::advance_matchday` / `start_next_season`).
pub const DEV_TICK_PERIOD_DAYS: i64 = 30;

const DAYS_PER_YEAR: f64 = fforge_domain::date::DAYS_PER_YEAR as f64;
pub(crate) const NUM_CATEGORIES: usize = 4;

/// The 30-day period a date falls in — the tick's identity and RNG-stream tag.
#[inline]
pub fn period_index(date: GameDate) -> i64 {
    date.days.div_euclid(DEV_TICK_PERIOD_DAYS)
}

/// The date at the start of a 30-day period (a tick resolves player ages here).
#[inline]
pub fn period_date(index: i64) -> GameDate {
    GameDate {
        days: index * DEV_TICK_PERIOD_DAYS,
    }
}

/// One category's age envelope (§2.1): a maturation logistic minus an aging
/// logistic. `env(y) = clamp(grow(y) − loss(y), 0, 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvParams {
    /// Maturation logistic midpoint (age of half-growth).
    pub g: f64,
    /// Maturation logistic width.
    pub s: f64,
    /// Peak fraction lost to aging.
    pub lmax: f64,
    /// Aging logistic midpoint (decline onset).
    pub d: f64,
    /// Aging logistic width.
    pub w: f64,
}

impl EnvParams {
    #[inline]
    fn env(&self, y: f64) -> f64 {
        self.env_lmax(y, self.lmax)
    }

    /// `env` with the aging peak-loss overridden — the seam Professionalism uses
    /// to flatten physical decline (§3, "the pro who ages well").
    #[inline]
    fn env_lmax(&self, y: f64, lmax: f64) -> f64 {
        let grow = 1.0 / (1.0 + (-(y - self.g) / self.s).exp());
        let loss = lmax / (1.0 + (-(y - self.d) / self.w).exp());
        (grow - loss).clamp(0.0, 1.0)
    }
}

/// The development knob table (`DEVELOPMENT_MODEL.md` §2–§3), sibling of
/// `match_engine::Knobs`. Values began as the fitted `dev_shape` scratchpad
/// point (`DEVELOPMENT_MODEL.md` §6); several have since been **re-fit against
/// real `worldgen`** by the career-arc harness (`crate::career_arc`), exactly as
/// `b_beat` was re-fit for the match engine — the fields carrying a re-fit note
/// below (`env_phys`, `plast_*`, `e_sigma`/`e_min`, and the earlier `k_dec`) are
/// the ones the real distribution moved. See `DEVELOPMENT_MODEL.md` §6 for the
/// banked readings and the two structural findings (near-plateau seeding, the
/// attainment floor) the harness surfaced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevKnobs {
    // --- per-category age envelopes (§2.1) ---
    pub env_phys: EnvParams,
    pub env_tech: EnvParams,
    pub env_ment: EnvParams,
    /// Goalkeeping (§2.1 note): flat, late, graceful — GKs age well by
    /// construction. The doc's §2.1 table is 3 rows; this fourth is the code's
    /// mental-but-flatter resolution for the `Goalkeeping` `DevCategory`.
    pub env_gk: EnvParams,

    // --- tracking rates (§2) ---
    /// Base growth tracking rate/yr (scaled by E, plasticity, coaching, minutes).
    pub k: f64,
    /// Aging (decline) tracking rate/yr — not E-gated; aging always applies.
    pub k_dec: f64,

    // --- plasticity window plast(y) = 1/(1+exp((y-mid)/width)) (§2.3) ---
    pub plast_mid: f64,
    pub plast_width: f64,

    // --- role-shaped ceiling (the §2.2-intent realization; see module note) ---
    /// Per-weight-point spread around the PA base (mirrors `worldgen`'s shaping).
    pub ceil_spread: f64,
    /// Floor a weighted attribute's ceiling never drops below.
    pub ceil_floor: f64,

    // --- growth efficiency E ~ N(base + det·(Det-50) + prof·(Prof-50), σ) (§2.3) ---
    pub e_base: f64,
    pub e_det: f64,
    pub e_prof: f64,
    pub e_sigma: f64,
    pub e_min: f64,
    pub e_max: f64,

    /// How much Professionalism flattens physical aging (§3): the physical
    /// envelope's `Lmax` is scaled by `1 − coeff·(Prof−50)/50`. The pro ages well.
    pub prof_aging_coeff: f64,

    /// Bloomer phase φ ~ N(0, σ) years (§2.3).
    pub phi_sigma: f64,
    /// Monthly jitter added to the annual rate before quantization (§2.3).
    pub jitter_sigma: f64,

    // --- playing-time multiplier, coarse appeared/benched/absent (§3) ---
    /// Appearance share (of the club's window matches) at/above which a player
    /// counts as a regular.
    pub minutes_regular_share: f64,
    pub minutes_regular: f64,
    pub minutes_rotation: f64,
    pub minutes_absent: f64,

    // --- per-club coaching coefficient resolution (§3) ---
    pub coaching_min: f64,
    pub coaching_max: f64,
    pub coaching_sigma: f64,

    /// Cap on a single tick's integer step per attribute (defensive).
    pub max_step: u8,
}

impl DevKnobs {
    /// A tick's fraction of a year (`DEV_TICK_PERIOD_DAYS / 365`).
    #[inline]
    pub fn dt(&self) -> f64 {
        DEV_TICK_PERIOD_DAYS as f64 / DAYS_PER_YEAR
    }

    fn env(&self, cat: DevCategory) -> &EnvParams {
        match cat {
            DevCategory::Physical => &self.env_phys,
            DevCategory::Technical => &self.env_tech,
            DevCategory::Mental => &self.env_ment,
            DevCategory::Goalkeeping => &self.env_gk,
        }
    }

    #[inline]
    fn plast(&self, y: f64) -> f64 {
        1.0 / (1.0 + ((y - self.plast_mid) / self.plast_width).exp())
    }
}

impl Default for DevKnobs {
    fn default() -> Self {
        DevKnobs {
            // §2.1 fitted envelope parameters (g, s, lmax, d, w).
            // Physical re-fit against real `worldgen` (career-arc harness, §6):
            // the scratchpad's (lmax 0.55, d 28.5, w 2.6) put the emergent
            // physical-composite peak at ~27 and the overall CA peak at ~31 —
            // both late — because worldgen seeds players below their target and
            // they climb past the envelope peak. Pulling decline earlier and
            // steeper (d 28.5→27.0, w 2.6→2.3, lmax 0.55→0.60) lands the physical
            // peak at ~26, the CA peak at ~29, and a ~−2.7 CA/yr veteran
            // (30→35) physical slope — the §6 targets.
            env_phys: EnvParams {
                g: 15.0,
                s: 3.0,
                lmax: 0.60,
                d: 27.0,
                w: 2.3,
            },
            env_tech: EnvParams {
                g: 17.5,
                s: 4.5,
                lmax: 0.22,
                d: 31.0,
                w: 3.4,
            },
            env_ment: EnvParams {
                g: 18.5,
                s: 5.0,
                lmax: 0.16,
                d: 32.5,
                w: 3.8,
            },
            env_gk: EnvParams {
                g: 18.0,
                s: 5.0,
                lmax: 0.12,
                d: 34.0,
                w: 4.5,
            },
            k: 0.55,
            // Gentler than the scratchpad's 1.0: `worldgen` seeds veterans
            // *above* their aging envelope (it is not env-consistent), so a
            // proportional pull that fast would crash their physicals ~20 pts in
            // a few seasons. 0.30 gives a believable early-30s decline from a
            // mid-career start; a from-youth env-consistent career declines
            // gentler still. (A harness re-fit target, DEVELOPMENT_MODEL.md §6.)
            k_dec: 0.30,
            // Plasticity window re-fit against real `worldgen` (career-arc
            // harness, §6): the scratchpad's (24.5, 2.5) never closes hard enough
            // — at age 30 `plast` is still ~0.10, so over a decade+ even a
            // low-`E` prospect crawls to PA and *nobody* falls short. Tightening
            // to (23.5, 2.2) freezes an unrealized gap past the mid-20s, giving
            // the real sub-0.80 attainment tail (~11%, p10 ~0.80) the flat
            // notebook cohort produced from-youth. (Hard flops <0.75 stay ~0 on
            // real worldgen — an attainment *floor*, not a knob; see §6.)
            plast_mid: 23.5,
            plast_width: 2.2,
            ceil_spread: 4.5,
            ceil_floor: 8.0,
            e_base: 0.72,
            e_det: 0.011,
            e_prof: 0.008,
            // E spread widened (0.34→0.42) and floor deepened (0.20→0.15) in the
            // real-`worldgen` re-fit (career-arc harness, §6): the scratchpad's
            // narrower spread realized potential too uniformly (wonderkid hit
            // rate ~0.75 vs the ~0.56 target, no shortfall tail). A fatter low
            // tail of `E`, together with the tighter plasticity window above,
            // spreads prospect outcomes into the believable §6 range.
            e_sigma: 0.42,
            e_min: 0.15,
            e_max: 1.9,
            prof_aging_coeff: 0.3,
            phi_sigma: 1.8,
            jitter_sigma: 0.35,
            minutes_regular_share: 0.5,
            minutes_regular: 1.0,
            minutes_rotation: 0.65,
            minutes_absent: 0.3,
            coaching_min: 0.85,
            coaching_max: 1.15,
            coaching_sigma: 0.06,
            max_step: 6,
        }
    }
}

#[inline]
pub(crate) fn cat_index(cat: DevCategory) -> usize {
    match cat {
        DevCategory::Physical => 0,
        DevCategory::Technical => 1,
        DevCategory::Mental => 2,
        DevCategory::Goalkeeping => 3,
    }
}

// A grid scan over the playing-age range; fine and deterministic (same-build).
const GRID_LO: f64 = 15.0;
const GRID_HI: f64 = 40.0;
const GRID_STEP: f64 = 0.05;

/// `NORM` per role (`DEVELOPMENT_MODEL.md` §2.2): the max over age of the
/// role-weighted mean of the category envelopes — "the role-weighted mean of env
/// at its blended peak." A per-role constant given the knobs.
pub(crate) fn norms_by_role(knobs: &DevKnobs) -> [f64; fforge_domain::NUM_ROLES] {
    let mut norms = [0.0f64; fforge_domain::NUM_ROLES];
    for role in fforge_domain::Role::ALL {
        let mut best = 0.0;
        let mut y = GRID_LO;
        while y <= GRID_HI {
            let mut num = 0.0;
            let mut den = 0.0;
            for attr in Attribute::ALL {
                let w = ROLE_WEIGHTS.weight(role, attr) as f64;
                if w > 0.0 {
                    num += w * knobs.env(attr.dev_category()).env(y);
                    den += w;
                }
            }
            let blend = num / den;
            if blend > best {
                best = blend;
            }
            y += GRID_STEP;
        }
        norms[role.index()] = best;
    }
    norms
}

/// Each category's envelope-peak age — past it, the downward (aging) pull acts;
/// before it a precocious youth holds rather than being pulled down (§2, §2.1).
pub(crate) fn category_peaks(knobs: &DevKnobs) -> [f64; NUM_CATEGORIES] {
    let mut peaks = [0.0f64; NUM_CATEGORIES];
    for cat in [
        DevCategory::Physical,
        DevCategory::Technical,
        DevCategory::Mental,
        DevCategory::Goalkeeping,
    ] {
        let p = knobs.env(cat);
        let mut best_y = GRID_LO;
        let mut best_v = p.env(GRID_LO);
        let mut y = GRID_LO;
        while y <= GRID_HI {
            let v = p.env(y);
            if v > best_v {
                best_v = v;
                best_y = y;
            }
            y += GRID_STEP;
        }
        peaks[cat_index(cat)] = best_y;
    }
    peaks
}

/// Per-role ceiling base offset: `pa_base = PA − spread·role_const` makes the
/// role-shaped ceiling's best-role CA equal PA exactly (before clamping).
/// `role_const = Σ w(w−3) / Σ w` over the role's weighted attributes.
pub(crate) fn role_ceiling_consts() -> [f64; fforge_domain::NUM_ROLES] {
    let mut consts = [0.0f64; fforge_domain::NUM_ROLES];
    for role in fforge_domain::Role::ALL {
        let mut num = 0.0;
        let mut den = 0.0;
        for attr in Attribute::ALL {
            let w = ROLE_WEIGHTS.weight(role, attr) as f64;
            if w > 0.0 {
                num += w * (w - 3.0);
                den += w;
            }
        }
        consts[role.index()] = num / den;
    }
    consts
}

/// The §2 growth/aging **rate law** for a single attribute — the shared core
/// that both the recording path (`tick_changes`) and the noise-free projection
/// (`crate::valuation::project_ca`) run, so there is exactly one law and no
/// second integrator to drift (`DEVELOPMENT_MODEL.md` §2.3). Returns the annual
/// rate in attribute-points/year; callers multiply by `dt` and then either
/// quantize it with jitter into the fold's recorded integer step (§5) or
/// accumulate it in float (a projection). Pure — no RNG, no clock. `y` is the
/// bloomer-shifted age (`age − φ`); `phys_lmax` is the professionalism-adjusted
/// physical peak-loss (§3). An attribute the role weights at 0 earns no
/// headroom and returns 0 (§2.2).
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn attr_rate(
    knobs: &DevKnobs,
    role: Role,
    attr: Attribute,
    a_cur: f64,
    norm: f64,
    pa_base: f64,
    e: f64,
    coaching: f64,
    minutes: f64,
    y: f64,
    phys_lmax: f64,
    peaks: &[f64; NUM_CATEGORIES],
) -> f64 {
    let w = ROLE_WEIGHTS.weight(role, attr);
    if w == 0 {
        return 0.0; // attributes the role does not value earn no headroom (§2.2)
    }
    let cat = attr.dev_category();
    let e_env = if cat == DevCategory::Physical {
        knobs.env_phys.env_lmax(y, phys_lmax)
    } else {
        knobs.env(cat).env(y)
    };
    let ceiling = (pa_base + (w as f64 - 3.0) * knobs.ceil_spread).clamp(knobs.ceil_floor, 100.0);
    let target = (ceiling / norm) * e_env;
    let gap = target - a_cur;
    if gap > 0.0 {
        knobs.k * e * knobs.plast(y) * coaching * minutes * gap
    } else if y >= peaks[cat_index(cat)] {
        knobs.k_dec * gap // aging decline (gap < 0)
    } else {
        0.0 // precocious youth above the young-envelope: hold (§2.1)
    }
}

/// The coarse appeared/benched/absent playing-time multiplier (§3), from the
/// player's appearances vs their club's matches in the tick's window.
fn minutes_multiplier(
    pid: PlayerId,
    club: ClubId,
    appearances: &BTreeMap<PlayerId, u32>,
    club_matches: &BTreeMap<ClubId, u32>,
    knobs: &DevKnobs,
) -> f64 {
    let matches = club_matches.get(&club).copied().unwrap_or(0);
    if matches == 0 {
        return knobs.minutes_absent; // no matches in window (e.g. offseason)
    }
    let apps = appearances.get(&pid).copied().unwrap_or(0);
    if apps == 0 {
        return knobs.minutes_absent;
    }
    if apps as f64 / matches as f64 >= knobs.minutes_regular_share {
        knobs.minutes_regular
    } else {
        knobs.minutes_rotation
    }
}

/// Resolve a player's once-only development trajectory (§2.3) at worldgen from
/// their character + seeded noise. Called from `worldgen`; the result is stored
/// in `Player::development` and recorded in the `World` snapshot — never
/// re-derived.
pub fn resolve_dev_profile(
    rng: &mut Rng,
    determination: u8,
    professionalism: u8,
    knobs: &DevKnobs,
) -> fforge_domain::DevProfile {
    let mean = knobs.e_base
        + knobs.e_det * (determination as f64 - 50.0)
        + knobs.e_prof * (professionalism as f64 - 50.0);
    let e = rng
        .normal(mean, knobs.e_sigma)
        .clamp(knobs.e_min, knobs.e_max);
    let phi = rng.normal(0.0, knobs.phi_sigma).clamp(-6.0, 6.0);
    fforge_domain::DevProfile {
        efficiency_milli: (e * 1000.0).round() as u16,
        bloomer_phase_centi: (phi * 100.0).round() as i16,
    }
}

/// Resolve a club's once-only coaching coefficient (§3) at worldgen: loosely
/// tied to club quality (better clubs, better academies) plus seeded noise.
pub fn resolve_coaching(rng: &mut Rng, quality: f64, knobs: &DevKnobs) -> u16 {
    // quality anchors span ~48..74 in worldgen; map to 0..1 across the league.
    let span = ((quality - 48.0) / 26.0).clamp(0.0, 1.0);
    let base = knobs.coaching_min + (knobs.coaching_max - knobs.coaching_min) * span;
    let c = rng
        .normal(base, knobs.coaching_sigma)
        .clamp(knobs.coaching_min, knobs.coaching_max);
    (c * 1000.0).round() as u16
}

/// Apply one recorded integer step to the world — pure integer add, clamped to
/// 0..=100. Shared by the fold (`GameState::apply`) and the in-`step` working
/// copy that lets successive offseason ticks compound.
#[inline]
pub fn apply_attr_step(world: &mut World, step: &AttrStep) {
    if let Some(p) = world.players.get_mut(&step.player) {
        let cur = p.attributes.get(step.attr) as i32;
        let nv = (cur + step.delta as i32).clamp(0, 100) as u8;
        p.attributes.set(step.attr, nv);
    }
}

/// Produce one development tick's resolved changes (§2, §5): the sparse set of
/// integer attribute steps that crossed a boundary this month. Reads world
/// attributes + the once-resolved per-player `E`/`φ` + coaching + the window's
/// appearances; draws jitter + a Bernoulli fractional step per attribute from
/// the tick's own seed stream, in `(player id, attribute)` order so replay of
/// the recorded deltas is exact and same-seed runs are identical.
pub fn tick_changes(
    world: &World,
    seed: u64,
    period: i64,
    tick_date: GameDate,
    appearances: &BTreeMap<PlayerId, u32>,
    club_matches: &BTreeMap<ClubId, u32>,
    knobs: &DevKnobs,
) -> Vec<AttrStep> {
    let mut rng = derive_stream(seed, DEV_STREAM_NS | (period as u64));
    let norms = norms_by_role(knobs);
    let peaks = category_peaks(knobs);
    let ceiling_consts = role_ceiling_consts();
    let dt = knobs.dt();

    // player -> club, for coaching and the window's match count.
    let mut player_club: BTreeMap<PlayerId, ClubId> = BTreeMap::new();
    for (&cid, club) in &world.clubs {
        for &pid in &club.players {
            player_club.insert(pid, cid);
        }
    }

    let mut changes = Vec::new();
    // BTreeMap iteration is id order — the determinism the fold relies on.
    for (&pid, player) in &world.players {
        let e = player.development.efficiency();
        let phi = player.development.bloomer_phase();
        let (role, _) = best_role(&player.attributes, &ROLE_WEIGHTS);
        let norm = norms[role.index()];
        let pa = player.character.potential as f64;
        let pa_base = pa - knobs.ceil_spread * ceiling_consts[role.index()];

        let (coaching, mult) = match player_club.get(&pid) {
            Some(&club) => (
                world.club(club).coaching(),
                minutes_multiplier(pid, club, appearances, club_matches, knobs),
            ),
            None => (1.0, knobs.minutes_absent),
        };

        let age = (tick_date.days - player.birth.days) as f64 / DAYS_PER_YEAR;
        let y = age - phi; // envelope/plasticity act in bloomer-shifted age
        // Professionalism flattens physical aging (§3): the pro ages well.
        let phys_lmax = knobs.env_phys.lmax
            * (1.0 - knobs.prof_aging_coeff * (player.character.professionalism as f64 - 50.0) / 50.0);

        for attr in Attribute::ALL {
            // Draw unconditionally so stream position is value-independent —
            // robust determinism regardless of which attributes step.
            let jitter = rng.normal(0.0, knobs.jitter_sigma);
            let u = rng.f64();

            if ROLE_WEIGHTS.weight(role, attr) == 0 {
                continue; // attributes the role does not value earn no headroom (§2.2)
            }

            let a_cur = player.attributes.get(attr) as f64;
            // The shared §2 law — identical to the projection's, jitter added
            // only here (the recording path); see `attr_rate`.
            let rate = attr_rate(
                knobs, role, attr, a_cur, norm, pa_base, e, coaching, mult, y, phys_lmax, &peaks,
            );

            let monthly = (rate + jitter) * dt;
            let mag = monthly.abs();
            let whole = mag.floor();
            let frac = mag - whole;
            let mut steps = whole as i64 + if u < frac { 1 } else { 0 };
            if steps > knobs.max_step as i64 {
                steps = knobs.max_step as i64;
            }
            let raw = if monthly >= 0.0 { steps } else { -steps };
            if raw == 0 {
                continue;
            }
            // Record the *effective* delta after the 0..=100 clamp, so the log
            // never claims a change that the fold won't make.
            let new_v = (a_cur as i64 + raw).clamp(0, 100);
            let eff = new_v - a_cur as i64;
            if eff != 0 {
                changes.push(AttrStep {
                    player: pid,
                    attr,
                    delta: eff as i8,
                });
            }
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_peaks_land_where_the_doc_says() {
        // §2.1 / §6: physical peaks mid-20s, technical late-20s, mental early-30s.
        let k = DevKnobs::default();
        let peaks = category_peaks(&k);
        let phys = peaks[cat_index(DevCategory::Physical)];
        let tech = peaks[cat_index(DevCategory::Technical)];
        let ment = peaks[cat_index(DevCategory::Mental)];
        assert!((22.0..=26.0).contains(&phys), "physical peak {phys}");
        assert!((27.0..=31.0).contains(&tech), "technical peak {tech}");
        assert!((30.0..=34.0).contains(&ment), "mental peak {ment}");
        assert!(phys < tech && tech < ment, "peak ordering phys<tech<ment");
    }

    #[test]
    fn norms_are_positive_and_bounded() {
        let norms = norms_by_role(&DevKnobs::default());
        for n in norms {
            assert!(n > 0.0 && n <= 1.0, "norm {n} out of (0,1]");
        }
    }

    /// `TRANSFER_MODEL.md` §8.3: a released, unsigned player has no club, so
    /// `tick_changes` cannot look up a coaching coefficient for him — the
    /// live panic waiting in the fold once P4.5 can release players. Confirm
    /// the neutral-coaching path both runs without panicking and produces
    /// **exactly** the same deltas as an otherwise-identical rostered player
    /// at a neutral-coaching (`coaching_milli = 1000`) club who is likewise
    /// absent from every match this window: coaching and the playing-time
    /// multiplier are the only club-derived inputs to the rate law, and both
    /// resolve identically (`coaching = 1.0`, `mult = minutes_absent`) in the
    /// two cases.
    #[test]
    fn clubless_player_develops_without_panicking_using_neutral_coaching() {
        let (world, _schedule, start) =
            crate::worldgen::generate(3, &crate::worldgen::WorldGenConfig::default());
        let knobs = DevKnobs::default();

        let club = *world.clubs.keys().next().unwrap();
        let mut world_rostered = world.clone();
        world_rostered.clubs.get_mut(&club).unwrap().coaching_milli = 1000;
        let pid = world_rostered.club(club).players[0];

        let mut world_clubless = world_rostered.clone();
        world_clubless
            .clubs
            .get_mut(&club)
            .unwrap()
            .players
            .retain(|&p| p != pid);

        let empty_apps: BTreeMap<PlayerId, u32> = BTreeMap::new();
        let empty_matches: BTreeMap<ClubId, u32> = BTreeMap::new();

        // No panic reaching past these two calls is itself the primary
        // assertion (§8.3): `world_clubless` has `pid` in `World.players`
        // but on no club's roster at all.
        let rostered_changes =
            tick_changes(&world_rostered, 1, 0, start, &empty_apps, &empty_matches, &knobs);
        let clubless_changes =
            tick_changes(&world_clubless, 1, 0, start, &empty_apps, &empty_matches, &knobs);

        assert!(
            !rostered_changes.is_empty(),
            "sanity: development should produce some deltas in this window"
        );
        assert_eq!(
            rostered_changes, clubless_changes,
            "a clubless player must develop identically to a rostered, \
             neutral-coaching, likewise-absent player — the coaching=1.0 \
             fallback must be exactly neutral, not merely non-panicking"
        );
    }
}
