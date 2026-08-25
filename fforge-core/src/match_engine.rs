//! The Phase-2a event-based possession match engine (`MATCH_MODEL.md`),
//! behind the same `play_match` call site the Phase-1 crude engine used to
//! occupy. State space, resolution model, the wide route, and the knob
//! table are a faithful Rust port of the calibrated Python prototype
//! (`match_model_prototype.ipynb`, referenced from `MATCH_MODEL.md` §1) —
//! nothing here is a re-guess of the shape-finding, only its translation.
//!
//! Phase 2e has begun: tactics (`TACTICS_MODEL.md`) lands as transition-
//! matrix modifiers behind this same call site (no structural change).
//! Still deferred: cards & fouls, injuries, set pieces, substitutions, and
//! the character/hidden attributes.

mod calibrate;
mod contest;
mod knobs;
mod ratings;
mod resolve;
mod stream;
mod tactics;
mod zone;

pub use calibrate::{
    DeviationReport, ELO_SCALE_S, FormationStats, GapBinStats, GapDeviation, PROFILE_SHIFT,
    ProfileRow, SQUAD_PROFILES, SquadProfile, StreamTelemetry, apply_squad_profile, elo_expected,
    probe_tactics, run_head_to_head, run_head_to_head_detailed, run_squad_conditional_probe,
};
pub use knobs::Knobs;
pub use ratings::man_of_the_match;
/// The fixed in-match decision points a `sub_plan` is evaluated at
/// (`MATCH_MODEL.md` §16), alongside half-time and any tick a new injury or
/// card fires on. Public because a plan editor that does not tell the manager
/// *when* his rules get their chance is showing him half the mechanism: a
/// `MinuteAtLeast(65)` rule fires at 70, not 65.
pub use resolve::SUB_CHECKPOINTS;
pub use stream::{MatchEvent, MatchEventKind, ShotKind, ShotOutcome, ShotSource, Side};
pub use zone::Zone;

use crate::rng::Rng;
use fforge_domain::{
    Attribute, BENCH_SIZE, ClubId, FORMATIONS, FormationDef, GameDate, Lineup, Mentality, PlayerId,
    Pressing, ROLE_WEIGHTS, Role, ScoreState, SubAction, SubCondition, SubRule, Tactics, Tempo,
    Width, World, XI, current_ability,
};
use serde::{Deserialize, Serialize};

/// Tag namespace for the per-match Consistency RNG stream
/// (`rng::derive_stream`, `MATCH_MODEL.md` §17, T8) — distinct from
/// `commands::FIXTURE_STREAM_NS`, `development::DEV_STREAM_NS`, and
/// `market::TRANSFER_STREAM_NS`, per §2.1's own-stream rule. Callers OR in a
/// per-fixture component (e.g. `CONSISTENCY_NS | fixture.id`) the same way
/// the main stream does.
pub const CONSISTENCY_NS: u64 = 0x434F_4E53_0000_0000; // "CONS"

/// Tag namespace for the per-match Injury RNG stream (`MATCH_MODEL.md` §14,
/// T10) — the "INJU" namespace §2.1 reserved, distinct from every other
/// stream so `injury_rate: 0.0` leaves both the main and Consistency streams'
/// draw sequences untouched.
pub const INJURY_NS: u64 = 0x494E_4A55_0000_0000; // "INJU"

/// Tag namespace for the per-match Foul RNG stream (`MATCH_MODEL.md` §15,
/// T11) — a fourth stream, distinct from `rng`/`consistency_rng`/
/// `injury_rng`, so `foul_rate: 0.0` leaves every other stream's draw
/// sequence untouched.
pub const FOUL_NS: u64 = 0x464F_554C_0000_0000; // "FOUL"

/// A resolved injury (`MATCH_MODEL.md` §12, §14): the *days out*, decided at
/// match time — never a severity category for the fold to re-roll, so the
/// severity model can evolve without rewriting anyone's recorded medical
/// history (the `DevelopmentTick` argument verbatim).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InjuryOutcome {
    pub player: PlayerId,
    /// Days unavailable, counted from the match date. The fold turns this
    /// into `Player.injured_until`.
    pub days_out: u16,
}

/// The card itself (`MATCH_MODEL.md` §15). A second yellow is recorded as
/// `SecondYellow` — a red by bookkeeping — so no consumer ever has to
/// reconstruct the distinction from minute ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Card {
    Yellow,
    SecondYellow,
    Red,
}

/// A resolved card (`MATCH_MODEL.md` §12, §15): the recorded truth from which
/// suspensions are *derived* in the fold — a ban is never stored and never its
/// own event (the derived-suspension rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardOutcome {
    pub player: PlayerId,
    pub card: Card,
    pub minute: u8,
}

/// Mean CA-in-slot-role over the eleven — a squad-quality scalar independent
/// of any particular match-resolution model. Used for display and by
/// `ai_pick_lineup`'s formation comparison below.
pub fn lineup_strength(world: &World, lineup: &Lineup) -> f64 {
    let def = lineup.formation_def();
    let mut sum = 0.0;
    for (slot, &pid) in lineup.players.iter().enumerate() {
        let player = world.player(pid);
        sum += current_ability(&player.attributes, def.slots[slot], &ROLE_WEIGHTS) as f64;
    }
    sum / XI as f64
}

/// The result of a simulated match: the score that folds into `GameState`
/// (via `Event::MatchPlayed`) plus the minute-by-minute trace. The trace
/// rides alongside the fold, never inside it (`MATCH_MODEL.md` §7) — it is a
/// Trace, not a fold input, and callers are free to discard it. Nothing here
/// is persisted by `commands::advance_matchday`; only the score is.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchOutcome {
    pub home_goals: u8,
    pub away_goals: u8,
    pub stream: Vec<MatchEvent>,
    /// Resolved per-player consequences that outlive the match
    /// (`MATCH_MODEL.md` §12): unlike `stream`, these *do* ride into
    /// `Event::MatchPlayed`. The boundary was grown once, ahead of the models
    /// that fill it; `injuries` is now populated by the §14 model (T10, own
    /// `INJURY_NS` stream, identity `injury_rate: 0.0`). `cards`/`ratings`
    /// still emit empty until the §15 foul/card contest and §18 rating
    /// derivation land, so nothing there touches the RNG draw sequence yet.
    pub injuries: Vec<InjuryOutcome>,
    pub cards: Vec<CardOutcome>,
    /// Per-player rating in tenths (`68` = 6.8), `MATCH_MODEL.md` §18.
    pub ratings: Vec<(PlayerId, u8)>,
    /// True minutes played, substitutions included (`MATCH_MODEL.md` §12,
    /// §16, R7). Every starting-XI player at 90 until T10/T11/T12 (injuries,
    /// red cards, substitutions) make partial minutes possible — there is no
    /// bench yet, so "0 otherwise" is simply absence from this vec, not a
    /// recorded entry.
    pub minutes: Vec<(PlayerId, u8)>,
}

/// Simulate one match: `(lineups, world, rng, consistency_rng, knobs,
/// conditions)` in, score + trace out. A pure function of its inputs — same
/// seed streams, same outcome, by construction (`MATCH_MODEL.md` §7).
/// `consistency_rng` must be a stream independent of `rng` (`MATCH_MODEL.md`
/// §17, T8) — callers typically derive it as `derive_stream(seed,
/// CONSISTENCY_NS | fixture.id)`, the sibling of however `rng` itself was
/// derived. `k` is almost always `&Knobs::default()`; tests that need to pin
/// a knob independent of the production default (the T5/T6 identity tests
/// pinning `consistency_sigma_max: 0.0`) pass their own. `conditions`
/// (`MATCH_MODEL.md` §13, T9) is a pre-computed `PlayerId -> condition` map —
/// typically `GameState::condition` for every player in both lineups; an
/// empty map (or a missing entry) is full condition, the identity setting.
/// `injury_rng` (`MATCH_MODEL.md` §14, T10) is a third stream, independent of
/// both `rng` and `consistency_rng`, typically derived as `derive_stream(seed,
/// INJURY_NS | fixture.id)` — at `k.injury_rate == 0.0` (§2.1's identity) it
/// is still drawn from, just never produces an injury. `foul_rng`
/// (`MATCH_MODEL.md` §15, T11) is a fourth stream, independent of the other
/// three, typically derived as `derive_stream(seed, FOUL_NS | fixture.id)` —
/// at `k.foul_rate == 0.0` it is still drawn from, just never produces a
/// foul. `today` is only used to compute each player's age for the ambient
/// injury channel.
#[allow(clippy::too_many_arguments)]
pub fn play_match(
    world: &World,
    home: &Lineup,
    away: &Lineup,
    rng: &mut Rng,
    consistency_rng: &mut Rng,
    injury_rng: &mut Rng,
    foul_rng: &mut Rng,
    k: &Knobs,
    conditions: &std::collections::BTreeMap<PlayerId, f64>,
    today: GameDate,
) -> MatchOutcome {
    resolve::play_match(
        world,
        home,
        away,
        rng,
        consistency_rng,
        injury_rng,
        foul_rng,
        k,
        conditions,
        today,
    )
}

/// Deterministic AI team selection: for each formation, greedily fill slots
/// with the best remaining player by CA-in-slot-role (ties → lower player
/// id); keep the formation with the best mean. This is the Phase-1 stub of
/// the layer-3 club decision AI — same seam, richer policy later.
pub fn ai_pick_lineup(world: &World, club: ClubId) -> Lineup {
    pick_lineup_from(world, world.club(club).players.clone())
}

/// `ai_pick_lineup`, but filtered to players available as of `today`
/// (`MATCH_MODEL.md` §12, §14, T10): "squad depth finally bites" (§2.5) —
/// an injured player is invisible to selection rather than merely losing
/// out on CA. `suspended` (`MATCH_MODEL.md` §15, T11) is the derived,
/// currently-banned set — `GameState::suspended_players()` for real
/// callers, empty for harnesses with no season to derive it from. Falls
/// back to the unfiltered squad if fewer than `XI` players are available (a
/// defensive floor; the transfer market's own `[18, 30]` squad-size
/// stabilizer and injuries'/cards' plausible-band targets should make this
/// unreachable in practice, but a real bug elsewhere should never turn into
/// a panic here).
pub fn ai_pick_lineup_available(
    world: &World,
    club: ClubId,
    today: GameDate,
    suspended: &std::collections::BTreeSet<PlayerId>,
) -> Lineup {
    let squad = &world.club(club).players;
    let available: Vec<PlayerId> = squad
        .iter()
        .copied()
        .filter(|&pid| {
            let injury_ok = match world.player(pid).injured_until {
                Some(until) => until <= today,
                None => true,
            };
            injury_ok && !suspended.contains(&pid)
        })
        .collect();
    if available.len() >= XI {
        pick_lineup_from(world, available)
    } else {
        pick_lineup_from(world, squad.clone())
    }
}

/// The shared greedy-fill selection both `ai_pick_lineup` and
/// `ai_pick_lineup_available` run, over whichever pool of candidates the
/// caller has already decided is eligible.
fn pick_lineup_from(world: &World, squad: Vec<PlayerId>) -> Lineup {
    let mut best: Option<(f64, u8, [PlayerId; XI], Vec<PlayerId>)> = None;

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
        match &best {
            Some((score, ..)) if *score >= mean => {}
            _ => best = Some((mean, fi as u8, chosen, remaining)),
        }
    }
    let (_, fi, chosen, remaining) = best.expect("at least one formation");
    let (bench, sub_plan) =
        ai_pick_bench_and_plan(world, &FORMATIONS[fi as usize], &chosen, remaining);
    Lineup {
        formation: fi,
        players: chosen,
        // Neutral here; ai_pick_tactics (below) is the sibling that
        // fills this in against a known opponent — kept separate since
        // this function doesn't know the opponent.
        tactics: Tactics::neutral(),
        bench,
        sub_plan,
    }
}

/// The v1 AI bench-selection and default-substitution-plan policy
/// (`MATCH_MODEL.md` §16, "The v1 AI bench-selection and default-
/// substitution-plan policy"): a pure `(World, formation, XI, remaining) ->
/// (bench, plan)` map, called once by `pick_lineup_from` after the winning
/// formation is chosen — not once per candidate formation, which would
/// compute four bench/plan pairs to throw three away. Zero RNG: every
/// quantity read (CA-in-role, Stamina, Work Rate, `Knobs::fatigue_wr`) is
/// already-resolved `World` state, so this runs entirely outside
/// `play_match` and none of its four in-match streams are touched.
fn ai_pick_bench_and_plan(
    world: &World,
    formation: &FormationDef,
    xi: &[PlayerId; XI],
    mut remaining: Vec<PlayerId>,
) -> (Vec<PlayerId>, Vec<SubRule>) {
    // --- 1/2. Bench composition & selection ---
    //
    // `bench[i]`'s entry in `covered_role` is the role it was picked *for*
    // (via `pick_best`, the same CA-in-role query the XI itself was chosen
    // with) — `Some(Gk)` for the reserved backup keeper, `Some(role)` for a
    // first-pass role-spread pick, `None` for a second-pass depth pick with
    // no specific role assignment. This tag, not natural role, is what the
    // default plan below pairs a starter against — reusing `pick_best`
    // "unchanged" per the design note's item 2.
    let mut bench: Vec<PlayerId> = Vec::new();
    let mut covered_role: Vec<Option<Role>> = Vec::new();

    // Mandatory backup GK, reserved ahead of every other slot. Never a
    // panic if `remaining` is somehow empty — the same defensive-floor
    // discipline `ai_pick_lineup_available`'s own fallback uses.
    if !remaining.is_empty() {
        let (idx, _) = pick_best(world, &remaining, Role::Gk);
        bench.push(remaining.remove(idx));
        covered_role.push(Some(Role::Gk));
    }

    // First pass: spread across the outfield roles, one bench slot per
    // role not yet covered, best-CA-in-role among what's left.
    for &role in Role::ALL.iter().filter(|&&r| r != Role::Gk) {
        if bench.len() >= BENCH_SIZE || remaining.is_empty() {
            break;
        }
        let (idx, _) = pick_best(world, &remaining, role);
        bench.push(remaining.remove(idx));
        covered_role.push(Some(role));
    }

    // Second pass: best-of-the-rest by CA-in-natural-role, pure squad
    // depth — no role tag, so no forced-cover rule ever pairs against one
    // of these (an honest gap, not a padded one).
    while bench.len() < BENCH_SIZE && !remaining.is_empty() {
        let mut best_idx = 0;
        let mut best_ca = 0u8;
        let mut best_id = PlayerId(u32::MAX);
        for (i, &pid) in remaining.iter().enumerate() {
            let player = world.player(pid);
            let ca = current_ability(&player.attributes, player.natural_role, &ROLE_WEIGHTS);
            if ca > best_ca || (ca == best_ca && pid < best_id) {
                best_idx = i;
                best_ca = ca;
                best_id = pid;
            }
        }
        bench.push(remaining.remove(best_idx));
        covered_role.push(None);
    }

    // --- 3. The default plan ---
    let mut plan: Vec<SubRule> = Vec::new();

    // Forced-cover: one PlayerInjured rule per formation-slot role that has
    // both a fielded starter and a bench player tagged for that role. The
    // weakest-CA-in-role starter is paired with the strongest-CA-in-role
    // tagged backup — freeing the strongest starter in that role to keep
    // playing, per the design note's "protect the weakest starter" rule.
    let mut forced_pairs: Vec<(PlayerId, PlayerId)> = Vec::new();
    let mut seen_roles: Vec<Role> = Vec::new();
    for &role in formation.slots.iter() {
        if seen_roles.contains(&role) {
            continue;
        }
        seen_roles.push(role);

        let mut weakest: Option<(PlayerId, u8)> = None;
        for (i, &r) in formation.slots.iter().enumerate() {
            if r != role {
                continue;
            }
            let pid = xi[i];
            let ca = current_ability(&world.player(pid).attributes, role, &ROLE_WEIGHTS);
            weakest = Some(match weakest {
                Some((wp, wca)) if wca < ca || (wca == ca && wp < pid) => (wp, wca),
                _ => (pid, ca),
            });
        }
        let Some((weakest_pid, _)) = weakest else {
            continue;
        };

        let mut backup: Option<(PlayerId, u8)> = None;
        for (i, &pid) in bench.iter().enumerate() {
            if covered_role[i] != Some(role) {
                continue;
            }
            let ca = current_ability(&world.player(pid).attributes, role, &ROLE_WEIGHTS);
            backup = Some(match backup {
                Some((bp, bca)) if bca > ca || (bca == ca && bp < pid) => (bp, bca),
                _ => (pid, ca),
            });
        }
        let Some((backup_pid, _)) = backup else {
            continue;
        };

        forced_pairs.push((weakest_pid, backup_pid));
        plan.push(SubRule {
            conditions: vec![SubCondition::PlayerInjured(weakest_pid)],
            action: SubAction::Substitute {
                player_out: weakest_pid,
                player_in: backup_pid,
            },
        });
    }

    // Fatigue: the two outfield starters with a forced-cover pair, ranked
    // by `contest::fatigue_mult`'s own non-time, non-press factors — the
    // same attribute-derived-score style `ai_pick_tactics` already uses.
    // Threshold `80`: `PlayerConditionBelow` reads `fatigue_mult`'s live
    // value, which never drops below a `0.7` floor even before the
    // pre-match condition factor multiplies in, so the reachable range in
    // a real match sits roughly `60-100` — a naive "below 60" would almost
    // never fire.
    let fatigue_wr = Knobs::default().fatigue_wr;
    let mut proneness: Vec<(PlayerId, f64)> = Vec::new();
    for (i, &role) in formation.slots.iter().enumerate() {
        if role == Role::Gk {
            continue;
        }
        let pid = xi[i];
        if !forced_pairs.iter().any(|&(s, _)| s == pid) {
            continue;
        }
        let attrs = &world.player(pid).attributes;
        let stamina = attrs.get(Attribute::Stamina) as f64 / 100.0;
        let work_rate = attrs.get(Attribute::WorkRate) as f64 / 100.0;
        proneness.push((pid, (1.0 - stamina) * (1.0 + fatigue_wr * work_rate)));
    }
    proneness.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for &(pid, _) in proneness.iter().take(2) {
        let backup = forced_pairs
            .iter()
            .find(|&&(s, _)| s == pid)
            .expect("fatigue candidates are pre-filtered to have a forced-cover pair")
            .1;
        plan.push(SubRule {
            conditions: vec![
                SubCondition::MinuteAtLeast(70),
                SubCondition::PlayerConditionBelow(pid, 80),
            ],
            action: SubAction::Substitute {
                player_out: pid,
                player_in: backup,
            },
        });
    }

    // Chase/hold: tactics only (§16's own vocabulary finding — a
    // score-conditioned player pick isn't expressible). Costs nothing
    // against the substitution cap.
    plan.push(SubRule {
        conditions: vec![
            SubCondition::Score(ScoreState::Trailing),
            SubCondition::MinuteAtLeast(70),
        ],
        action: SubAction::SetMentality(Mentality::Attacking),
    });
    plan.push(SubRule {
        conditions: vec![
            SubCondition::Score(ScoreState::Leading),
            SubCondition::MinuteAtLeast(75),
        ],
        action: SubAction::SetMentality(Mentality::Defensive),
    });

    (bench, plan)
}

/// The AI tactics policy is gated off by default. `ai_pick_lineup_vs` (below)
/// reads this, so every real AI-controlled match runs `Tactics::neutral()` —
/// which by the `TACTICS_MODEL.md` §4 invariant reproduces the T5 golden
/// baseline bit-for-bit — letting each 2e feature measure a clean
/// single-feature delta against a stable reference.
///
/// **Now `true` (T7-R2).** Both blockers are cleared. `TACTICS_MODEL.md` §9
/// item 6 resolved in favour of squad-conditional non-dominance (Tempo and
/// Pressing re-fitted so no tactic dominates); §9 item 7 then resolved the
/// Mentality axis, which the first re-fit had exposed as the remaining
/// dominant instruction — `Attacking` beat `Balanced` 0.540/0.460 because
/// the axis had two advance-class gains and no advance-class risk term at
/// all. It now has one, and every forced matchup on the axis reads within
/// ±0.3pt of even while §8's goal-expectation predictions land in band for
/// the first time.
///
/// With no instruction dominating, an AI side picking tactics is finally a
/// decision rather than a free upgrade, so the policy is live: every
/// AI-controlled match now runs `ai_pick_tactics`'s choice.
/// `Tactics::neutral()` remains bit-identical to the T5 golden baseline by
/// the §4 invariant, so the identity tests are unaffected by the flip — only
/// the pooled league aggregates move, and those are re-banked in §8.
pub const AI_TACTICS_ENABLED: bool = true;

/// An AI-controlled side's lineup *and* tactics for a real fixture
/// (`TACTICS_MODEL.md` §7): `ai_pick_lineup_available`'s XI (`today` gates
/// injury availability, T10; `suspended` gates card-derived bans, T11),
/// with `ai_pick_tactics`'s choice against the named opponent applied only
/// while `AI_TACTICS_ENABLED` is `true`. The call-site convenience every
/// real AI-vs-AI (and AI-vs-human) match uses.
pub fn ai_pick_lineup_vs(
    world: &World,
    club: ClubId,
    opponent: ClubId,
    is_home: bool,
    today: GameDate,
    suspended: &std::collections::BTreeSet<PlayerId>,
) -> Lineup {
    let mut lineup = ai_pick_lineup_available(world, club, today, suspended);
    if AI_TACTICS_ENABLED {
        lineup.tactics = ai_pick_tactics(world, club, opponent, is_home, &AiTacticKnobs::default());
    }
    lineup
}

/// Thresholds for `ai_pick_tactics` (`TACTICS_MODEL.md` §7) — plausibility-
/// picked from real `worldgen` + `ai_pick_lineup` percentiles (roughly the
/// 25th/75th split on each signal, so about half of matches land Balanced
/// and a quarter each way), the `ValueKnobs`/`Knobs` discipline: a fit
/// target for the calibration harness, not a finished calibration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiTacticKnobs {
    /// Mentality: `|own − opponent| lineup_strength` beyond this →
    /// Attacking (favourite) / Defensive (underdog); else Balanced.
    pub mentality_strength_gap: f64,
    /// Width: `wide_presence_share` of the chosen XI above/below these →
    /// Wide / Narrow; else Balanced.
    pub width_wide_share: f64,
    pub width_narrow_share: f64,
    /// Tempo: team `PASS_ATK` mean above/below these → Patient / Direct;
    /// else Balanced.
    pub tempo_patient_pass_atk: f64,
    pub tempo_direct_pass_atk: f64,
    /// Pressing: team Work-Rate + Stamina mean above/below these → High /
    /// Deep; else Balanced.
    pub pressing_high_legs: f64,
    pub pressing_deep_legs: f64,
}

impl Default for AiTacticKnobs {
    fn default() -> Self {
        AiTacticKnobs {
            mentality_strength_gap: 8.0,
            width_wide_share: 0.555,
            width_narrow_share: 0.48,
            tempo_patient_pass_atk: 71.0,
            tempo_direct_pass_atk: 58.0,
            pressing_high_legs: 144.0,
            pressing_deep_legs: 117.0,
        }
    }
}

/// Deterministic AI tactics policy (`TACTICS_MODEL.md` §7) — the tactics
/// sibling of `ai_pick_lineup`: same seam ("Phase-1-style stub of the
/// layer-3 decision AI"), richer policy later (Phase 5). RNG-free.
///
/// Deliberately opponent-blind beyond the strength gap: real counter-
/// picking (reading the opponent's likely tactics and choosing the §5
/// counter) is a decision-*quality* behaviour Phase 5's ablation measures —
/// building it into the v1 baseline would flatten that ablation.
pub fn ai_pick_tactics(
    world: &World,
    club: ClubId,
    opponent: ClubId,
    is_home: bool,
    k: &AiTacticKnobs,
) -> Tactics {
    let own = ai_pick_lineup(world, club);
    let opp = ai_pick_lineup(world, opponent);
    let gap = lineup_strength(world, &own) - lineup_strength(world, &opp);

    let mentality = if gap > k.mentality_strength_gap {
        Mentality::Attacking
    } else if gap < -k.mentality_strength_gap {
        Mentality::Defensive
    } else {
        Mentality::Balanced
    };

    // Reuses the exact function `formation_p_wide` already scales against —
    // no second encoding of "how wide is this team."
    let roles: Vec<Role> = own.formation_def().slots.to_vec();
    let wide_share = resolve::wide_presence_share(&roles);
    let width = if wide_share > k.width_wide_share {
        Width::Wide
    } else if wide_share < k.width_narrow_share {
        Width::Narrow
    } else {
        Width::Balanced
    };

    let n = XI as f64;
    let pass_atk_mean: f64 = own
        .players
        .iter()
        .map(|&pid| contest::score(&world.player(pid).attributes, contest::PASS_ATK))
        .sum::<f64>()
        / n;
    let mut tempo = if pass_atk_mean > k.tempo_patient_pass_atk {
        Tempo::Patient
    } else if pass_atk_mean < k.tempo_direct_pass_atk {
        Tempo::Direct
    } else {
        Tempo::Balanced
    };
    // Away underdogs pair Defensive with Direct — the counter posture of §5,
    // emerging from the policy rather than hard-coded as a named tactic.
    if !is_home && mentality == Mentality::Defensive {
        tempo = Tempo::Direct;
    }

    let legs_mean: f64 = own
        .players
        .iter()
        .map(|&pid| {
            let a = &world.player(pid).attributes;
            a.get(Attribute::WorkRate) as f64 + a.get(Attribute::Stamina) as f64
        })
        .sum::<f64>()
        / n;
    let pressing = if legs_mean > k.pressing_high_legs {
        Pressing::High
    } else if legs_mean < k.pressing_deep_legs {
        Pressing::Deep
    } else {
        Pressing::Balanced
    };

    Tactics {
        mentality,
        tempo,
        width,
        pressing,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::derive_stream;
    use fforge_domain::World;

    fn tiny_world_and_lineups() -> (World, Lineup, Lineup) {
        super::golden::phase_2a_world_and_lineups()
    }

    /// Test-only convenience: `play_match` with production `Knobs`, an
    /// arbitrary-but-fixed Consistency stream, identity (empty) Condition,
    /// and its own fixed Injury stream — for tests that don't care about any
    /// of them specifically (most of this module). The T5/T6 identity tests
    /// (`golden` module) call `play_match` directly with
    /// `consistency_sigma_max: 0.0`/`injury_rate: 0.0` pinned instead.
    fn play(world: &World, home: &Lineup, away: &Lineup, rng: &mut Rng) -> MatchOutcome {
        let mut consistency_rng = derive_stream(0, CONSISTENCY_NS);
        let mut injury_rng = derive_stream(0, INJURY_NS);
        let mut foul_rng = derive_stream(0, FOUL_NS);
        play_match(
            world,
            home,
            away,
            rng,
            &mut consistency_rng,
            &mut injury_rng,
            &mut foul_rng,
            &Knobs::default(),
            &std::collections::BTreeMap::new(),
            GameDate { days: 0 },
        )
    }

    #[test]
    fn every_event_names_an_actor_in_the_fielding_sides_xi() {
        // The identity enrichment invariant (MATCH_MODEL.md §9): each beat's
        // `actor` is a player the resolver sampled from the `side`-relative
        // fielding XI, so it must be a member of that XI — and `opponent`,
        // when a contest names one, must belong to the other side's XI. No
        // event may reference a player who was not on the pitch for its side.
        let (world, home, away) = tiny_world_and_lineups();
        let home_xi: std::collections::BTreeSet<_> = home.players.iter().copied().collect();
        let away_xi: std::collections::BTreeSet<_> = away.players.iter().copied().collect();
        for seed in 0..64u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play(&world, &home, &away, &mut rng);
            for event in &outcome.stream {
                let (fielding, opposing) = match event.side {
                    Side::Home => (&home_xi, &away_xi),
                    Side::Away => (&away_xi, &home_xi),
                };
                assert!(
                    fielding.contains(&event.actor),
                    "seed {seed}: {:?} at {}' names actor {} who is not in the fielding side's XI",
                    event.kind,
                    event.minute,
                    event.actor
                );
                if let Some(opponent) = event.opponent {
                    assert!(
                        opposing.contains(&opponent),
                        "seed {seed}: {:?} at {}' names opponent {opponent} who is not in the \
                         opposing side's XI",
                        event.kind,
                        event.minute
                    );
                }
            }
        }
    }

    #[test]
    fn every_boundary_consequence_is_now_populated() {
        // MATCH_MODEL.md §12/§11 sequencing step 1: the boundary was grown
        // ahead of the models that fill it (`injuries` T10, `cards` T11,
        // `ratings` T13, each with its own dedicated test coverage
        // elsewhere) — this is the closing half of that sequencing note,
        // now that every field the boundary was grown for is real: a full
        // 90-minute match must produce a non-empty rating for every player
        // who actually appeared.
        let (world, home, away) = tiny_world_and_lineups();
        for seed in 0..32u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play(&world, &home, &away, &mut rng);
            assert!(
                !outcome.ratings.is_empty(),
                "seed {seed}: a full match must rate every appeared player"
            );
            let appeared: std::collections::BTreeSet<PlayerId> = outcome
                .minutes
                .iter()
                .filter(|&&(_, m)| m > 0)
                .map(|&(pid, _)| pid)
                .collect();
            let rated: std::collections::BTreeSet<PlayerId> =
                outcome.ratings.iter().map(|&(pid, _)| pid).collect();
            assert_eq!(
                appeared, rated,
                "seed {seed}: exactly the players with minutes > 0 must be rated"
            );
        }
    }

    #[test]
    fn same_seed_same_outcome() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut r1 = derive_stream(99, 1);
        let mut r2 = derive_stream(99, 1);
        let a = play(&world, &home, &away, &mut r1);
        let b = play(&world, &home, &away, &mut r2);
        assert_eq!(
            a, b,
            "identical (lineups, world, rng stream) must yield an identical outcome"
        );
    }

    #[test]
    fn different_streams_can_diverge() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut r1 = derive_stream(1, 1);
        let mut r2 = derive_stream(2, 1);
        let a = play(&world, &home, &away, &mut r1);
        let b = play(&world, &home, &away, &mut r2);
        assert_ne!(
            a.stream, b.stream,
            "different rng streams should not replay identically"
        );
    }

    #[test]
    fn stream_is_never_empty_and_ends_with_a_final_score_consistent_with_shot_events() {
        let (world, home, away) = tiny_world_and_lineups();
        let mut rng = derive_stream(42, 1);
        let outcome = play(&world, &home, &away, &mut rng);
        assert!(
            !outcome.stream.is_empty(),
            "a 90-minute match must produce events"
        );
        let goal_events = outcome
            .stream
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    MatchEventKind::Shot {
                        outcome: ShotOutcome::Goal,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            goal_events,
            outcome.home_goals as usize + outcome.away_goals as usize,
            "every goal in the score must have exactly one corresponding Shot{{outcome: Goal}} event"
        );
    }

    #[test]
    fn identical_squads_show_a_structural_home_advantage() {
        // Same club on both sides of the ball — the only asymmetry left is
        // home_bias and each half's kickoff. Pooled over many seeds, home
        // must win more often than away (mirrors the Phase-1 crude-engine
        // home-advantage invariant, now against the real resolution model).
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _schedule, _start) = crate::worldgen::generate(7, &cfg);
        let club = world.competition.clubs[0];
        let lineup = ai_pick_lineup(&world, club);

        let mut home_wins = 0u32;
        let mut away_wins = 0u32;
        for seed in 0..200u64 {
            let mut rng = derive_stream(seed, 1);
            let outcome = play(&world, &lineup, &lineup, &mut rng);
            match outcome.home_goals.cmp(&outcome.away_goals) {
                std::cmp::Ordering::Greater => home_wins += 1,
                std::cmp::Ordering::Less => away_wins += 1,
                std::cmp::Ordering::Equal => {}
            }
        }
        assert!(
            home_wins > away_wins,
            "home_bias must be visible: {home_wins} home wins vs {away_wins} away wins"
        );
    }

    #[test]
    fn a_tired_home_side_scores_visibly_fewer_expected_goals_than_a_fresh_one() {
        // MATCH_MODEL.md §13: condition scales fatigue_mult's starting
        // point, so a side that kicks off already tired should underperform
        // the identical side at full condition, pooled over many seeds.
        // Applied to home rather than away: the golden fixture's two clubs
        // sit at the extreme ends of worldgen's quality spread
        // (`golden::PHASE_2A_SEEDS_0_32`'s own doc comment), so away scores
        // ~0 in every seed regardless of condition — a floor a `<` comparison
        // could never detect. Home scores double digits, giving the
        // multiplier's effect real room to show up in the pooled total.
        let (world, home, away) = tiny_world_and_lineups();
        let tired: std::collections::BTreeMap<PlayerId, f64> =
            home.players.iter().map(|&pid| (pid, 0.6)).collect();
        let fresh: std::collections::BTreeMap<PlayerId, f64> = std::collections::BTreeMap::new();

        // Identity Injuries and Fouls (§2.1): this test isolates condition's
        // own effect on fatigue, and a player dropping out of contention
        // (mid-match injury, or a red card) is an unrelated confound to the
        // question it's asking.
        let k = Knobs {
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        };
        let pooled_home_goals = |conditions: &std::collections::BTreeMap<PlayerId, f64>| {
            let mut total = 0u32;
            for seed in 0..300u64 {
                let mut rng = derive_stream(seed, 1);
                let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS);
                let mut injury_rng = derive_stream(seed, INJURY_NS);
                let mut foul_rng = derive_stream(seed, FOUL_NS);
                let outcome = play_match(
                    &world,
                    &home,
                    &away,
                    &mut rng,
                    &mut consistency_rng,
                    &mut injury_rng,
                    &mut foul_rng,
                    &k,
                    conditions,
                    GameDate { days: 0 },
                );
                total += outcome.home_goals as u32;
            }
            total
        };

        let tired_goals = pooled_home_goals(&tired);
        let fresh_goals = pooled_home_goals(&fresh);
        assert!(
            tired_goals < fresh_goals,
            "a tired home side ({tired_goals} goals pooled) should score fewer than the \
             identical side at full condition ({fresh_goals} goals pooled)"
        );
    }

    /// The home lineup from `tiny_world_and_lineups`, with a bench drawn
    /// from whichever squad players `ai_pick_lineup` didn't start
    /// (`MATCH_MODEL.md` §16, T12) — the shared fixture the substitution
    /// tests below build their plans against.
    fn home_lineup_with_bench(world: &World, home: &Lineup, bench_size: usize) -> Lineup {
        let squad = &world.club(world.club_of(home.players[0]).unwrap()).players;
        let bench: Vec<PlayerId> = squad
            .iter()
            .copied()
            .filter(|pid| !home.players.contains(pid))
            .take(bench_size)
            .collect();
        assert_eq!(
            bench.len(),
            bench_size,
            "the worldgen squad must have enough depth for this test's bench"
        );
        Lineup {
            bench,
            ..home.clone()
        }
    }

    #[test]
    fn a_hand_built_substitution_plan_is_honoured_deterministically() {
        use fforge_domain::{SubAction, SubCondition, SubRule};

        let (world, home, away) = tiny_world_and_lineups();
        let mut home = home_lineup_with_bench(&world, &home, 1);
        let player_out = home.players[10];
        let player_in = home.bench[0];
        home.sub_plan = vec![SubRule {
            conditions: vec![SubCondition::MinuteAtLeast(60)],
            action: SubAction::Substitute {
                player_out,
                player_in,
            },
        }];

        let run = || {
            let mut rng = derive_stream(42, 1);
            play(&world, &home, &away, &mut rng)
        };
        let a = run();
        let b = run();
        assert_eq!(
            a, b,
            "identical inputs must resolve the same substitution the same way"
        );

        let subs: Vec<_> = a
            .stream
            .iter()
            .filter(|e| matches!(e.kind, MatchEventKind::Substitution { .. }))
            .collect();
        assert_eq!(
            subs.len(),
            1,
            "exactly the one plan rule's substitution must fire, no more"
        );
        let sub = subs[0];
        assert_eq!(
            sub.actor, player_in,
            "the entering player is the beat's actor"
        );
        assert_eq!(sub.kind, MatchEventKind::Substitution { player_out });
        assert!(
            sub.minute >= 60,
            "the rule's MinuteAtLeast(60) condition must gate when it fires, got minute {}",
            sub.minute
        );

        // MATCH_MODEL.md §16/§2.8: minutes are now non-degenerate — both the
        // departing starter and the entering substitute record partial
        // (neither zero nor a flat 90) minutes.
        let mins = |pid: PlayerId| a.minutes.iter().find(|&&(p, _)| p == pid).map(|&(_, m)| m);
        let out_mins = mins(player_out).expect("the departed starter must still be recorded");
        let in_mins = mins(player_in).expect("the entering substitute must be recorded");
        assert!(
            out_mins > 0 && out_mins < 90,
            "the departed starter's minutes must be partial, got {out_mins}"
        );
        assert!(
            in_mins > 0 && in_mins < 90,
            "the entering substitute's minutes must be partial, got {in_mins}"
        );
    }

    #[test]
    fn substitution_count_never_exceeds_the_cap() {
        use fforge_domain::{MAX_SUBSTITUTIONS, SubAction, SubCondition, SubRule};

        let (world, home, away) = tiny_world_and_lineups();
        let mut home = home_lineup_with_bench(&world, &home, 5);
        // Five immediately-eligible rules, each a distinct starter/bench
        // pair — more than the cap, all triggerable from kickoff.
        home.sub_plan = (0..5)
            .map(|i| SubRule {
                conditions: vec![SubCondition::MinuteAtLeast(0)],
                action: SubAction::Substitute {
                    player_out: home.players[6 + i],
                    player_in: home.bench[i],
                },
            })
            .collect();

        let mut rng = derive_stream(7, 1);
        let outcome = play(&world, &home, &away, &mut rng);
        let subs = outcome
            .stream
            .iter()
            .filter(|e| matches!(e.kind, MatchEventKind::Substitution { .. }))
            .count();
        assert_eq!(
            subs, MAX_SUBSTITUTIONS,
            "a plan offering more substitutions than the cap must still execute only {MAX_SUBSTITUTIONS}"
        );
    }

    #[test]
    fn forced_evaluation_fires_promptly_on_injury() {
        use fforge_domain::{SubAction, SubCondition, SubRule};

        let (world, home, away) = tiny_world_and_lineups();
        let mut home = home_lineup_with_bench(&world, &home, 1);
        let player_in = home.bench[0];
        // One rule per starter: whoever gets hurt first is covered — proves
        // the *mechanism* (forced evaluation reacts to an injury, not just
        // the fixed checkpoints) rather than depending on which player an
        // extreme injury rate happens to hit.
        home.sub_plan = home
            .players
            .iter()
            .map(|&pid| SubRule {
                conditions: vec![SubCondition::PlayerInjured(pid)],
                action: SubAction::Substitute {
                    player_out: pid,
                    player_in,
                },
            })
            .collect();

        // Cranked far past any calibrated production value: this test only
        // cares that a forced decision point fires *before* the next fixed
        // checkpoint once an injury lands, not about a plausible rate.
        let k = Knobs {
            injury_rate: 1.0,
            injury_ambient_base: 1.0,
            injury_knock_prob: 1.0, // every hit is a trivial Knock — irrelevant to this test
            ..Knobs::default()
        };
        let mut rng = derive_stream(0, 1);
        let mut consistency_rng = derive_stream(0, CONSISTENCY_NS);
        let mut injury_rng = derive_stream(0, INJURY_NS);
        let mut foul_rng = derive_stream(0, FOUL_NS);
        let outcome = play_match(
            &world,
            &home,
            &away,
            &mut rng,
            &mut consistency_rng,
            &mut injury_rng,
            &mut foul_rng,
            &k,
            &std::collections::BTreeMap::new(),
            GameDate { days: 0 },
        );

        assert!(
            !outcome.injuries.is_empty(),
            "the cranked ambient rate must produce at least one injury"
        );
        let sub = outcome
            .stream
            .iter()
            .find(|e| matches!(e.kind, MatchEventKind::Substitution { .. }))
            .expect("an injured starter must be covered by the matching plan rule");
        assert!(
            sub.minute < 45,
            "forced evaluation must react before the next fixed checkpoint (half-time), \
             got minute {} — a substitution this early cannot be explained by a fixed \
             checkpoint alone",
            sub.minute
        );
    }

    #[test]
    fn forced_evaluation_fires_promptly_on_a_red_card() {
        use fforge_domain::{SubAction, SubCondition, SubRule};

        let (world, home, away) = tiny_world_and_lineups();
        let mut home = home_lineup_with_bench(&world, &home, 1);
        let player_in = home.bench[0];
        home.sub_plan = vec![SubRule {
            conditions: vec![SubCondition::ManDown],
            action: SubAction::Substitute {
                // Bring the sub on for a nominal outfielder — the point of
                // this test is *when* the plan reacts, not which player it
                // covers.
                player_out: home.players[9],
                player_in,
            },
        }];

        // Well above any calibrated production value (foul_base -2.5,
        // foul_red_base 0.002) but deliberately *not* cranked to saturation
        // — a foul_red_base near 1.0 sends off enough of a side that
        // `sample_by_presence` runs out of on-pitch players for some zone
        // before full time (11 players don't survive a whole match of
        // "every foul is a red"). Pooled over a few seeds for robustness
        // rather than betting on one hand-picked seed.
        let k = Knobs {
            foul_rate: 1.0,
            foul_base: -1.0,
            foul_red_base: 0.08,
            ..Knobs::default()
        };
        let mut found = false;
        for seed in 0..30u64 {
            let mut rng = derive_stream(seed, 1);
            let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS);
            let mut injury_rng = derive_stream(seed, INJURY_NS);
            let mut foul_rng = derive_stream(seed, FOUL_NS);
            let outcome = play_match(
                &world,
                &home,
                &away,
                &mut rng,
                &mut consistency_rng,
                &mut injury_rng,
                &mut foul_rng,
                &k,
                &std::collections::BTreeMap::new(),
                GameDate { days: 0 },
            );
            if let Some(sub) = outcome
                .stream
                .iter()
                .find(|e| matches!(e.kind, MatchEventKind::Substitution { .. }))
            {
                assert!(
                    sub.minute < 45,
                    "forced evaluation must react before the next fixed checkpoint \
                     (half-time), got minute {} — a substitution this early cannot be \
                     explained by a fixed checkpoint alone",
                    sub.minute
                );
                found = true;
                break;
            }
        }
        assert!(
            found,
            "an elevated red-card rate, pooled over 30 seeds, must produce at least one \
             ManDown-triggered substitution"
        );
    }

    // --- S1b: ai_pick_lineup's bench-selection / default-plan policy
    // (MATCH_MODEL.md §16's "The v1 AI bench-selection and
    // default-substitution-plan policy") ---

    /// A handful of freshly-worldgen'd clubs, each with a full-depth squad
    /// (`worldgen::SQUAD_TEMPLATE` sums to 24, well above `XI + BENCH_SIZE`
    /// = 18), so bench composition is never depth-starved in these tests.
    fn bench_test_clubs() -> Vec<(World, ClubId)> {
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 6,
            ..Default::default()
        };
        (0..3u64)
            .map(|seed| {
                let (world, _schedule, _start) = crate::worldgen::generate(seed, &cfg);
                let clubs = world.competition.clubs.clone();
                (world, clubs)
            })
            .flat_map(|(world, clubs)| clubs.into_iter().map(move |c| (world.clone(), c)))
            .collect()
    }

    #[test]
    fn bench_composition_invariants_hold() {
        for (world, club) in bench_test_clubs() {
            let lineup = ai_pick_lineup(&world, club);
            let xi: std::collections::BTreeSet<_> = lineup.players.iter().copied().collect();

            assert_eq!(
                lineup.bench.len(),
                fforge_domain::BENCH_SIZE,
                "club {club:?}: a full-depth squad must fill the bench to capacity"
            );

            let bench_set: std::collections::BTreeSet<_> = lineup.bench.iter().copied().collect();
            assert_eq!(
                bench_set.len(),
                lineup.bench.len(),
                "club {club:?}: no duplicate players on the bench"
            );
            assert!(
                xi.is_disjoint(&bench_set),
                "club {club:?}: no player is both a starter and on the bench"
            );

            assert!(
                lineup
                    .bench
                    .iter()
                    .any(|&pid| world.player(pid).natural_role == Role::Gk),
                "club {club:?}: the bench must reserve a backup goalkeeper"
            );
        }
    }

    #[test]
    fn default_plan_only_names_dressed_players_in_valid_pairs() {
        for (world, club) in bench_test_clubs() {
            let lineup = ai_pick_lineup(&world, club);
            let xi: std::collections::BTreeSet<_> = lineup.players.iter().copied().collect();
            let bench: std::collections::BTreeSet<_> = lineup.bench.iter().copied().collect();
            let dressed: std::collections::BTreeSet<_> = xi.union(&bench).copied().collect();

            assert!(
                !lineup.sub_plan.is_empty(),
                "club {club:?}: a full-depth squad's default plan should not be empty"
            );

            let mut substitute_rules = 0;
            let mut tactics_rules = 0;
            for rule in &lineup.sub_plan {
                match rule.action {
                    SubAction::Substitute {
                        player_out,
                        player_in,
                    } => {
                        substitute_rules += 1;
                        assert!(
                            xi.contains(&player_out),
                            "club {club:?}: a Substitute rule's player_out must be a starter"
                        );
                        assert!(
                            bench.contains(&player_in),
                            "club {club:?}: a Substitute rule's player_in must be on the bench"
                        );
                        assert_ne!(
                            player_out, player_in,
                            "club {club:?}: a rule can't substitute a player for himself"
                        );
                    }
                    SubAction::SetMentality(_)
                    | SubAction::SetTempo(_)
                    | SubAction::SetWidth(_)
                    | SubAction::SetPressing(_) => tactics_rules += 1,
                }
                for cond in &rule.conditions {
                    if let SubCondition::PlayerInjured(pid)
                    | SubCondition::PlayerConditionBelow(pid, _) = *cond
                    {
                        assert!(
                            dressed.contains(&pid),
                            "club {club:?}: a condition may only name a dressed player"
                        );
                    }
                }
            }
            // Authoring-time bound, not the runtime MAX_SUBSTITUTIONS firing
            // cap: at most one forced-cover rule per Role variant (8) plus
            // at most two fatigue rules (§16.3's own cap).
            assert!(
                substitute_rules <= Role::ALL.len() + 2,
                "club {club:?}: {substitute_rules} Substitute rules exceeds the authoring-time \
                 bound of one forced-cover rule per role plus two fatigue rules"
            );
            let fatigue_rules = lineup
                .sub_plan
                .iter()
                .filter(|r| {
                    matches!(r.action, SubAction::Substitute { .. })
                        && r.conditions.contains(&SubCondition::MinuteAtLeast(70))
                        && r.conditions
                            .iter()
                            .any(|c| matches!(c, SubCondition::PlayerConditionBelow(_, 80)))
                })
                .count();
            assert!(
                fatigue_rules <= 2,
                "club {club:?}: at most two fatigue rules, found {fatigue_rules}"
            );
            // The chase/hold rules (§16.3): tactics-only, per the vocabulary
            // finding — always exactly two, unconditional on squad depth.
            assert_eq!(
                tactics_rules, 2,
                "club {club:?}: chase/hold contributes exactly two tactics-change rules"
            );

            // A bench player may be the designated backup for more than one
            // rule (a forced-cover rule and a fatigue rule reuse the same
            // pair, §16.3), but never for two *different* starters — each
            // bench slot covers at most one role.
            let mut backup_for: std::collections::BTreeMap<PlayerId, PlayerId> =
                std::collections::BTreeMap::new();
            for rule in &lineup.sub_plan {
                if let SubAction::Substitute {
                    player_out,
                    player_in,
                } = rule.action
                {
                    if let Some(&existing) = backup_for.get(&player_in) {
                        assert_eq!(
                            existing, player_out,
                            "club {club:?}: bench player {player_in:?} is designated as the \
                             backup for two different starters ({existing:?} and {player_out:?})"
                        );
                    } else {
                        backup_for.insert(player_in, player_out);
                    }
                }
            }
        }
    }

    #[test]
    fn bench_and_plan_are_deterministic() {
        for (world, club) in bench_test_clubs() {
            let a = ai_pick_lineup(&world, club);
            let b = ai_pick_lineup(&world, club);
            assert_eq!(
                a.bench, b.bench,
                "club {club:?}: bench selection must be a pure function of world state"
            );
            assert_eq!(
                a.sub_plan, b.sub_plan,
                "club {club:?}: default plan generation must be a pure function of world state"
            );
        }
    }

    #[test]
    fn chase_and_hold_rules_are_tactics_only_as_the_vocabulary_finding_requires() {
        // §16's own finding: a score-conditioned player pick isn't
        // expressible, so v1's chase/hold is SetMentality-only. Pin the
        // exact rules so a future change to this shape is a deliberate
        // decision, not a silent drift.
        let (world, club) = bench_test_clubs().into_iter().next().unwrap();
        let lineup = ai_pick_lineup(&world, club);
        let chase = SubRule {
            conditions: vec![
                SubCondition::Score(fforge_domain::ScoreState::Trailing),
                SubCondition::MinuteAtLeast(70),
            ],
            action: SubAction::SetMentality(Mentality::Attacking),
        };
        let hold = SubRule {
            conditions: vec![
                SubCondition::Score(fforge_domain::ScoreState::Leading),
                SubCondition::MinuteAtLeast(75),
            ],
            action: SubAction::SetMentality(Mentality::Defensive),
        };
        assert!(
            lineup.sub_plan.contains(&chase),
            "the plan must contain the chase rule verbatim"
        );
        assert!(
            lineup.sub_plan.contains(&hold),
            "the plan must contain the hold rule verbatim"
        );
    }
}

/// The pinned Phase-2a golden baseline (batch-3 handoff T5): the reference
/// every 2e identity invariant in the batch asserts against (§2.1). Captured
/// as the last commit before any engine change, per the handoff's explicit
/// ordering — it cannot be captured retroactively, since T3 (`natural_fitness`)
/// already changed which world every worldgen seed produces.
///
/// `TACTICS_MODEL.md` §4's `neutral_tactics_reproduce_phase_2a_bit_for_bit`
/// (T6) replays these seeds through the tactics-aware engine at
/// `neutral()`/`neutral()` and asserts equality against this table; any
/// accidental extra draw or perturbed probability at the neutral setting
/// fails it loudly, as a wiring bug rather than a value to update.
#[cfg(test)]
pub(crate) mod golden {
    use super::*;
    use crate::rng::derive_stream;

    /// The exact fixture `TACTICS_MODEL.md` §4 names: a 2-club world at
    /// worldgen seed 7, `ai_pick_lineup` XIs for each club.
    pub(crate) fn phase_2a_world_and_lineups() -> (World, Lineup, Lineup) {
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _schedule, _start) = crate::worldgen::generate(7, &cfg);
        let clubs = world.competition.clubs.clone();
        let home = ai_pick_lineup(&world, clubs[0]);
        let away = ai_pick_lineup(&world, clubs[1]);
        (world, home, away)
    }

    /// `(home_goals, away_goals, stream.len())` for seeds `0..32`, RNG stream
    /// tag `1` (`derive_stream(seed, 1)`), against
    /// `phase_2a_world_and_lineups()`. The two clubs in this seed-7, 2-club
    /// world sit at the extreme ends of worldgen's quality spread (§3's
    /// evenly-spread-then-shuffled anchors collapse to the min/max with only
    /// two clubs), so every match is a lopsided home win — irrelevant here,
    /// since this table exists to catch *any* movement, not to be a
    /// representative match.
    // Re-pinned by `DEVELOPMENT_MODEL.md` §8's worldgen seeding invert
    // (`worldgen::gen_player` now draws PA first and seeds attributes on the
    // envelope beneath it — §8.1): the pinned world at seed 7 is a real
    // `worldgen::generate` output, so a change to how players are generated
    // moves this table exactly the way a world re-roll is expected to (§8.2/
    // `BACKLOG.md` §2 item 2: "a re-roll is not a re-fit") — nothing about
    // `play_match` itself changed, and both golden tests below still agree
    // with each other bit-for-bit, which is what this table actually guards.
    pub(crate) const PHASE_2A_SEEDS_0_32: [(u8, u8, usize); 32] = [
        (11, 0, 870),
        (17, 0, 871),
        (13, 0, 866),
        (13, 0, 868),
        (16, 0, 868),
        (12, 0, 884),
        (17, 0, 864),
        (9, 0, 858),
        (7, 0, 873),
        (14, 0, 876),
        (13, 0, 880),
        (14, 0, 870),
        (14, 0, 876),
        (16, 0, 876),
        (11, 0, 862),
        (16, 0, 876),
        (10, 0, 859),
        (15, 0, 877),
        (17, 0, 872),
        (12, 0, 865),
        (11, 0, 857),
        (13, 0, 872),
        (12, 0, 870),
        (17, 0, 876),
        (18, 0, 874),
        (12, 0, 870),
        (16, 0, 876),
        (11, 0, 868),
        (11, 0, 877),
        (9, 0, 863),
        (14, 0, 886),
        (14, 0, 874),
    ];

    /// The T5/T6 identity tests below pin `consistency_sigma_max: 0.0`,
    /// `injury_rate: 0.0`, and `foul_rate: 0.0` (§2.1) explicitly rather
    /// than using `Knobs::default()` — T8/T10/T11 gave Consistency,
    /// Injuries, and Fouls real nonzero production defaults, so these
    /// golden baselines must keep asserting the *pre*-2e engine
    /// specifically, independent of whatever `Knobs::default()` is today.
    fn identity_2e_knobs() -> Knobs {
        Knobs {
            consistency_sigma_max: 0.0,
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        }
    }

    #[test]
    fn phase_2a_golden_baseline_reproduces() {
        // Tracks whatever `ai_pick_lineup` currently produces — neutral
        // tactics today (T6's scope fence), but T7 will make it call
        // `ai_pick_tactics`, at which point this reading is expected to move
        // and gets re-pinned deliberately (§8's rollout discipline), same as
        // `favourite_discrimination_regression_guard`.
        //
        // S1b (`MATCH_MODEL.md` §16's bench-selection/default-plan policy):
        // `ai_pick_lineup` now also fills a real `bench`/`sub_plan`, which
        // is a Rust wiring change, not a tracked "current AI behaviour"
        // drift the way tactics is meant to be — this test's job is to pin
        // the pre-substitution engine, so bench/plan are forced to the
        // identity (empty) explicitly, the same discipline
        // `neutral_tactics_reproduce_phase_2a_bit_for_bit` already applies
        // to tactics below.
        let (world, mut home, mut away) = phase_2a_world_and_lineups();
        home.bench.clear();
        home.sub_plan.clear();
        away.bench.clear();
        away.sub_plan.clear();
        let k = identity_2e_knobs();
        for (seed, &(hg, ag, len)) in (0u64..32).zip(PHASE_2A_SEEDS_0_32.iter()) {
            let mut rng = derive_stream(seed, 1);
            let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS);
            let mut injury_rng = derive_stream(seed, INJURY_NS);
            let mut foul_rng = derive_stream(seed, FOUL_NS);
            let outcome = play_match(
                &world,
                &home,
                &away,
                &mut rng,
                &mut consistency_rng,
                &mut injury_rng,
                &mut foul_rng,
                &k,
                &std::collections::BTreeMap::new(),
                GameDate { days: 0 },
            );
            assert_eq!(
                (outcome.home_goals, outcome.away_goals, outcome.stream.len()),
                (hg, ag, len),
                "seed {seed}: Phase-2a golden baseline moved — a wiring bug \
                 in whatever landed since T5, never a re-tune"
            );
        }
    }

    #[test]
    fn neutral_tactics_reproduce_phase_2a_bit_for_bit() {
        // TACTICS_MODEL.md §4's named golden test: explicitly force
        // `Tactics::neutral()` on both sides — independent of whatever
        // `ai_pick_lineup` defaults to (today neutral, but T7 changes that)
        // — so this stays a permanent bit-identity guardrail rather than
        // tracking the AI policy's evolving choice. S1b: also force
        // bench/sub_plan to empty for the same reason — this is the
        // "policy disabled" identity case the bench/plan policy's own
        // T5/T6-style guardrail needs.
        let (world, mut home, mut away) = phase_2a_world_and_lineups();
        home.tactics = fforge_domain::Tactics::neutral();
        away.tactics = fforge_domain::Tactics::neutral();
        home.bench.clear();
        home.sub_plan.clear();
        away.bench.clear();
        away.sub_plan.clear();
        let k = identity_2e_knobs();
        for (seed, &(hg, ag, len)) in (0u64..32).zip(PHASE_2A_SEEDS_0_32.iter()) {
            let mut rng = derive_stream(seed, 1);
            let mut consistency_rng = derive_stream(seed, CONSISTENCY_NS);
            let mut injury_rng = derive_stream(seed, INJURY_NS);
            let mut foul_rng = derive_stream(seed, FOUL_NS);
            let outcome = play_match(
                &world,
                &home,
                &away,
                &mut rng,
                &mut consistency_rng,
                &mut injury_rng,
                &mut foul_rng,
                &k,
                &std::collections::BTreeMap::new(),
                GameDate { days: 0 },
            );
            assert_eq!(
                (outcome.home_goals, outcome.away_goals, outcome.stream.len()),
                (hg, ag, len),
                "seed {seed}: neutral tactics must reproduce the Phase-2a \
                 baseline bit-for-bit (§4) — movement here is a wiring bug, \
                 never a re-tune"
            );
        }
    }
}
