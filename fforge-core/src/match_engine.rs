//! The Phase-1 **crude** match engine (DESIGN.md §9, Phase 1).
//!
//! Deliberately shallow: XI strength → Poisson goal counts. It exists so the
//! whole loop runs end-to-end; Phase 2 replaces it with the event-based
//! possession model behind the same call site. What it *does* already
//! exercise for real: the role-weighting table (a striker picked at CB rates
//! as a CB and drags the team down) and derived per-fixture RNG streams.
//!
//! Baseline constants target believable aggregates (≈2.6 goals/game, visible
//! home advantage) but are eyeballed, not calibrated — the calibration
//! harness is a Phase 2 deliverable alongside the real engine.

use crate::rng::Rng;
use fforge_domain::{
    current_ability, ClubId, Lineup, PlayerId, Role, World, FORMATIONS, ROLE_WEIGHTS, XI,
};

/// Mean CA-in-slot-role over the eleven. Playing out of position is legal and
/// simply rates at that role's CA — the weighting table is the whole penalty
/// model, no extra fudge factor.
pub fn lineup_strength(world: &World, lineup: &Lineup) -> f64 {
    let def = lineup.formation_def();
    let mut sum = 0.0;
    for (slot, &pid) in lineup.players.iter().enumerate() {
        let player = world.player(pid);
        sum += current_ability(&player.attributes, def.slots[slot], &ROLE_WEIGHTS) as f64;
    }
    sum / XI as f64
}

/// Home λ base > away λ base ⇒ home advantage; totals ≈ 2.6.
const HOME_BASE: f64 = 1.42;
const AWAY_BASE: f64 = 1.12;
/// Sensitivity per rating point of strength difference.
const DIFF_K: f64 = 0.030;

pub fn simulate_match(home_strength: f64, away_strength: f64, rng: &mut Rng) -> (u8, u8) {
    let diff = home_strength - away_strength;
    let lambda_home = (HOME_BASE * (DIFF_K * diff).exp()).clamp(0.15, 6.0);
    let lambda_away = (AWAY_BASE * (-DIFF_K * diff).exp()).clamp(0.10, 6.0);
    let hg = rng.poisson(lambda_home).min(9) as u8;
    let ag = rng.poisson(lambda_away).min(9) as u8;
    (hg, ag)
}

/// Deterministic AI team selection: for each formation, greedily fill slots
/// with the best remaining player by CA-in-slot-role (ties → lower player
/// id); keep the formation with the best mean. This is the Phase-1 stub of
/// the layer-3 club decision AI — same seam, richer policy later.
pub fn ai_pick_lineup(world: &World, club: ClubId) -> Lineup {
    let squad: Vec<PlayerId> = world.club(club).players.clone();
    let mut best: Option<(f64, Lineup)> = None;

    for (fi, formation) in FORMATIONS.iter().enumerate() {
        let mut remaining = squad.clone();
        let mut chosen = [PlayerId(0); XI];
        let mut total = 0.0;
        for (slot, &role) in formation.slots.iter().enumerate() {
            let (idx, ca) = pick_best(world, &remaining, role);
            chosen[slot] = remaining.remove(idx);
            total += ca as f64;
        }
        let mean = total / XI as f64;
        let candidate = Lineup {
            formation: fi as u8,
            players: chosen,
        };
        match &best {
            Some((score, _)) if *score >= mean => {}
            _ => best = Some((mean, candidate)),
        }
    }
    best.expect("at least one formation").1
}

fn pick_best(world: &World, pool: &[PlayerId], role: Role) -> (usize, u8) {
    let mut best_idx = 0;
    let mut best_ca = 0u8;
    let mut best_id = PlayerId(u32::MAX);
    for (i, &pid) in pool.iter().enumerate() {
        let ca = current_ability(&world.player(pid).attributes, role, &ROLE_WEIGHTS);
        if ca > best_ca || (ca == best_ca && pid < best_id) {
            best_idx = i;
            best_ca = ca;
            best_id = pid;
        }
    }
    (best_idx, best_ca)
}