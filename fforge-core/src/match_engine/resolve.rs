//! The possession loop (`MATCH_MODEL.md` §3–5): sample an actor and a
//! primary defender by zone presence, resolve one contest with the shared
//! logistic shape, and transition zones — or, on reaching `Box`, resolve a
//! shot immediately. A direct port of the calibrated Python prototype's
//! `_step` / `_take_shot` / `select_action`.

use super::MatchOutcome;
use super::contest::{self, blend, contest_p, fatigue_mult};
use super::knobs::Knobs;
use super::stream::{MatchEvent, MatchEventKind, ShotKind, ShotOutcome, Side};
use super::zone::{self, Zone};
use crate::rng::Rng;
use fforge_domain::{Attribute, Attributes, Lineup, Role, World};

struct XiPlayer {
    role: Role,
    attrs: Attributes,
}

fn build_xi(world: &World, lineup: &Lineup) -> Vec<XiPlayer> {
    let def = lineup.formation_def();
    lineup
        .players
        .iter()
        .enumerate()
        .map(|(slot, &pid)| XiPlayer {
            role: def.slots[slot],
            attrs: world.player(pid).attributes.clone(),
        })
        .collect()
}

/// Per-contest team-quality means (the support term, `MATCH_MODEL.md` §4),
/// precomputed once per match per side — only for the contests that are
/// actually blended (the actor's attacking side of pass/take-on/cross/shot).
struct TeamMeans {
    pass_atk: f64,
    takeon_atk: f64,
    cross_atk: f64,
    finish_atk: f64,
    header_atk: f64,
}

fn team_means(xi: &[XiPlayer]) -> TeamMeans {
    let n = xi.len() as f64;
    let mean =
        |w: &[(Attribute, f64)]| xi.iter().map(|p| contest::score(&p.attrs, w)).sum::<f64>() / n;
    TeamMeans {
        pass_atk: mean(contest::PASS_ATK),
        takeon_atk: mean(contest::TAKEON_ATK),
        cross_atk: mean(contest::CROSS_ATK),
        finish_atk: mean(contest::FINISH_ATK),
        header_atk: mean(contest::HEADER_ATK),
    }
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

/// Turnover mirroring (`MATCH_MODEL.md` §3): possession flips and the winner
/// restarts in the mirrored zone — lose it deep and the opponent wins it
/// high; lose it high and they win it deep.
fn turnover(poss: Side, zone: Zone) -> (Side, Zone) {
    let next_zone = match zone {
        Zone::Def => Zone::AttC,
        Zone::Mid => Zone::Mid,
        Zone::AttC => Zone::Def,
        Zone::AttW => Zone::Def,
        Zone::Box => Zone::Def,
    };
    (other_side(poss), next_zone)
}

/// Sample a slot index from `xi` weighted by zone presence (`MATCH_MODEL.md`
/// §6). `presence` selects the attacking or defending table.
fn sample_by_presence(
    xi: &[XiPlayer],
    zone: Zone,
    presence: fn(Role, Zone) -> u32,
    rng: &mut Rng,
) -> usize {
    let weights: Vec<u32> = xi.iter().map(|p| presence(p.role, zone)).collect();
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
/// more) — where a future direct/patient tactic re-weights, no structural
/// change (`MATCH_MODEL.md` §3).
fn select_action(zone: Zone, actor: &XiPlayer, rng: &mut Rng, k: &Knobs) -> Action {
    match zone {
        Zone::Def => Action::Pass,
        Zone::Mid => weighted_choice(
            &[
                (Action::Pass, k.w_pass_mid),
                (
                    Action::TakeOn,
                    k.w_takeon_mid * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
                ),
            ],
            rng,
        ),
        Zone::AttC => weighted_choice(
            &[
                (Action::Pass, k.w_pass_attc),
                (
                    Action::TakeOn,
                    k.w_takeon_attc * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
                ),
                (
                    Action::LongShot,
                    k.w_longshot_attc * (actor.attrs.get(Attribute::Finishing) as f64 / 50.0),
                ),
            ],
            rng,
        ),
        Zone::AttW => weighted_choice(
            &[
                (
                    Action::Cross,
                    k.w_cross_attw * (actor.attrs.get(Attribute::Crossing) as f64 / 50.0),
                ),
                (
                    Action::TakeOn,
                    k.w_takeon_attw * (actor.attrs.get(Attribute::Dribbling) as f64 / 50.0),
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
    base_q: f64,
    att: &[XiPlayer],
    def_side: &[XiPlayer],
    tm_att: &TeamMeans,
    minute: f64,
    rng: &mut Rng,
    k: &Knobs,
    home_attacking: bool,
    goals: &mut [u32; 2],
    stream: &mut Vec<MatchEvent>,
) -> (Side, Zone) {
    let shooter = &att[sample_by_presence(att, Zone::Box, zone::attacking_presence, rng)];
    let defender =
        &def_side[sample_by_presence(def_side, Zone::Box, zone::defending_presence, rng)];
    let gk = &def_side[0]; // formation slot 0 is always Gk (formation.rs: "GK first")

    let mut kind = kind;
    let mut base_q = base_q;
    let minute_u8 = minute as u8;

    // Up to two rebound follow-ups, mirroring the notebook's bounded retry.
    for _ in 0..3 {
        let (atk, d_block, d_gk) = match kind {
            ShotKind::Header => (
                blend(
                    contest::score(&shooter.attrs, contest::HEADER_ATK),
                    tm_att.header_atk,
                    k,
                ) * fatigue_mult(&shooter.attrs, minute, k),
                contest::score(&defender.attrs, contest::AERIAL_DEF),
                contest::score(&gk.attrs, contest::GK_AERIAL),
            ),
            ShotKind::Finish | ShotKind::LongShot => (
                blend(
                    contest::score(&shooter.attrs, contest::FINISH_ATK),
                    tm_att.finish_atk,
                    k,
                ) * fatigue_mult(&shooter.attrs, minute, k),
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
                        outcome: ShotOutcome::Goal,
                    },
                });
                return (other_side(poss), Zone::Mid); // conceding side kicks off
            }
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone: Zone::Box,
                kind: MatchEventKind::Shot {
                    kind,
                    outcome: ShotOutcome::Saved,
                },
            });
            let rebound = rng.f64() < k.p_rebound;
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone: Zone::Box,
                kind: MatchEventKind::Save { parried: rebound },
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
            kind: MatchEventKind::Shot { kind, outcome },
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
    minute: f64,
    rng: &mut Rng,
    k: &Knobs,
    goals: &mut [u32; 2],
    stream: &mut Vec<MatchEvent>,
) -> (Side, Zone) {
    let (att, def_side) = match poss {
        Side::Home => (home, away),
        Side::Away => (away, home),
    };
    let tm_att = &tm[side_index(poss)];
    let home_attacking = poss == Side::Home;
    let minute_u8 = minute as u8;

    let actor = &att[sample_by_presence(att, zone, zone::attacking_presence, rng)];
    let defender = &def_side[sample_by_presence(def_side, zone, zone::defending_presence, rng)];
    let action = select_action(zone, actor, rng, k);

    match action {
        Action::Pass => {
            let atk = blend(
                contest::score(&actor.attrs, contest::PASS_ATK),
                tm_att.pass_atk,
                k,
            ) * fatigue_mult(&actor.attrs, minute, k);
            let dfe = contest::score(&defender.attrs, contest::PASS_DEF)
                * fatigue_mult(&defender.attrs, minute, k);
            let success = rng.f64() < contest_p(atk, dfe, k.b_pass, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::Pass { success },
            });
            if !success {
                return turnover(poss, zone);
            }
            match zone {
                Zone::Def => (
                    poss,
                    if rng.f64() < k.p_def_advance {
                        Zone::Mid
                    } else {
                        Zone::Def
                    },
                ),
                Zone::Mid => {
                    if rng.f64() < k.p_mid_advance {
                        (
                            poss,
                            if rng.f64() < k.p_wide {
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
                    if rng.f64() < k.p_attc_penetrate {
                        take_shot(
                            poss,
                            ShotKind::Finish,
                            k.q_through,
                            att,
                            def_side,
                            tm_att,
                            minute,
                            rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
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
            ) * fatigue_mult(&actor.attrs, minute, k);
            let dfe = contest::score(&defender.attrs, contest::TAKEON_DEF)
                * fatigue_mult(&defender.attrs, minute, k);
            let success = rng.f64() < contest_p(atk, dfe, k.b_takeon, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::TakeOn { success },
            });
            if !success {
                return turnover(poss, zone);
            }
            match zone {
                Zone::Mid => {
                    if rng.f64() < k.p_mid_advance {
                        (
                            poss,
                            if rng.f64() < k.p_wide {
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
                    if rng.f64() < k.p_attc_dribble_box {
                        take_shot(
                            poss,
                            ShotKind::Finish,
                            k.q_dribble,
                            att,
                            def_side,
                            tm_att,
                            minute,
                            rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
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
                            k.q_cutback,
                            att,
                            def_side,
                            tm_att,
                            minute,
                            rng,
                            k,
                            home_attacking,
                            goals,
                            stream,
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
            ) * fatigue_mult(&actor.attrs, minute, k);
            let dfe = contest::score(&defender.attrs, contest::CROSS_DEF)
                * fatigue_mult(&defender.attrs, minute, k);
            let success = rng.f64() < contest_p(atk, dfe, k.b_cross_delivery, k, home_attacking);
            stream.push(MatchEvent {
                minute: minute_u8,
                side: poss,
                zone,
                kind: MatchEventKind::Cross { success },
            });
            if success {
                take_shot(
                    poss,
                    ShotKind::Header,
                    k.q_header,
                    att,
                    def_side,
                    tm_att,
                    minute,
                    rng,
                    k,
                    home_attacking,
                    goals,
                    stream,
                )
            } else {
                stream.push(MatchEvent {
                    minute: minute_u8,
                    side: poss,
                    zone,
                    kind: MatchEventKind::Clearance,
                });
                turnover(poss, zone)
            }
        }
        Action::LongShot => take_shot(
            poss,
            ShotKind::LongShot,
            k.q_long,
            att,
            def_side,
            tm_att,
            minute,
            rng,
            k,
            home_attacking,
            goals,
            stream,
        ),
    }
}

pub fn play_match(
    world: &World,
    home_lineup: &Lineup,
    away_lineup: &Lineup,
    rng: &mut Rng,
) -> MatchOutcome {
    let k = Knobs::default();
    let home = build_xi(world, home_lineup);
    let away = build_xi(world, away_lineup);
    let tm = [team_means(&home), team_means(&away)];

    let mut goals = [0u32, 0u32];
    let mut stream = Vec::new();

    for half in 0..2u8 {
        let start = 45.0 * half as f64;
        let end = 45.0 * (half as f64 + 1.0);
        let mut poss = if half == 0 { Side::Home } else { Side::Away }; // each half kicked off by the appropriate side
        let mut zone = Zone::Mid;
        let mut minute = start;
        while minute < end {
            let (next_poss, next_zone) = step(
                poss,
                zone,
                &home,
                &away,
                &tm,
                minute,
                rng,
                &k,
                &mut goals,
                &mut stream,
            );
            poss = next_poss;
            zone = next_zone;
            minute += k.delta;
        }
    }

    MatchOutcome {
        home_goals: goals[0].min(u8::MAX as u32) as u8,
        away_goals: goals[1].min(u8::MAX as u32) as u8,
        stream,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
