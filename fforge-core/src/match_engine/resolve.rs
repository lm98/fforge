//! The possession loop (`MATCH_MODEL.md` §3–5): sample an actor and a
//! primary defender by zone presence, resolve one contest with the shared
//! logistic shape, and transition zones — or, on reaching `Box`, resolve a
//! shot immediately. A direct port of the calibrated Python prototype's
//! `_step` / `_take_shot` / `select_action`.

use super::contest::{self, blend, contest_p, fatigue_mult};
use super::knobs::Knobs;
use super::ratings;
use super::stream::{MatchEvent, MatchEventKind, ShotKind, ShotOutcome, ShotSource, Side};
use super::tactics::{SideEffects, resolve_tactics};
use super::zone::{self, Zone};
use super::{Card, CardOutcome, InjuryOutcome, MatchOutcome};
use crate::rng::Rng;
use fforge_domain::{
    Attribute, Attributes, GameDate, Lineup, MAX_SUBSTITUTIONS, PlayerId, Role, ScoreState,
    SubAction, SubCondition, SubRule, Tactics, World, XI,
};
use std::cell::Cell;
use std::collections::BTreeMap;

#[derive(Clone)]
struct XiPlayer {
    /// The domain identity of this eleven's player, carried so the emitted
    /// stream can name who did what (`MATCH_MODEL.md` §9 / `TRANSFER_MODEL.md`
    /// §12 item 1) — it is only propagated into events, never sampled on.
    pid: PlayerId,
    role: Role,
    attrs: Attributes,
    /// Match minute this player entered the pitch (`MATCH_MODEL.md` §16,
    /// T12): `0.0` for every starter (the identity — every `fatigue_mult`
    /// call already effectively subtracts zero). A substitute's is his
    /// entry minute, so his own fatigue clock starts there rather than
    /// inheriting the departed player's accumulated drop — "fresh legs are
    /// mechanically real via `fatigue_mult`'s minute argument being offset"
    /// (§16), computed at each call site as `minute - entered_at_minute`.
    entered_at_minute: f64,
    /// Pre-match condition (`MATCH_MODEL.md` §13, identity `1.0`) — looked up
    /// once here from the caller's `conditions` map, then read by every
    /// `fatigue_mult` call this player is involved in for the rest of the
    /// match; not baked into `attrs` like Consistency, since it scales only
    /// the fatigue curve's starting point, not every attribute uniformly.
    condition: f64,
    /// Precomputed contact-channel injury multiplier (`MATCH_MODEL.md` §14,
    /// T10): folds in hidden Injury-proneness, its Professionalism discount,
    /// and the condition scaling — every input is static for the whole
    /// match, so each contact-event roll only has to apply the *other*
    /// player's Aggression intensity and `injury_base_contact`.
    contact_injury_mult: f64,
    /// A pre-rolled ambient injury (`MATCH_MODEL.md` §14), if any: `(onset
    /// minute, days_out)`. Rolled once per player at kickoff — an
    /// approximation of a per-minute hazard integrated over 90' as one
    /// Bernoulli trial, since the expected count is what §14's targets care
    /// about, not the intra-match timing of a non-eventful injury. `simulate`
    /// fires it (setting `injured_from_minute`) the first time its loop's
    /// `minute` reaches `onset`, and only if the contact channel hasn't
    /// already injured this player first.
    pending_ambient: Cell<Option<(f64, u16)>>,
    /// Minute injured from, if any — `Cell` so a contest resolution can mark
    /// a participant injured through the shared `&[XiPlayer]` slice without
    /// threading `&mut` through the whole possession loop.
    injured_from_minute: Cell<Option<f64>>,
    /// In-match foul count so far (`MATCH_MODEL.md` §15, T11) — repeat
    /// fouling pushes up the yellow-card probability (the referee's own
    /// patience, state the engine already has for free).
    foul_count: Cell<u8>,
    /// Whether this player already has a yellow card this match — the next
    /// bookable foul is a second yellow (a red by bookkeeping) rather than a
    /// fresh severity draw.
    has_yellow: Cell<bool>,
    /// Minute sent off from, if any (`MATCH_MODEL.md` §15: a red removes the
    /// player for the remainder) — `Cell` for the same shared-slice-mutation
    /// reason as `injured_from_minute`.
    sent_off_from_minute: Cell<Option<f64>>,
}

/// Whether `player` is still on the pitch at `minute` (`MATCH_MODEL.md` §15,
/// T11) — false only once their `sent_off_from_minute` has been reached. An
/// injured-but-not-sent-off player still counts as on the pitch (§14:
/// "continues at reduced effectiveness"); only a red card actually shrinks
/// the XI.
fn on_pitch(player: &XiPlayer, minute: f64) -> bool {
    match player.sent_off_from_minute.get() {
        Some(off) => minute < off,
        None => true,
    }
}

/// The player currently keeping goal (`MATCH_MODEL.md` §15's red-carded-
/// keeper edge case): formation slot 0 if he's still on the pitch, else the
/// lowest-indexed on-pitch outfielder — re-roled by circumstance, not by
/// field, so *his* attributes (not a real keeper's) make the punishment
/// automatic. No substitutions exist yet (T12), so this is the only
/// response v1 has to a sent-off keeper.
fn current_gk(xi: &[XiPlayer], minute: f64) -> &XiPlayer {
    if on_pitch(&xi[0], minute) {
        return &xi[0];
    }
    xi.iter()
        .find(|p| on_pitch(p, minute))
        .expect("a match is abandoned, never simulated, once a side runs out of outfield players")
}

/// The contact-channel injury multiplier (`MATCH_MODEL.md` §14, T10):
/// Injury-proneness scales it up, Professionalism discounts it (the
/// schema's "aging/injury resistance"), and a low `condition` deepens it
/// ("gives §13 teeth"). Every input here is fixed for the whole match, so
/// `build_xi` computes this once per player rather than at every contest.
fn contact_injury_mult(
    injury_proneness: u8,
    professionalism: u8,
    condition: f64,
    k: &Knobs,
) -> f64 {
    let prone = 1.0 + k.injury_contact_prone_scale * (injury_proneness as f64 - 50.0) / 50.0;
    let prof_discount =
        1.0 - k.injury_contact_prof_discount * (professionalism as f64 - 50.0) / 50.0;
    let condition_mult = 1.0 + k.injury_contact_condition_scale * (1.0 - condition);
    (prone * prof_discount * condition_mult).max(0.0)
}

/// Builds one side's XI, applying the Consistency per-match multiplier
/// (`MATCH_MODEL.md` §17, T8) to each player's effective attributes for the
/// whole match. `consistency_rng` is drawn from **unconditionally, one
/// player at a time in slot order** — fixed count and order regardless of
/// any attribute value, so stream position stays value-independent
/// (`development::tick_changes`'s own rule). At `k.consistency_sigma_max ==
/// 0.0` every multiplier is exactly `1.0` (§2.1's identity), so this
/// reproduces the pre-2e attributes bit-for-bit even though draws still
/// happen — they land on `consistency_rng`, a stream wholly separate from
/// the match's own `rng`, so the main draw sequence is never touched
/// regardless of `sigma_max`. Also attaches each player's pre-match
/// `condition` (`MATCH_MODEL.md` §13, T9) from the caller-supplied,
/// already-derived `conditions` map — no RNG involved, so it can never
/// perturb either stream's draw sequence at any setting. Rolls the ambient
/// injury channel (`MATCH_MODEL.md` §14, T10) from `injury_rng` — its own
/// stream, drawn **unconditionally, one player at a time in slot order**,
/// the same discipline Consistency's `z` follows: an onset-minute draw
/// always happens, then `roll_injury`'s check (and, only if it fires, its
/// severity draws).
/// Builds one player's match-time `XiPlayer` — the per-slot body `build_xi`
/// used to run unconditionally over exactly the starting XI; factored out
/// (`MATCH_MODEL.md` §16, T12) so the same unconditional-fixed-order draw
/// discipline extends to bench players too, letting a substitute enter with
/// a properly-rolled Consistency multiplier and ambient-injury pre-roll
/// instead of a special-cased default. `entered_at_minute` is `0.0` for a
/// starter (the fatigue identity) or the caller-supplied entry minute for a
/// substitute.
#[allow(clippy::too_many_arguments)]
fn build_xi_player(
    world: &World,
    pid: PlayerId,
    role: Role,
    entered_at_minute: f64,
    consistency_rng: &mut Rng,
    injury_rng: &mut Rng,
    today: GameDate,
    k: &Knobs,
    conditions: &BTreeMap<PlayerId, f64>,
) -> XiPlayer {
    let player = world.player(pid);
    let consistency = player.character.consistency as f64;
    let sigma = k.consistency_sigma_max * (1.0 - consistency / 100.0);
    let z = consistency_rng.normal(0.0, 1.0);
    let mult = (1.0 + sigma * z).clamp(k.consistency_mult_min, k.consistency_mult_max);

    let mut attrs = player.attributes.clone();
    for attr in Attribute::ALL {
        let scaled = (attrs.get(attr) as f64 * mult).round().clamp(0.0, 100.0);
        attrs.set(attr, scaled as u8);
    }
    // Absent from the map = full condition (`MATCH_MODEL.md` §13's
    // identity): a caller with no `GameState` to read (tests, a
    // player with no recent appearances) gets exactly today's
    // pre-condition behaviour without needing a special-case map.
    let condition = conditions.get(&pid).copied().unwrap_or(1.0);
    let contact_injury_mult = contact_injury_mult(
        player.character.injury_proneness,
        player.character.professionalism,
        condition,
        k,
    );

    let age = player.age(today) as f64;
    let ambient_minute = injury_rng.f64() * 90.0; // always drawn — fixed order
    let p_ambient = (k.injury_rate
        * k.injury_ambient_base
        * (1.0 + k.injury_ambient_condition_scale * (1.0 - condition))
        * (1.0 + k.injury_ambient_age_scale * (age - k.injury_age_anchor).max(0.0)))
    .clamp(0.0, 1.0);
    let pending_ambient =
        roll_injury(injury_rng, p_ambient, k).map(|days_out| (ambient_minute, days_out));

    XiPlayer {
        pid,
        role,
        attrs,
        entered_at_minute,
        condition,
        contact_injury_mult,
        pending_ambient: Cell::new(pending_ambient),
        injured_from_minute: Cell::new(None),
        foul_count: Cell::new(0),
        has_yellow: Cell::new(false),
        sent_off_from_minute: Cell::new(None),
    }
}

/// Builds one side's starting XI, applying the Consistency per-match
/// multiplier (`MATCH_MODEL.md` §17, T8) to each player's effective
/// attributes for the whole match. `consistency_rng` is drawn from
/// **unconditionally, one player at a time in slot order** — fixed count
/// and order regardless of any attribute value, so stream position stays
/// value-independent (`development::tick_changes`'s own rule). At
/// `k.consistency_sigma_max == 0.0` every multiplier is exactly `1.0`
/// (§2.1's identity), so this reproduces the pre-2e attributes bit-for-bit
/// even though draws still happen — they land on `consistency_rng`, a
/// stream wholly separate from the match's own `rng`, so the main draw
/// sequence is never touched regardless of `sigma_max`. Also attaches each
/// player's pre-match `condition` (`MATCH_MODEL.md` §13, T9) from the
/// caller-supplied, already-derived `conditions` map — no RNG involved, so
/// it can never perturb either stream's draw sequence at any setting.
/// Rolls the ambient injury channel (`MATCH_MODEL.md` §14, T10) from
/// `injury_rng` — its own stream, drawn **unconditionally, one player at a
/// time in slot order**, the same discipline Consistency's `z` follows: an
/// onset-minute draw always happens, then `roll_injury`'s check (and, only
/// if it fires, its severity draws).
#[allow(clippy::too_many_arguments)]
fn build_xi(
    world: &World,
    lineup: &Lineup,
    consistency_rng: &mut Rng,
    injury_rng: &mut Rng,
    today: GameDate,
    k: &Knobs,
    conditions: &BTreeMap<PlayerId, f64>,
) -> Vec<XiPlayer> {
    let def = lineup.formation_def();
    lineup
        .players
        .iter()
        .enumerate()
        .map(|(slot, &pid)| {
            build_xi_player(
                world,
                pid,
                def.slots[slot],
                0.0,
                consistency_rng,
                injury_rng,
                today,
                k,
                conditions,
            )
        })
        .collect()
}

/// Builds a side's bench (`MATCH_MODEL.md` §16, T12), in `lineup.bench`'s
/// own order — the fixed-order tail of the same unconditional whole-squad
/// draw `build_xi` starts, so every dressed player (starter or bench) gets
/// exactly one Consistency multiplier and one ambient-injury pre-roll at
/// kickoff, regardless of whether he ever actually enters. This is what
/// keeps substitution evaluation itself entirely draw-free (§16's own
/// requirement): by the time a decision point runs, there is nothing left
/// to roll. `entered_at_minute` is filled in for real once (if) a bench
/// player is substituted on; `0.0` here is a harmless placeholder no
/// `fatigue_mult` call ever reads before that (a bench player never
/// appears in `att`/`def_side` until he's swapped into a starting slot).
/// Each bench player's role is his own `natural_role` — there is no
/// formation slot to assign him one from.
fn build_bench(
    world: &World,
    lineup: &Lineup,
    consistency_rng: &mut Rng,
    injury_rng: &mut Rng,
    today: GameDate,
    k: &Knobs,
    conditions: &BTreeMap<PlayerId, f64>,
) -> Vec<XiPlayer> {
    lineup
        .bench
        .iter()
        .map(|&pid| {
            let role = world.player(pid).natural_role;
            build_xi_player(
                world,
                pid,
                role,
                0.0,
                consistency_rng,
                injury_rng,
                today,
                k,
                conditions,
            )
        })
        .collect()
}

/// Draws an injury check unconditionally, then — only if it fires — its
/// severity (a category draw, then a within-band interpolation draw), the
/// same conditional-on-outcome shape `take_shot`'s on-target/beat-keeper
/// rolls already use. Returns the resolved `days_out` on a hit.
fn roll_injury(rng: &mut Rng, p_injury: f64, k: &Knobs) -> Option<u16> {
    let check = rng.f64();
    if check >= p_injury {
        return None;
    }
    let cat_roll = rng.f64();
    let within_roll = rng.f64();
    let [lo, hi] = if cat_roll < k.injury_knock_prob {
        k.injury_knock_days
    } else if cat_roll < k.injury_minor_cum_prob {
        k.injury_minor_days
    } else if cat_roll < k.injury_moderate_cum_prob {
        k.injury_moderate_days
    } else {
        k.injury_severe_days
    };
    Some((lo + within_roll * (hi - lo)).round() as u16)
}

/// Rolls a contact-channel injury check for `victim` (`MATCH_MODEL.md` §14,
/// T10) — a failed take-on (the tackle) or a headed shot's aerial duel.
/// `culprit_aggression` is the *other* player's Aggression (the tackler's
/// recklessness raises the tackled player's risk, not their own). No-ops
/// (no draw at all) if `victim` is already injured this match — the same
/// "narrative branch doesn't apply" shape as `take_shot` skipping its
/// beat-keeper roll on an off-target effort, not a violation of the
/// unconditional-fixed-order rule (which governs whole-population draws
/// like `build_xi`'s, not per-event narrative branches).
#[allow(clippy::too_many_arguments)]
fn maybe_contact_injury(
    injury_rng: &mut Rng,
    victim: &XiPlayer,
    culprit_aggression: u8,
    minute: f64,
    k: &Knobs,
    injuries: &mut Vec<InjuryOutcome>,
    stream: &mut Vec<MatchEvent>,
    side: Side,
    zone: Zone,
) {
    if victim.injured_from_minute.get().is_some() {
        return;
    }
    let intensity =
        (1.0 + k.injury_aggression_scale * (culprit_aggression as f64 - 50.0) / 50.0).max(0.0);
    let p = (k.injury_rate * k.injury_base_contact * victim.contact_injury_mult * intensity)
        .clamp(0.0, 1.0);
    if let Some(days_out) = roll_injury(injury_rng, p, k) {
        victim.injured_from_minute.set(Some(minute));
        injuries.push(InjuryOutcome {
            player: victim.pid,
            days_out,
        });
        push_injury_beat(stream, minute, side, zone, victim.pid, days_out);
    }
}

/// The Trace half of an injury (`MATCH_MODEL.md` §9): the same resolved
/// layoff, plus the minute it happened, which `InjuryOutcome` does not carry
/// and does not need to — a fold input records what outlives the match, a
/// Trace records its telling.
fn push_injury_beat(
    stream: &mut Vec<MatchEvent>,
    minute: f64,
    side: Side,
    zone: Zone,
    player: PlayerId,
    days_out: u16,
) {
    stream.push(MatchEvent {
        minute: minute as u8,
        side,
        zone,
        kind: MatchEventKind::Injury { days_out },
        actor: player,
        opponent: None,
    });
}

/// The effective-attribute multiplier from an in-match injury
/// (`MATCH_MODEL.md` §14, T10, identity `1.0`): once a player's
/// `injured_from_minute` has been reached, they "continue at reduced
/// effectiveness for the remainder" (§2.5) rather than leaving the pitch —
/// the forced substitution is T12's.
fn impairment_mult(player: &XiPlayer, minute: f64, k: &Knobs) -> f64 {
    match player.injured_from_minute.get() {
        Some(onset) if minute >= onset => k.injury_impairment_mult,
        _ => 1.0,
    }
}

/// Fires any of `xi`'s pre-rolled ambient injuries whose onset minute has
/// now been reached (`MATCH_MODEL.md` §14, T10) — called once per possession
/// step after `minute` advances. A player already injured by the contact
/// channel keeps that earlier onset; the ambient roll is simply discarded
/// rather than overwriting it (first injury wins, `MATCH_MODEL.md` §12's
/// "never shorten" spirit applied to onset time instead of duration).
fn fire_due_ambient_injuries(
    xi: &[XiPlayer],
    minute: f64,
    injuries: &mut Vec<InjuryOutcome>,
    stream: &mut Vec<MatchEvent>,
    side: Side,
    zone: Zone,
) {
    for player in xi {
        if let Some((onset, days_out)) = player.pending_ambient.get()
            && minute >= onset
        {
            player.pending_ambient.set(None);
            if player.injured_from_minute.get().is_none() {
                player.injured_from_minute.set(Some(onset));
                injuries.push(InjuryOutcome {
                    player: player.pid,
                    days_out,
                });
                // An ambient injury has no location of its own (it is a
                // muscle going, not a challenge), so it reports wherever play
                // happens to be — the zone is context for the beat, and the
                // commentary never quotes it for this kind.
                push_injury_beat(stream, minute, side, zone, player.pid, days_out);
            }
        }
    }
}

/// The outcome of a foul check (`MATCH_MODEL.md` §15, T11): either no foul
/// fired, or one did — carrying whatever card (if any) the severity draw
/// resolved. A foul that draws no card is still a foul (§2.6: "buys the
/// defending side a reset at the cost of card risk" — most fouls draw no
/// card at all).
enum FoulResult {
    NoFoul,
    Foul(Option<Card>),
}

/// The foul-and-card contest (`MATCH_MODEL.md` §15, T11), rolled on
/// `defender` after a take-on resolves (either way) or a failed pass in a
/// pressed zone. `p_foul` is the schema §6 #8 signature (↑ Aggression, ↓
/// Composure/Decisions) plus the two 2e modulators (a High press fouls
/// more, via `press_mult`; tired legs foul more, via `defender_fatigue`),
/// scaled by `foul_rate` (identity `0.0`: no foul ever fires, §2.1). Draws
/// the check unconditionally, then — only if it fires — the severity (a
/// single categorical roll for red/yellow/none), the same
/// conditional-on-outcome shape `roll_injury` already uses. No-ops (no draw
/// at all) if `defender` is already sent off — an XI-shrunk side draws no
/// further fouls off a player who isn't on the pitch.
#[allow(clippy::too_many_arguments)]
fn maybe_foul(
    foul_rng: &mut Rng,
    defender: &XiPlayer,
    defender_fatigue: f64,
    press_mult: f64,
    minute: f64,
    k: &Knobs,
    cards: &mut Vec<CardOutcome>,
) -> FoulResult {
    if defender.sent_off_from_minute.get().is_some() {
        return FoulResult::NoFoul;
    }
    let aggression = defender.attrs.get(Attribute::Aggression) as f64;
    let composure_decisions = 0.5
        * (defender.attrs.get(Attribute::Composure) as f64
            + defender.attrs.get(Attribute::Decisions) as f64);
    let logit = k.foul_base + k.foul_aggression_scale * (aggression - 50.0) / 50.0
        - k.foul_composure_scale * (composure_decisions - 50.0) / 50.0
        + k.foul_press_scale * (press_mult - 1.0)
        + k.foul_fatigue_scale * (1.0 - defender_fatigue);
    let p_foul = (k.foul_rate * contest::sigmoid(logit)).clamp(0.0, 1.0);
    if foul_rng.f64() >= p_foul {
        return FoulResult::NoFoul;
    }

    let foul_count = defender.foul_count.get() + 1;
    defender.foul_count.set(foul_count);
    let already_yellow = defender.has_yellow.get();
    let p_red = k.foul_red_base.max(0.0);
    // A player already cautioned draws from a separate, much lower
    // probability (`foul_second_yellow_base`) rather than the fresh-yellow
    // formula's repeat/aggression bumps — see the knob's own doc comment
    // for why reusing that formula let one repeat fouler's second-yellow
    // rate snowball.
    let p_second_band = if already_yellow {
        k.foul_second_yellow_base.max(0.0)
    } else {
        (k.foul_yellow_base
            + k.foul_repeat_scale * (foul_count - 1) as f64
            + k.foul_yellow_aggression_scale * (aggression - 50.0) / 50.0)
            .max(0.0)
    };
    let roll = foul_rng.f64();
    let card = if roll < p_red {
        Some(if already_yellow {
            Card::SecondYellow
        } else {
            Card::Red
        })
    } else if roll < p_red + p_second_band {
        Some(if already_yellow {
            Card::SecondYellow
        } else {
            Card::Yellow
        })
    } else {
        None
    };
    if let Some(c) = card {
        cards.push(CardOutcome {
            player: defender.pid,
            card: c,
            minute: minute as u8,
        });
        match c {
            Card::Yellow => defender.has_yellow.set(true),
            Card::SecondYellow | Card::Red => defender.sent_off_from_minute.set(Some(minute)),
        }
    }
    FoulResult::Foul(card)
}

/// Per-contest team-quality means (the support term, `MATCH_MODEL.md` §4),
/// precomputed once per match per side — only for the contests that are
/// actually blended (the actor's attacking side of pass/take-on/cross/shot).
#[derive(Debug, Clone, Copy)]
struct TeamMeans {
    pass_atk: f64,
    takeon_atk: f64,
    cross_atk: f64,
    finish_atk: f64,
    header_atk: f64,
    /// This side's `Mid` → `AttC`/`AttW` lateral-split probability
    /// (`MATCH_MODEL.md` §10 item 1's formation-coupling): `Knobs::p_wide`
    /// scaled by how this XI's actual role shape compares to the reference
    /// shape the knob was fitted against (see `formation_p_wide`).
    p_wide: f64,
}

/// Recomputed against only the players still `on_pitch` at `minute`
/// (`MATCH_MODEL.md` §15, T11: "presence sampling renormalizes over ten...
/// team means recompute") — with nobody sent off, this is every XI slot, so
/// the identity `foul_rate: 0.0` setting (nobody ever leaves) reproduces the
/// pre-2e reading bit-for-bit.
fn team_means(xi: &[XiPlayer], minute: f64, k: &Knobs) -> TeamMeans {
    let on: Vec<&XiPlayer> = xi.iter().filter(|p| on_pitch(p, minute)).collect();
    let n = on.len() as f64;
    let mean =
        |w: &[(Attribute, f64)]| on.iter().map(|p| contest::score(&p.attrs, w)).sum::<f64>() / n;
    let roles: Vec<Role> = on.iter().map(|p| p.role).collect();
    TeamMeans {
        pass_atk: mean(contest::PASS_ATK),
        takeon_atk: mean(contest::TAKEON_ATK),
        cross_atk: mean(contest::CROSS_ATK),
        finish_atk: mean(contest::FINISH_ATK),
        header_atk: mean(contest::HEADER_ATK),
        p_wide: formation_p_wide(&roles, k),
    }
}

/// The role shape the global presence table and every `Knobs` split
/// probability (including `p_wide`) were fitted against — the notebook's
/// fixed calibration XI (`resolve::notebook_parity`'s `FIXED_XI`), not any
/// of the four real `FORMATIONS`. A lineup shaped exactly like this one
/// gets `k.p_wide` back unchanged; every other shape scales relative to it.
const REFERENCE_XI_ROLES: [Role; 11] = [
    Role::Gk,
    Role::Cb,
    Role::Cb,
    Role::Fb,
    Role::Fb,
    Role::Dm,
    Role::Cm,
    Role::Am,
    Role::W,
    Role::W,
    Role::St,
];

/// Share of this role set's total `AttC` + `AttW` attacking presence
/// (`MATCH_MODEL.md` §6's existing, unedited table) that sits in `AttW` — a
/// team's structural wide-outlet strength, purely a function of who's on
/// the pitch.
pub(super) fn wide_presence_share(roles: &[Role]) -> f64 {
    let (mut attc, mut attw) = (0u32, 0u32);
    for &role in roles {
        attc += zone::attacking_presence(role, Zone::AttC);
        attw += zone::attacking_presence(role, Zone::AttW);
    }
    let total = attc + attw;
    if total == 0 {
        0.5
    } else {
        attw as f64 / total as f64
    }
}

/// `MATCH_MODEL.md` §10 item 1 ("presence table → formation coupling"):
/// couple the `Mid` → `AttC`/`AttW` lateral split to the formation actually
/// fielded, using only the already-fitted presence table and `p_wide` knob
/// — no new shape-finding numbers, which the design doc reserves for real
/// calibration (`match_engine.rs`'s own doc comment: nothing here re-guesses
/// the shape-finding). A winger-less back three routes less of its play
/// into a zone it has no specialist for, same as a wide-heavy 4-3-3 routes
/// more.
fn formation_p_wide(roles: &[Role], k: &Knobs) -> f64 {
    let reference = wide_presence_share(&REFERENCE_XI_ROLES);
    let team = wide_presence_share(roles);
    (k.p_wide * team / reference).clamp(0.0, 1.0)
}

fn side_index(s: Side) -> usize {
    match s {
        Side::Home => 0,
        Side::Away => 1,
    }
}

fn other_side(s: Side) -> Side {
    match s {
        Side::Home => Side::Away,
        Side::Away => Side::Home,
    }
}

/// The zone a turnover's winner restarts in (`MATCH_MODEL.md` §3): lose it
/// deep and the opponent wins it high; lose it high and they win it deep.
/// `pub(super)` so `calibrate`'s turnover-won-by-zone telemetry cut
/// (`TACTICS_MODEL.md` §8, T7) reads the same mapping rather than a second
/// encoding of it.
pub(super) fn mirrored_zone(zone: Zone) -> Zone {
    match zone {
        Zone::Def => Zone::AttC,
        Zone::Mid => Zone::Mid,
        Zone::AttC => Zone::Def,
        Zone::AttW => Zone::Def,
        Zone::Box => Zone::Def,
    }
}

/// Turnover mirroring (`MATCH_MODEL.md` §3): possession flips and the winner
/// restarts in the mirrored zone.
fn turnover(poss: Side, zone: Zone) -> (Side, Zone) {
    (other_side(poss), mirrored_zone(zone))
}

/// Sample a slot index from `xi` weighted by zone presence (`MATCH_MODEL.md`
/// §6). `presence` selects the attacking or defending table. A player no
/// longer `on_pitch` at `minute` (`MATCH_MODEL.md` §15, T11: a red card)
/// gets weight zero — presence sampling renormalizes over whoever remains
/// without any change to the presence tables themselves.
fn sample_by_presence(
    xi: &[XiPlayer],
    zone: Zone,
    presence: fn(Role, Zone) -> u32,
    minute: f64,
    rng: &mut Rng,
) -> usize {
    let weights: Vec<u32> = xi
        .iter()
        .map(|p| {
            if on_pitch(p, minute) {
                presence(p.role, zone)
            } else {
                0
            }
        })
        .collect();
    let total: u32 = weights.iter().sum();
    debug_assert!(
        total > 0,
        "zone must have nonzero presence for some slot role in the lineup"
    );
    let mut draw = rng.below(total);
    for (i, &w) in weights.iter().enumerate() {
        if draw < w {
            return i;
        }
        draw -= w;
    }
    unreachable!("draw < total by construction")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Pass,
    TakeOn,
    Cross,
    LongShot,
}

/// A weighted choice per zone, biased by the on-ball actor's attributes
/// (dribblers take on more, crossers cross more, finishers shoot from range
/// more), and by the actor's side's own Tempo/Width tactics
/// (`TACTICS_MODEL.md` §3: `w_longshot`/`w_takeon`/`w_cross` multipliers —
/// identity `1.0` at neutral, so this is bit-identical to the pre-2e engine
/// when `se` is `SideEffects::identity()`).
fn select_action(
    zone: Zone,
    actor: &XiPlayer,
    rng: &mut Rng,
    k: &Knobs,
    se: &SideEffects,
) -> Action {
    match zone {
        Zone::Def => Action::Pass,
        Zone::Mid => weighted_choice(
            &[
                (Action::Pass, k.w_pass_mid),
                (
                    Action::TakeOn,
                    k.w_takeon_mid
                        * se.w_takeon_mult
                        * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
                ),
            ],
            rng,
        ),
        Zone::AttC => weighted_choice(
            &[
                (Action::Pass, k.w_pass_attc),
                (
                    Action::TakeOn,
                    k.w_takeon_attc
                        * se.w_takeon_mult
                        * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
                ),
                (
                    Action::LongShot,
                    k.w_longshot_attc
                        * se.w_longshot_mult
                        * (actor.attrs.get(Attribute::Finishing) as f64 / 50.0),
                ),
            ],
            rng,
        ),
        Zone::AttW => weighted_choice(
            &[
                (
                    Action::Cross,
                    k.w_cross_attw
                        * se.w_cross_mult
                        * (actor.attrs.get(Attribute::Crossing) as f64 / 50.0),
                ),
                (
                    Action::TakeOn,
                    k.w_takeon_attw
                        * se.w_takeon_mult
                        * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
                ),
                (Action::Pass, k.w_pass_attw),
            ],
            rng,
        ),
        Zone::Box => unreachable!("Box is never a dwelling zone — it resolves inline"),
    }
}

fn weighted_choice<T: Copy>(options: &[(T, f64)], rng: &mut Rng) -> T {
    let total: f64 = options.iter().map(|&(_, w)| w.max(0.0)).sum();
    if total <= 0.0 {
        return options[0].0;
    }
    let mut draw = rng.f64() * total;
    for &(item, w) in options {
        let w = w.max(0.0);
        if draw < w {
            return item;
        }
        draw -= w;
    }
    options[options.len() - 1].0
}

#[allow(clippy::too_many_arguments)]
fn take_shot(
    poss: Side,
    kind: ShotKind,
    source: ShotSource,
    base_q: f64,
    att: &[XiPlayer],
    def_side: &[XiPlayer],
    tm_att: &TeamMeans,
    se_att: &SideEffects,
    minute: f64,
    rng: &mut Rng,
    injury_rng: &mut Rng,
    k: &Knobs,
    home_attacking: bool,
    goals: &mut [u32; 2],
    stream: &mut Vec<MatchEvent>,
    injuries: &mut Vec<InjuryOutcome>,
) -> (Side, Zone) {
    let shooter = &att[sample_by_presence(att, Zone::Box, zone::attacking_presence, minute, rng)];
    let defender =
        &def_side[sample_by_presence(def_side, Zone::Box, zone::defending_presence, minute, rng)];
    // Formation slot 0 is always Gk (formation.rs: "GK first") — unless a red
    // card has sent him off, in which case `current_gk` (MATCH_MODEL.md §15,
    // T11) re-roles an outfielder into goal.
    let gk = current_gk(def_side, minute);

    let mut kind = kind;
    let mut base_q = base_q;
    let minute_u8 = minute as u8;

    // Up to two rebound follow-ups, mirroring the notebook's bounded retry.
    for _ in 0..3 {
        // The aerial duel is a headed shot's two-player contest (§9): the
        // header's defender is its named opponent. A finish/long-range effort
        // or a rebounded knock-down has no single duelling opponent — the
        // keeper it faces is named on the `Save` beat that resolves it.
        let shot_opponent = match kind {
            ShotKind::Header => Some(defender.pid),
            ShotKind::Finish | ShotKind::LongShot => None,
        };
        if kind == ShotKind::Header {
            // The aerial duel's contact-channel injury check (MATCH_MODEL.md
            // §14, T10) — the shooter risks it, the marking defender's
            // Aggression is the intensity. Only reachable once per
            // `take_shot` call: a rebound always mutates `kind` to `Finish`.
            maybe_contact_injury(
                injury_rng,
                shooter,
                defender.attrs.get(Attribute::Aggression),
                minute,
                k,
                injuries,
                stream,
                poss,
                Zone::Box,
            );
        }
        let (atk, d_block, d_gk) = match kind {
            ShotKind::Header => (
                blend(
                    contest::score(&shooter.attrs, contest::HEADER_ATK),
                    tm_att.header_atk,
                    k,
                ) * fatigue_mult(
                    &shooter.attrs,
                    minute - shooter.entered_at_minute,
                    k,
                    se_att.fatigue_mult,
                    shooter.condition,
                ) * impairment_mult(shooter, minute, k),
                contest::score(&defender.attrs, contest::AERIAL_DEF),
                contest::score(&gk.attrs, contest::GK_AERIAL),
            ),
            ShotKind::Finish | ShotKind::LongShot => (
                blend(
                    contest::score(&shooter.attrs, contest::FINISH_ATK),
                    tm_att.finish_atk,
                    k,
                ) * fatigue_mult(
                    &shooter.attrs,
                    minute - shooter.entered_at_minute,
                    k,
                    se_att.fatigue_mult,
                    shooter.condition,
                ) * impairment_mult(shooter, minute, k),
                contest::score(&defender.attrs, contest::BLOCK_DEF),
                contest::score(&gk.attrs, contest::GK_SHOT),
            ),
        };

        let hb = if home_attacking { k.home_bias } else { 0.0 };
        let p_on =
            contest::sigmoid(k.k_ontarget * (atk - d_block) / k.s + k.b_ontarget + base_q + hb);
        let p_beat = contest::sigmoid(k.k_gk * (atk - d_gk) / k.s + k.b_beat + base_q);

        if rng.f64() < p_on {
            if rng.f64() < p_beat {
                goals[side_index(poss)] += 1;
                stream.push(MatchEvent {
                    minute: minute_u8,
                    side: poss,
                    zone: Zone::Box,
                    kind: MatchEventKind::Shot {
                        kind,
                        source,
                        outcome: ShotOutcome::Goal,
                    },
                    actor: shooter.pid,
                    opponent: shot_opponent,
                });
                return (other_side(poss), Zone::Mid); // conceding side kicks off
            }
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone: Zone::Box,
                kind: MatchEventKind::Shot {
                    kind,
                    source,
                    outcome: ShotOutcome::Saved,
                },
                actor: shooter.pid,
                opponent: shot_opponent,
            });
            let rebound = rng.f64() < k.p_rebound;
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone: Zone::Box,
                kind: MatchEventKind::Save { parried: rebound },
                // The save is the shooter-vs-keeper contest; the keeper is
                // the named opponent (the beat's `side` is the attacking
                // side, so `actor` stays the shooter).
                actor: shooter.pid,
                opponent: Some(gk.pid),
            });
            if rebound {
                kind = ShotKind::Finish;
                base_q = k.q_rebound;
                continue;
            }
            return (other_side(poss), Zone::Def); // keeper collects
        }
        let outcome = if rng.f64() < k.p_off_frac {
            ShotOutcome::Off
        } else {
            ShotOutcome::Blocked
        };
        stream.push(MatchEvent {
            minute: minute_u8,
            side: poss,
            zone: Zone::Box,
            kind: MatchEventKind::Shot {
                kind,
                source,
                outcome,
            },
            actor: shooter.pid,
            opponent: shot_opponent,
        });
        return (other_side(poss), Zone::Def); // off / blocked → cleared
    }
    (other_side(poss), Zone::Def)
}

#[allow(clippy::too_many_arguments)]
fn step(
    poss: Side,
    zone: Zone,
    home: &[XiPlayer],
    away: &[XiPlayer],
    tm: &[TeamMeans; 2],
    se: &[SideEffects; 2],
    minute: f64,
    rng: &mut Rng,
    injury_rng: &mut Rng,
    foul_rng: &mut Rng,
    k: &Knobs,
    goals: &mut [u32; 2],
    stream: &mut Vec<MatchEvent>,
    injuries: &mut Vec<InjuryOutcome>,
    cards: &mut Vec<CardOutcome>,
) -> (Side, Zone) {
    let (att, def_side) = match poss {
        Side::Home => (home, away),
        Side::Away => (away, home),
    };
    // Fast path: with nobody sent off yet, reuse the once-per-match means
    // (bit-identical to the pre-T11 reading). Only recompute — filtered to
    // whoever's still on the pitch — once a red card has actually shrunk
    // this side (MATCH_MODEL.md §15's "team means recompute").
    let tm_att: TeamMeans = if att.iter().any(|p| p.sent_off_from_minute.get().is_some()) {
        team_means(att, minute, k)
    } else {
        tm[side_index(poss)]
    };
    let se_att = &se[side_index(poss)];
    let se_def = &se[side_index(other_side(poss))];
    let home_attacking = poss == Side::Home;
    let minute_u8 = minute as u8;

    let actor = &att[sample_by_presence(att, zone, zone::attacking_presence, minute, rng)];
    let defender =
        &def_side[sample_by_presence(def_side, zone, zone::defending_presence, minute, rng)];
    let action = select_action(zone, actor, rng, k, se_att);

    // Pressing/Mentality's bias term (`TACTICS_MODEL.md` §3): the attacker's
    // own posture, plus the defender's own defensive posture negated — a
    // positive `def_bias_by_zone` entry means *their* defence is better in
    // this (possessor's) zone, which subtracts from the attacker's success.
    let tactics_bias = se_att.atk_bias - se_def.def_bias_by_zone[zone.index()];

    match action {
        Action::Pass => {
            let atk = blend(
                contest::score(&actor.attrs, contest::PASS_ATK),
                tm_att.pass_atk,
                k,
            ) * fatigue_mult(
                &actor.attrs,
                minute - actor.entered_at_minute,
                k,
                se_att.fatigue_mult,
                actor.condition,
            ) * impairment_mult(actor, minute, k);
            let def_fatigue = fatigue_mult(
                &defender.attrs,
                minute - defender.entered_at_minute,
                k,
                se_def.fatigue_mult,
                defender.condition,
            );
            let dfe = contest::score(&defender.attrs, contest::PASS_DEF)
                * def_fatigue
                * impairment_mult(defender, minute, k);
            let bias = k.b_pass + se_att.b_pass_delta_by_zone[zone.index()] + tactics_bias;
            let success = rng.f64() < contest_p(atk, dfe, bias, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::Pass { success },
                // A pass is played into space/to a teammate — no single
                // named opponent, even when it is cut out (§9).
                actor: actor.pid,
                opponent: None,
            });
            if !success {
                // The foul contest (MATCH_MODEL.md §15, T11): a failed pass
                // outside the actor's own defensive third (build-up deep in
                // Def draws no foul — the challenge there reads as a clean
                // interception, not a physical press) can be a foul instead
                // of a clean turnover.
                if zone != Zone::Def
                    && let FoulResult::Foul(card) = maybe_foul(
                        foul_rng,
                        defender,
                        def_fatigue,
                        se_def.fatigue_mult,
                        minute,
                        k,
                        cards,
                    )
                {
                    stream.push(MatchEvent {
                        minute: minute_u8,
                        side: poss,
                        zone,
                        kind: MatchEventKind::Foul { card },
                        actor: actor.pid,
                        opponent: Some(defender.pid),
                    });
                    return (poss, zone);
                }
                return turnover(poss, zone);
            }
            match zone {
                Zone::Def => (
                    poss,
                    if rng.f64() < (k.p_def_advance * se_att.advance_mult).clamp(0.0, 1.0) {
                        Zone::Mid
                    } else {
                        Zone::Def
                    },
                ),
                Zone::Mid => {
                    let p_mid_advance =
                        (k.p_mid_advance * se_att.advance_mult * se_def.opp_mid_advance_mult)
                            .clamp(0.0, 1.0);
                    if rng.f64() < p_mid_advance {
                        (
                            poss,
                            if rng.f64() < (tm_att.p_wide * se_att.p_wide_mult).clamp(0.0, 1.0) {
                                Zone::AttW
                            } else {
                                Zone::AttC
                            },
                        )
                    } else {
                        (poss, Zone::Mid)
                    }
                }
                Zone::AttC => {
                    let p_penetrate =
                        (k.p_attc_penetrate * se_att.penetrate_mult * se_def.opp_penetrate_mult)
                            .clamp(0.0, 1.0);
                    if rng.f64() < p_penetrate {
                        take_shot(
                            poss,
                            ShotKind::Finish,
                            ShotSource::Through,
                            k.q_through,
                            att,
                            def_side,
                            &tm_att,
                            se_att,
                            minute,
                            rng,
                            injury_rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
                            injuries,
                        )
                    } else if rng.f64() < 0.5 {
                        (poss, Zone::Mid)
                    } else {
                        (poss, Zone::AttC)
                    }
                }
                Zone::AttW => {
                    if rng.f64() < 0.5 {
                        (poss, Zone::AttC)
                    } else {
                        (poss, Zone::Mid)
                    }
                }
                Zone::Box => unreachable!("Box is never a dwelling zone"),
            }
        }
        Action::TakeOn => {
            let atk = blend(
                contest::score(&actor.attrs, contest::TAKEON_ATK),
                tm_att.takeon_atk,
                k,
            ) * fatigue_mult(
                &actor.attrs,
                minute - actor.entered_at_minute,
                k,
                se_att.fatigue_mult,
                actor.condition,
            ) * impairment_mult(actor, minute, k);
            let def_fatigue = fatigue_mult(
                &defender.attrs,
                minute - defender.entered_at_minute,
                k,
                se_def.fatigue_mult,
                defender.condition,
            );
            let dfe = contest::score(&defender.attrs, contest::TAKEON_DEF)
                * def_fatigue
                * impairment_mult(defender, minute, k);
            let bias = k.b_takeon + tactics_bias;
            let success = rng.f64() < contest_p(atk, dfe, bias, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::TakeOn { success },
                // A take-on (and its failure, the tackle) is the dribbler-vs-
                // marker contest: the sampled defender is the named opponent.
                actor: actor.pid,
                opponent: Some(defender.pid),
            });
            if !success {
                // The tackle's contact-channel injury check (MATCH_MODEL.md
                // §14, T10): the dribbler risks it, the tackler's Aggression
                // is the intensity.
                maybe_contact_injury(
                    injury_rng,
                    actor,
                    defender.attrs.get(Attribute::Aggression),
                    minute,
                    k,
                    injuries,
                    stream,
                    poss,
                    zone,
                );
            }
            // The foul contest (MATCH_MODEL.md §15, T11): rolled whichever
            // way the take-on resolved (§2.6: "a take-on resolves (either
            // way)") — a cynical foul after being beaten, or a mistimed
            // tackle that wins the ball dirtily, both stop play instead of
            // the actual outcome above.
            if let FoulResult::Foul(card) = maybe_foul(
                foul_rng,
                defender,
                def_fatigue,
                se_def.fatigue_mult,
                minute,
                k,
                cards,
            ) {
                stream.push(MatchEvent {
                    minute: minute_u8,
                    side: poss,
                    zone,
                    kind: MatchEventKind::Foul { card },
                    actor: actor.pid,
                    opponent: Some(defender.pid),
                });
                return (poss, zone);
            }
            if !success {
                return turnover(poss, zone);
            }
            match zone {
                Zone::Mid => {
                    let p_mid_advance =
                        (k.p_mid_advance * se_att.advance_mult * se_def.opp_mid_advance_mult)
                            .clamp(0.0, 1.0);
                    if rng.f64() < p_mid_advance {
                        (
                            poss,
                            if rng.f64() < (tm_att.p_wide * se_att.p_wide_mult).clamp(0.0, 1.0) {
                                Zone::AttW
                            } else {
                                Zone::AttC
                            },
                        )
                    } else {
                        (poss, Zone::Mid)
                    }
                }
                Zone::AttC => {
                    let p_dribble_box =
                        (k.p_attc_dribble_box * se_att.penetrate_mult).clamp(0.0, 1.0);
                    if rng.f64() < p_dribble_box {
                        take_shot(
                            poss,
                            ShotKind::Finish,
                            ShotSource::Dribble,
                            k.q_dribble,
                            att,
                            def_side,
                            &tm_att,
                            se_att,
                            minute,
                            rng,
                            injury_rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
                            injuries,
                        )
                    } else {
                        (poss, Zone::AttC)
                    }
                }
                Zone::AttW => {
                    if rng.f64() < k.p_attw_cutback {
                        take_shot(
                            poss,
                            ShotKind::Finish,
                            ShotSource::Cutback,
                            k.q_cutback,
                            att,
                            def_side,
                            &tm_att,
                            se_att,
                            minute,
                            rng,
                            injury_rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
                            injuries,
                        )
                    } else if rng.f64() < k.p_attw_cut_inside {
                        (poss, Zone::AttC)
                    } else {
                        (poss, Zone::AttW)
                    }
                }
                Zone::Def | Zone::Box => {
                    unreachable!("take-on never selected in Def; Box is never dwelt in")
                }
            }
        }
        Action::Cross => {
            let atk = blend(
                contest::score(&actor.attrs, contest::CROSS_ATK),
                tm_att.cross_atk,
                k,
            ) * fatigue_mult(
                &actor.attrs,
                minute - actor.entered_at_minute,
                k,
                se_att.fatigue_mult,
                actor.condition,
            ) * impairment_mult(actor, minute, k);
            let dfe = contest::score(&defender.attrs, contest::CROSS_DEF)
                * fatigue_mult(
                    &defender.attrs,
                    minute - defender.entered_at_minute,
                    k,
                    se_def.fatigue_mult,
                    defender.condition,
                )
                * impairment_mult(defender, minute, k);
            let bias = k.b_cross_delivery + tactics_bias;
            let success = rng.f64() < contest_p(atk, dfe, bias, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::Cross { success },
                // The delivery itself has no single duelling opponent — the
                // aerial duel it sets up is the following headed `Shot`'s.
                actor: actor.pid,
                opponent: None,
            });
            if success {
                take_shot(
                    poss,
                    ShotKind::Header,
                    ShotSource::Cross,
                    k.q_header,
                    att,
                    def_side,
                    &tm_att,
                    se_att,
                    minute,
                    rng,
                    injury_rng,
                    k,
                    home_attacking,
                    goals,
                    stream,
                    injuries,
                )
            } else {
                stream.push(MatchEvent {
                    minute: minute_u8,
                    side: poss,
                    zone,
                    kind: MatchEventKind::Clearance,
                    // A cleared cross belongs to the attacking beat (its
                    // `side` is the crossing side); it is not a duel, so the
                    // crosser stays the actor and there is no named opponent.
                    actor: actor.pid,
                    opponent: None,
                });
                turnover(poss, zone)
            }
        }
        Action::LongShot => take_shot(
            poss,
            ShotKind::LongShot,
            ShotSource::Long,
            k.q_long,
            att,
            def_side,
            &tm_att,
            se_att,
            minute,
            rng,
            injury_rng,
            k,
            home_attacking,
            goals,
            stream,
            injuries,
        ),
    }
}

/// `consistency_rng` must be a stream **separate from `rng`**
/// (`MATCH_MODEL.md` §17, T8, §2.1's own-stream rule): the possession loop
/// only ever draws from `rng`, so a caller who always passes a fresh,
/// independently-derived `consistency_rng` (e.g. `derive_stream(seed,
/// CONSISTENCY_NS | fixture.id)`) gets the §4 neutral-adjacent guarantee —
/// nothing here can perturb `rng`'s own draw sequence, regardless of
/// `k.consistency_sigma_max`. Takes `k` explicitly (the `notebook_parity`/
/// `simulate` precedent) so a caller can pin `consistency_sigma_max: 0.0`
/// independent of whatever `Knobs::default()` currently is in production —
/// exactly what the T5/T6 identity tests need to keep asserting the
/// pre-Consistency baseline. `conditions` (`MATCH_MODEL.md` §13, T9) is a
/// pre-computed `PlayerId -> condition` map — RNG-free, since recovery is a
/// deterministic function of the calendar, so it needs no stream of its own;
/// a player absent from the map (or the whole map empty, the identity
/// setting) reads full condition. `injury_rng` (`MATCH_MODEL.md` §14, T10)
/// is a third stream, independent of both `rng` and `consistency_rng`;
/// `foul_rng` (`MATCH_MODEL.md` §15, T11) is a fourth, independent of all
/// three; `today` is only used to compute each player's age for the ambient
/// channel.
#[allow(clippy::too_many_arguments)]
pub fn play_match(
    world: &World,
    home_lineup: &Lineup,
    away_lineup: &Lineup,
    rng: &mut Rng,
    consistency_rng: &mut Rng,
    injury_rng: &mut Rng,
    foul_rng: &mut Rng,
    k: &Knobs,
    conditions: &BTreeMap<PlayerId, f64>,
    today: GameDate,
) -> MatchOutcome {
    let home = build_xi(
        world,
        home_lineup,
        consistency_rng,
        injury_rng,
        today,
        k,
        conditions,
    );
    let away = build_xi(
        world,
        away_lineup,
        consistency_rng,
        injury_rng,
        today,
        k,
        conditions,
    );
    // MATCH_MODEL.md §16, T12: the bench is built from the same
    // unconditional whole-squad draw `build_xi` starts (see `build_bench`'s
    // own doc comment) — an empty `Lineup.bench` (the identity) draws
    // nothing extra and returns an empty `Vec`.
    let home_bench = build_bench(
        world,
        home_lineup,
        consistency_rng,
        injury_rng,
        today,
        k,
        conditions,
    );
    let away_bench = build_bench(
        world,
        away_lineup,
        consistency_rng,
        injury_rng,
        today,
        k,
        conditions,
    );
    simulate(
        home,
        away,
        home_bench,
        away_bench,
        home_lineup.tactics,
        away_lineup.tactics,
        &home_lineup.sub_plan,
        &away_lineup.sub_plan,
        rng,
        injury_rng,
        foul_rng,
        k,
    )
}

/// Fixed decision-point checkpoints within the second half (`MATCH_MODEL.md`
/// §16): half-time itself is the natural boundary between `simulate`'s two
/// `for half` iterations, handled separately below.
pub const SUB_CHECKPOINTS: [f64; 3] = [60.0, 70.0, 80.0];

/// Whether a single `SubCondition` clause currently holds (`MATCH_MODEL.md`
/// §16, T12/§2.7's vocabulary) — a pure read of already-resolved match
/// state, never a draw. `press_mult` is this side's own current
/// `SideEffects::fatigue_mult`, needed to read `PlayerConditionBelow`'s live
/// fatigue exactly the way every open-play contest already computes it.
fn condition_holds(
    cond: &SubCondition,
    minute: f64,
    goals: &[u32; 2],
    side: Side,
    xi: &[XiPlayer],
    k: &Knobs,
    press_mult: f64,
) -> bool {
    match *cond {
        SubCondition::MinuteAtLeast(m) => minute >= m as f64,
        SubCondition::Score(state) => {
            let (own, opp) = match side {
                Side::Home => (goals[0], goals[1]),
                Side::Away => (goals[1], goals[0]),
            };
            match state {
                ScoreState::Trailing => own < opp,
                ScoreState::Level => own == opp,
                ScoreState::Leading => own > opp,
            }
        }
        SubCondition::PlayerConditionBelow(pid, threshold) => xi
            .iter()
            .find(|p| p.pid == pid && on_pitch(p, minute))
            .map(|p| {
                let live_fatigue = fatigue_mult(
                    &p.attrs,
                    minute - p.entered_at_minute,
                    k,
                    press_mult,
                    p.condition,
                );
                live_fatigue * 100.0 < threshold as f64
            })
            .unwrap_or(false),
        SubCondition::PlayerInjured(pid) => xi
            .iter()
            .any(|p| p.pid == pid && p.injured_from_minute.get().is_some()),
        SubCondition::ManDown => xi.iter().filter(|p| on_pitch(p, minute)).count() < XI,
    }
}

/// Evaluates one side's substitution/tactics-change plan at a decision
/// point (`MATCH_MODEL.md` §16, T12: half-time, 60'/70'/80', or forced
/// immediately on an injury/red card) — rules run in list order, each
/// firing if every one of its `conditions` currently holds. RNG-free by
/// construction (§16's own requirement, and the reason `build_xi`/
/// `build_bench` pre-roll every dressed player's Consistency/ambient-injury
/// state at kickoff): every condition and action here reads or mutates
/// already-resolved match state, never draws. A `Substitute` whose
/// `player_out` is no longer on the pitch (already subbed or sent off), or
/// whose `player_in` is no longer on the bench (already brought on), is a
/// silent no-op — the same "narrative branch doesn't apply" shape the rest
/// of the engine already uses for a stale condition, not a violation of
/// "the plan is honoured deterministically" (the plan *is* still followed;
/// there is simply nothing left for that particular rule to do).
#[allow(clippy::too_many_arguments)]
fn evaluate_decision_point(
    minute: f64,
    goals: &[u32; 2],
    side: Side,
    xi: &mut [XiPlayer],
    bench: &mut Vec<XiPlayer>,
    tactics: &mut Tactics,
    plan: &[SubRule],
    subs_used: &mut u8,
    k: &Knobs,
    press_mult: f64,
    departed: &mut Vec<(PlayerId, Side, Role, u8)>,
    stream: &mut Vec<MatchEvent>,
) {
    for rule in plan {
        let holds = rule
            .conditions
            .iter()
            .all(|c| condition_holds(c, minute, goals, side, xi, k, press_mult));
        if !holds {
            continue;
        }
        match rule.action {
            SubAction::Substitute {
                player_out,
                player_in,
            } => {
                if *subs_used as usize >= MAX_SUBSTITUTIONS {
                    continue;
                }
                let Some(slot) = xi
                    .iter()
                    .position(|p| p.pid == player_out && on_pitch(p, minute))
                else {
                    continue;
                };
                let Some(bench_idx) = bench.iter().position(|p| p.pid == player_in) else {
                    continue;
                };
                let outgoing_minutes = (minute - xi[slot].entered_at_minute)
                    .round()
                    .clamp(0.0, 90.0) as u8;
                departed.push((player_out, side, xi[slot].role, outgoing_minutes));
                let mut incoming = bench.remove(bench_idx);
                incoming.entered_at_minute = minute;
                xi[slot] = incoming;
                *subs_used += 1;
                stream.push(MatchEvent {
                    minute: minute as u8,
                    side,
                    zone: Zone::Mid,
                    kind: MatchEventKind::Substitution { player_out },
                    actor: player_in,
                    opponent: None,
                });
            }
            SubAction::SetMentality(m) => tactics.mentality = m,
            SubAction::SetTempo(t) => tactics.tempo = t,
            SubAction::SetWidth(w) => tactics.width = w,
            SubAction::SetPressing(p) => tactics.pressing = p,
        }
    }
}

/// The possession loop over two already-built XIs, independent of
/// `World`/`Lineup`/formation selection — the seam the port-parity harness
/// (`MATCH_MODEL.md` §10 diagnosis) needs to feed notebook-equivalent test
/// inputs straight through the real Rust resolution loop. Takes `k`
/// explicitly (rather than defaulting internally) so that harness can pin
/// the notebook's own fitted snapshot independent of whatever
/// `Knobs::default()` currently is in production. Owns `home`/`away`/the
/// bench `Vec`s (`MATCH_MODEL.md` §16, T12) rather than borrowing them,
/// since a substitution replaces a slot's `XiPlayer` outright — `step`/
/// `take_shot`/`sample_by_presence`/`team_means` still only ever see a
/// borrowed `&[XiPlayer]` for the duration of one segment between decision
/// points, so none of their own signatures changed for this.
///
/// Tactics resolution (`TACTICS_MODEL.md` §3) happens once here initially,
/// the same "resolve once per match" shape `team_means` already
/// established — recomputed only when a decision point's tactics-change
/// action actually fires (§16's tactic-change seam), which the identity
/// setting (`sub_plan` empty on both sides) never does, so this stays
/// consuming no RNG and the §4 neutral-tactics invariant still holds by
/// construction.
#[allow(clippy::too_many_arguments)]
fn simulate(
    mut home: Vec<XiPlayer>,
    mut away: Vec<XiPlayer>,
    mut home_bench: Vec<XiPlayer>,
    mut away_bench: Vec<XiPlayer>,
    home_tactics: Tactics,
    away_tactics: Tactics,
    home_plan: &[SubRule],
    away_plan: &[SubRule],
    rng: &mut Rng,
    injury_rng: &mut Rng,
    foul_rng: &mut Rng,
    k: &Knobs,
) -> MatchOutcome {
    let mut tm = [team_means(&home, 0.0, k), team_means(&away, 0.0, k)];
    let mut home_tactics = home_tactics;
    let mut away_tactics = away_tactics;
    let mut se = resolve_tactics(home_tactics, away_tactics);

    let mut goals = [0u32, 0u32];
    let mut stream = Vec::new();
    let mut injuries = Vec::new();
    let mut cards = Vec::new();
    let mut home_subs_used = 0u8;
    let mut away_subs_used = 0u8;
    // `(player, side, role, minutes)` for anyone ever substituted *off* —
    // their `XiPlayer` is fully replaced in `home`/`away` at that point (a
    // fresh struct occupies the slot from then on), so the final per-slot
    // scan below can no longer see them; this is where their resolved
    // minutes (and role, for the rating clean-sheet gate, §18) survive
    // instead.
    let mut departed: Vec<(PlayerId, Side, Role, u8)> = Vec::new();

    for half in 0..2u8 {
        let start = 45.0 * half as f64;
        let end = 45.0 * (half as f64 + 1.0);
        let mut poss = if half == 0 { Side::Home } else { Side::Away }; // each half kicked off by the appropriate side
        let mut zone = Zone::Mid;
        let mut minute = start;
        while minute < end {
            let prev_minute = minute;
            let injuries_before = injuries.len();
            let cards_before = cards.len();
            let (next_poss, next_zone) = step(
                poss,
                zone,
                &home,
                &away,
                &tm,
                &se,
                minute,
                rng,
                injury_rng,
                foul_rng,
                k,
                &mut goals,
                &mut stream,
                &mut injuries,
                &mut cards,
            );
            poss = next_poss;
            zone = next_zone;
            minute += k.delta;
            // MATCH_MODEL.md §14, T10: fire any ambient injury whose
            // pre-rolled onset the clock has now reached — no RNG here, the
            // draw already happened at kickoff in `build_xi`.
            fire_due_ambient_injuries(&home, minute, &mut injuries, &mut stream, Side::Home, zone);
            fire_due_ambient_injuries(&away, minute, &mut injuries, &mut stream, Side::Away, zone);

            // MATCH_MODEL.md §16, T12: forced evaluation on a new injury or
            // card this tick, attributed to whichever side it actually
            // belongs to (never both, unless a genuinely simultaneous
            // in-tick pair happens to touch both squads).
            let home_forced = injuries[injuries_before..]
                .iter()
                .any(|i| home.iter().any(|p| p.pid == i.player))
                || cards[cards_before..]
                    .iter()
                    .any(|c| home.iter().any(|p| p.pid == c.player));
            let away_forced = injuries[injuries_before..]
                .iter()
                .any(|i| away.iter().any(|p| p.pid == i.player))
                || cards[cards_before..]
                    .iter()
                    .any(|c| away.iter().any(|p| p.pid == c.player));
            let crossed_checkpoint = SUB_CHECKPOINTS
                .iter()
                .any(|&cp| prev_minute < cp && minute >= cp);

            if home_forced || crossed_checkpoint {
                evaluate_decision_point(
                    minute,
                    &goals,
                    Side::Home,
                    &mut home,
                    &mut home_bench,
                    &mut home_tactics,
                    home_plan,
                    &mut home_subs_used,
                    k,
                    se[side_index(Side::Home)].fatigue_mult,
                    &mut departed,
                    &mut stream,
                );
            }
            if away_forced || crossed_checkpoint {
                evaluate_decision_point(
                    minute,
                    &goals,
                    Side::Away,
                    &mut away,
                    &mut away_bench,
                    &mut away_tactics,
                    away_plan,
                    &mut away_subs_used,
                    k,
                    se[side_index(Side::Away)].fatigue_mult,
                    &mut departed,
                    &mut stream,
                );
            }
            if home_forced || away_forced || crossed_checkpoint {
                tm = [team_means(&home, minute, k), team_means(&away, minute, k)];
                se = resolve_tactics(home_tactics, away_tactics);
            }
        }
        if half == 0 {
            // Half-time: a fixed decision point regardless of anything
            // forced during the first half.
            evaluate_decision_point(
                45.0,
                &goals,
                Side::Home,
                &mut home,
                &mut home_bench,
                &mut home_tactics,
                home_plan,
                &mut home_subs_used,
                k,
                se[side_index(Side::Home)].fatigue_mult,
                &mut departed,
                &mut stream,
            );
            evaluate_decision_point(
                45.0,
                &goals,
                Side::Away,
                &mut away,
                &mut away_bench,
                &mut away_tactics,
                away_plan,
                &mut away_subs_used,
                k,
                se[side_index(Side::Away)].fatigue_mult,
                &mut departed,
                &mut stream,
            );
            tm = [team_means(&home, 45.0, k), team_means(&away, 45.0, k)];
            se = resolve_tactics(home_tactics, away_tactics);
        }
    }

    // MATCH_MODEL.md §15/§16, T11/T12: a sent-off player's minutes stop at
    // his dismissal; a substitute's start at his entry (`entered_at_minute`,
    // `0.0` for every starter, the identity). Anyone substituted *off* no
    // longer occupies a slot here at all — `departed` carries their
    // resolved minutes (and role, side) instead. No RNG: every input was
    // already resolved above.
    let mut rated_players: Vec<ratings::RatedPlayer> = home
        .iter()
        .map(|p| (p, Side::Home))
        .chain(away.iter().map(|p| (p, Side::Away)))
        .map(|(p, side)| {
            let mins = match p.sent_off_from_minute.get() {
                Some(off) => (off - p.entered_at_minute).round().clamp(0.0, 90.0) as u8,
                None => (90.0 - p.entered_at_minute).round().clamp(0.0, 90.0) as u8,
            };
            ratings::RatedPlayer {
                pid: p.pid,
                side,
                role: p.role,
                minutes: mins,
            }
        })
        .collect();
    rated_players.extend(
        departed
            .iter()
            .map(|&(pid, side, role, minutes)| ratings::RatedPlayer {
                pid,
                side,
                role,
                minutes,
            }),
    );

    // MATCH_MODEL.md §18, T13: a pure fold over the already-resolved stream
    // — no RNG, so it can run after `stream` is fully built without
    // touching any draw sequence.
    let resolved_ratings = ratings::compute_ratings(
        &stream,
        &rated_players,
        goals[0].min(u8::MAX as u32) as u8,
        goals[1].min(u8::MAX as u32) as u8,
    );
    let minutes: Vec<(PlayerId, u8)> = rated_players.iter().map(|p| (p.pid, p.minutes)).collect();

    MatchOutcome {
        home_goals: goals[0].min(u8::MAX as u32) as u8,
        away_goals: goals[1].min(u8::MAX as u32) as u8,
        stream,
        // MATCH_MODEL.md §14, T10: resolved contact + ambient injuries.
        injuries,
        // MATCH_MODEL.md §15, T11: resolved fouls' cards.
        cards,
        // MATCH_MODEL.md §18, T13: resolved per-player ratings.
        ratings: resolved_ratings,
        minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::derive_stream;

    #[test]
    fn build_xi_reproduces_raw_attributes_bit_for_bit_at_the_consistency_identity() {
        // §2.1: consistency_sigma_max = 0.0 must reproduce every attribute
        // exactly, regardless of the drawn z (unconditional draws still
        // happen — only the resulting multiplier is pinned to 1.0 exactly).
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _s, _d) = crate::worldgen::generate(7, &cfg);
        let club = world.competition.clubs[0];
        let lineup = crate::match_engine::ai_pick_lineup(&world, club);
        let k = Knobs {
            consistency_sigma_max: 0.0,
            ..Knobs::default()
        };
        let mut rng = Rng::seed_from(123);
        let mut injury_rng = Rng::seed_from(456);
        let today = GameDate { days: 0 };
        let xi = build_xi(
            &world,
            &lineup,
            &mut rng,
            &mut injury_rng,
            today,
            &k,
            &BTreeMap::new(),
        );
        for player in &xi {
            let raw = &world.player(player.pid).attributes;
            for attr in Attribute::ALL {
                assert_eq!(
                    player.attrs.get(attr),
                    raw.get(attr),
                    "player {}: {attr:?} moved at the consistency identity",
                    player.pid
                );
            }
        }
    }

    #[test]
    fn build_xi_attaches_condition_from_the_caller_supplied_map() {
        // MATCH_MODEL.md §13, T9: `build_xi` reads each player's condition
        // from the caller's map, defaulting a missing entry to `1.0` (the
        // identity) rather than requiring every player to be present.
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _s, _d) = crate::worldgen::generate(7, &cfg);
        let club = world.competition.clubs[0];
        let lineup = crate::match_engine::ai_pick_lineup(&world, club);
        let k = Knobs::default();
        let named = lineup.players[0];
        let conditions: BTreeMap<PlayerId, f64> = [(named, 0.75)].into_iter().collect();

        let mut rng = Rng::seed_from(123);
        let mut injury_rng = Rng::seed_from(456);
        let today = GameDate { days: 0 };
        let xi = build_xi(
            &world,
            &lineup,
            &mut rng,
            &mut injury_rng,
            today,
            &k,
            &conditions,
        );
        for player in &xi {
            if player.pid == named {
                assert_eq!(
                    player.condition, 0.75,
                    "the mapped player's condition must be read verbatim"
                );
            } else {
                assert_eq!(
                    player.condition, 1.0,
                    "a player absent from the map must default to full condition"
                );
            }
        }
    }

    #[test]
    fn low_consistency_shows_a_visibly_wider_match_to_match_spread_than_high_consistency() {
        // §2.9/MATCH_MODEL.md §17: at equal starting CA (same attribute
        // value), a low-Consistency player's per-match effective attribute
        // must vary more from match to match than a high-Consistency
        // player's — the whole point of the split from Concentration.
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (mut world, _s, _d) = crate::worldgen::generate(7, &cfg);
        let club = world.competition.clubs[0];
        let lineup = crate::match_engine::ai_pick_lineup(&world, club);
        let pid = lineup.players[0];
        world
            .players
            .get_mut(&pid)
            .unwrap()
            .attributes
            .set(Attribute::Passing, 60);
        let k = Knobs::default();

        let today = GameDate { days: 0 };
        let mut low_draws = Vec::new();
        let mut high_draws = Vec::new();
        for seed in 0..500u64 {
            world.players.get_mut(&pid).unwrap().character.consistency = 25;
            let mut rng = derive_stream(seed, 1);
            let mut injury_rng = derive_stream(seed, 2);
            let xi = build_xi(
                &world,
                &lineup,
                &mut rng,
                &mut injury_rng,
                today,
                &k,
                &BTreeMap::new(),
            );
            low_draws.push(
                xi.iter()
                    .find(|p| p.pid == pid)
                    .unwrap()
                    .attrs
                    .get(Attribute::Passing) as f64,
            );

            world.players.get_mut(&pid).unwrap().character.consistency = 90;
            let mut rng2 = derive_stream(seed, 1);
            let mut injury_rng2 = derive_stream(seed, 2);
            let xi2 = build_xi(
                &world,
                &lineup,
                &mut rng2,
                &mut injury_rng2,
                today,
                &k,
                &BTreeMap::new(),
            );
            high_draws.push(
                xi2.iter()
                    .find(|p| p.pid == pid)
                    .unwrap()
                    .attrs
                    .get(Attribute::Passing) as f64,
            );
        }

        fn variance(v: &[f64]) -> f64 {
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / v.len() as f64
        }
        let low_var = variance(&low_draws);
        let high_var = variance(&high_draws);
        assert!(
            low_var > high_var * 2.0,
            "low-consistency (var {low_var:.2}) should spread visibly wider than \
             high-consistency (var {high_var:.2}) at equal starting CA"
        );
    }

    #[test]
    fn turnover_mirrors_zones_per_match_model_table() {
        assert_eq!(turnover(Side::Home, Zone::Def), (Side::Away, Zone::AttC));
        assert_eq!(turnover(Side::Home, Zone::Mid), (Side::Away, Zone::Mid));
        assert_eq!(turnover(Side::Home, Zone::AttC), (Side::Away, Zone::Def));
        assert_eq!(turnover(Side::Home, Zone::AttW), (Side::Away, Zone::Def));
        assert_eq!(turnover(Side::Away, Zone::Def), (Side::Home, Zone::AttC));
    }

    #[test]
    fn weighted_choice_always_picks_the_only_positive_option() {
        let mut rng = Rng::seed_from(1);
        for _ in 0..50 {
            let picked = weighted_choice(
                &[
                    (Action::Pass, 0.0),
                    (Action::TakeOn, 5.0),
                    (Action::Cross, 0.0),
                ],
                &mut rng,
            );
            assert_eq!(picked, Action::TakeOn);
        }
    }

    #[test]
    fn formation_p_wide_is_unchanged_for_the_reference_shape() {
        let k = Knobs::default();
        let p = formation_p_wide(&REFERENCE_XI_ROLES, &k);
        assert!(
            (p - k.p_wide).abs() < 1e-9,
            "the reference shape must reproduce the fitted knob exactly: got {p}, knob {}",
            k.p_wide
        );
    }

    #[test]
    fn formation_p_wide_drops_for_a_winger_less_back_three() {
        let k = Knobs::default();
        // 3-5-2: no W at all — the weakest structural wide outlet among the
        // four real FORMATIONS (MATCH_MODEL.md §10 item 1's premise).
        let three_five_two = fforge_domain::FORMATIONS[3].slots;
        assert_eq!(fforge_domain::FORMATIONS[3].name, "3-5-2");
        let p = formation_p_wide(&three_five_two, &k);
        assert!(
            p < k.p_wide,
            "a winger-less shape must route less often into AttW than the fitted knob: got {p}, knob {}",
            k.p_wide
        );
    }

    #[test]
    fn formation_p_wide_stays_a_probability_for_every_real_formation() {
        let k = Knobs::default();
        for formation in fforge_domain::FORMATIONS {
            let p = formation_p_wide(&formation.slots, &k);
            assert!(
                (0.0..=1.0).contains(&p),
                "{}: formation_p_wide {p} out of [0,1]",
                formation.name
            );
        }
    }

    /// Builds a real home/away pair of XIs via `build_xi` for the T11
    /// ten-man tests below — identity Consistency/Injuries/Fouls (§2.1) so
    /// only the manually-forced dismissal below drives any XI shrinkage.
    fn built_xi_pair() -> (World, Vec<XiPlayer>, Vec<XiPlayer>) {
        let cfg = crate::worldgen::WorldGenConfig {
            num_clubs: 2,
            ..Default::default()
        };
        let (world, _s, _d) = crate::worldgen::generate(7, &cfg);
        let clubs = world.competition.clubs.clone();
        let home_lineup = crate::match_engine::ai_pick_lineup(&world, clubs[0]);
        let away_lineup = crate::match_engine::ai_pick_lineup(&world, clubs[1]);
        let k = Knobs {
            consistency_sigma_max: 0.0,
            injury_rate: 0.0,
            ..Knobs::default()
        };
        let today = GameDate { days: 0 };
        let mut consistency_rng = Rng::seed_from(1);
        let mut injury_rng = Rng::seed_from(2);
        let home = build_xi(
            &world,
            &home_lineup,
            &mut consistency_rng,
            &mut injury_rng,
            today,
            &k,
            &BTreeMap::new(),
        );
        let away = build_xi(
            &world,
            &away_lineup,
            &mut consistency_rng,
            &mut injury_rng,
            today,
            &k,
            &BTreeMap::new(),
        );
        (world, home, away)
    }

    /// `MATCH_MODEL.md` §15, T11: "no code path assumes a full eleven" —
    /// forcing an outfielder off from kickoff (as if red-carded before the
    /// match even started) must let a full 90 minutes simulate without
    /// panicking, and no event may ever name the sent-off player again.
    #[test]
    fn a_ten_man_side_completes_a_full_match_without_panicking() {
        let (_world, home, away) = built_xi_pair();
        let sent_off = home[5].pid; // an outfielder, not the Gk (slot 0)
        home[5].sent_off_from_minute.set(Some(0.0));

        let k = Knobs {
            consistency_sigma_max: 0.0,
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        };
        let mut rng = Rng::seed_from(3);
        let mut injury_rng = Rng::seed_from(4);
        let mut foul_rng = Rng::seed_from(5);
        let outcome = simulate(
            home,
            away,
            Vec::new(),
            Vec::new(),
            Tactics::neutral(),
            Tactics::neutral(),
            &[],
            &[],
            &mut rng,
            &mut injury_rng,
            &mut foul_rng,
            &k,
        );

        assert!(
            !outcome.stream.is_empty(),
            "a ten-man side must still produce a full match's worth of events"
        );
        for event in &outcome.stream {
            assert_ne!(
                event.actor, sent_off,
                "the sent-off player must never be sampled as an actor again"
            );
            assert_ne!(
                event.opponent,
                Some(sent_off),
                "the sent-off player must never be sampled as an opponent again"
            );
        }
        let sent_off_minutes = outcome
            .minutes
            .iter()
            .find(|&&(pid, _)| pid == sent_off)
            .map(|&(_, m)| m);
        assert_eq!(
            sent_off_minutes,
            Some(0),
            "a player sent off at minute 0 must record 0 minutes played"
        );
    }

    /// `MATCH_MODEL.md` §15's named edge case: a red-carded keeper, with no
    /// substitutions yet (T12), forces an outfielder into goal via
    /// `current_gk` — his attributes (not a real keeper's) make the
    /// punishment automatic. Pinned here as a no-panic + no-self-reference
    /// test, since the calibrated *size* of the punishment is a T12+
    /// concern once subs exist to actually replace him.
    #[test]
    fn a_red_carded_keeper_forces_an_outfielder_into_goal() {
        let (_world, home, away) = built_xi_pair();
        let sent_off_gk = home[0].pid; // formation slot 0 is always Gk
        home[0].sent_off_from_minute.set(Some(0.0));

        let k = Knobs {
            consistency_sigma_max: 0.0,
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        };
        let mut rng = Rng::seed_from(6);
        let mut injury_rng = Rng::seed_from(7);
        let mut foul_rng = Rng::seed_from(8);
        let outcome = simulate(
            home,
            away,
            Vec::new(),
            Vec::new(),
            Tactics::neutral(),
            Tactics::neutral(),
            &[],
            &[],
            &mut rng,
            &mut injury_rng,
            &mut foul_rng,
            &k,
        );

        assert!(
            !outcome.stream.is_empty(),
            "a match missing its keeper from kickoff must still simulate to completion"
        );
        for event in &outcome.stream {
            assert_ne!(
                event.actor, sent_off_gk,
                "the sent-off keeper must never be sampled again"
            );
            assert_ne!(
                event.opponent,
                Some(sent_off_gk),
                "the sent-off keeper must never keep goal (or anything else) again"
            );
        }
    }
}

/// Port-parity harness (`MATCH_MODEL.md` §10 diagnosis): does `simulate` —
/// the possession loop, unchanged from the notebook port — reproduce the
/// notebook's own ~2.5-2.9 goals/match when fed the notebook's *own*
/// synthetic-squad generator instead of this crate's `worldgen`? A pass here
/// means the whole gap between real-worldgen gpm (~1.7-2.0) and the
/// notebook's fitted ~2.6-2.7 is an input-distribution effect (real
/// `worldgen::gen_player` + `ai_pick_lineup`'s formation mix), not a bug in
/// this loop — the decisive port-faithfulness-vs-input-distribution check
/// the calibration plan calls for before any knob or presence-table edit.
#[cfg(test)]
mod notebook_parity {
    use super::*;
    use crate::rng::derive_stream;
    use crate::schedule::double_round_robin;
    use fforge_domain::{ClubId, NUM_ATTRIBUTES, ROLE_WEIGHTS, XI};

    /// Verbatim port of the notebook's `gen_player`: base ~ N(club_q, 6)
    /// clamp [25,92]; per attribute, weight 0 -> uniform[8,22], else
    /// N(base + (w-3.0)*4.0, 4.5) clamp [15,96]. Deliberately *not* this
    /// crate's `worldgen::gen_player` (which models age/PA/youth-discount
    /// and uses different shape constants) — parity is meaningless if this
    /// generator drifts from the notebook's.
    fn notebook_gen_player(rng: &mut Rng, role: Role, club_q: f64) -> Attributes {
        let base = rng.normal(club_q, 6.0).clamp(25.0, 92.0);
        let mut values = [0u8; NUM_ATTRIBUTES];
        for attr in Attribute::ALL {
            let w = ROLE_WEIGHTS.weight(role, attr);
            let v = if w == 0 {
                rng.range_i32(8, 22) as f64
            } else {
                rng.normal(base + (w as f64 - 3.0) * 4.0, 4.5)
            };
            values[attr.index()] = v.clamp(15.0, 96.0) as u8;
        }
        Attributes::new(values)
    }

    /// The notebook's fixed calibration XI: one of each outfield archetype
    /// in a shape the global presence table was fitted against, not any of
    /// the four real `FORMATIONS` (`MATCH_MODEL.md` §10 item 1's premise).
    const FIXED_XI: [Role; XI] = [
        Role::Gk,
        Role::Cb,
        Role::Cb,
        Role::Fb,
        Role::Fb,
        Role::Dm,
        Role::Cm,
        Role::Am,
        Role::W,
        Role::W,
        Role::St,
    ];

    fn build_fixed_xi(rng: &mut Rng, club_q: f64) -> Vec<XiPlayer> {
        FIXED_XI
            .iter()
            .enumerate()
            .map(|(slot, &role)| XiPlayer {
                // Synthetic identities: the parity harness has no `World`, so
                // any distinct ids suffice — nothing here reads them back.
                pid: PlayerId(slot as u32),
                role,
                attrs: notebook_gen_player(rng, role, club_q),
                entered_at_minute: 0.0,
                condition: 1.0,
                contact_injury_mult: 1.0,
                pending_ambient: Cell::new(None),
                injured_from_minute: Cell::new(None),
                foul_count: Cell::new(0),
                has_yellow: Cell::new(false),
                sent_off_from_minute: Cell::new(None),
            })
            .collect()
    }

    /// Tag namespace for this harness's derived streams — distinct from any
    /// real gameplay tag (`commands::FIXTURE_STREAM_NS`, `worldgen`'s), and
    /// unrelated to the seeds used elsewhere in the test suite.
    const PARITY_NS: u64 = 0x4E42_5052_0000_0000; // "NBPR"

    #[test]
    fn port_reproduces_notebook_gpm_on_notebook_equivalent_inputs() {
        const NUM_LEAGUES: u64 = 8;
        const NUM_CLUBS: usize = 20;

        // The notebook's own fitted b_beat, pinned independent of
        // `Knobs::default()`: the Rust-side calibration harness re-tuned
        // b_beat against real `worldgen`'s attribute distribution
        // (`knobs.rs`'s doc comment), so `Knobs::default()` no longer *is*
        // the notebook's snapshot. This test's whole point is checking the
        // loop against what the notebook actually reported, not against
        // whatever production is calibrated to today.
        let notebook_knobs = Knobs {
            b_beat: -1.7,
            // The notebook never modeled injuries or fouls; identity here
            // keeps this test checking only the parity question
            // (MATCH_MODEL.md §14 T10, §15 T11).
            injury_rate: 0.0,
            foul_rate: 0.0,
            ..Knobs::default()
        };

        let mut total_goals = 0u32;
        let mut total_matches = 0u32;

        for league in 0..NUM_LEAGUES {
            // Club quality anchors: linspace(48, 74), mirroring the
            // notebook's `run_batch` synthetic-league sweep.
            let qualities: Vec<f64> = (0..NUM_CLUBS)
                .map(|i| 48.0 + 26.0 * i as f64 / (NUM_CLUBS - 1) as f64)
                .collect();

            let mut gen_rng = derive_stream(league, PARITY_NS);
            let teams: Vec<Vec<XiPlayer>> = qualities
                .iter()
                .map(|&q| build_fixed_xi(&mut gen_rng, q))
                .collect();

            let club_ids: Vec<ClubId> = (0..NUM_CLUBS as u16).map(ClubId).collect();
            let fixtures = double_round_robin(&club_ids);

            for fixture in &fixtures {
                let home = &teams[fixture.home.0 as usize];
                let away = &teams[fixture.away.0 as usize];
                let mut match_rng = derive_stream(league, PARITY_NS | (fixture.id.0 as u64 + 1));
                let mut injury_rng = derive_stream(
                    league,
                    PARITY_NS | crate::match_engine::INJURY_NS | (fixture.id.0 as u64 + 1),
                );
                let mut foul_rng = derive_stream(
                    league,
                    PARITY_NS | crate::match_engine::FOUL_NS | (fixture.id.0 as u64 + 1),
                );
                let outcome = simulate(
                    home.clone(),
                    away.clone(),
                    Vec::new(),
                    Vec::new(),
                    Tactics::neutral(),
                    Tactics::neutral(),
                    &[],
                    &[],
                    &mut match_rng,
                    &mut injury_rng,
                    &mut foul_rng,
                    &notebook_knobs,
                );
                total_goals += outcome.home_goals as u32 + outcome.away_goals as u32;
                total_matches += 1;
            }
        }

        let gpm = total_goals as f64 / total_matches as f64;
        assert!(
            (2.3..=3.1).contains(&gpm),
            "pooled gpm {gpm} over {total_matches} notebook-equivalent-input matches falls \
             outside the ~2.5-2.9 band the notebook itself reads (~2.6-2.7 target/fitted). That \
             means the gap versus real-worldgen gpm (~1.7-2.0) is NOT purely an input-distribution \
             effect — diff this loop against the notebook cell-by-cell (kickoff alternation, the \
             minute += delta step count, the take_shot rebound loop, turnover mirroring, the \
             action-selection weights) before touching any knob or presence table."
        );
    }
}
