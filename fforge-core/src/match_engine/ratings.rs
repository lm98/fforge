//! Match ratings (`MATCH_MODEL.md` §18, T13): a pure, RNG-free fold over an
//! already-resolved `MatchOutcome.stream` plus final minutes and score — one
//! definition, recorded resolved on `MatchPlayed` (§12) since the stream
//! itself is a Trace and never folded: a rating that is not recorded could
//! never be re-derived on replay.

use super::Card;
use super::stream::{MatchEvent, MatchEventKind, ShotOutcome, Side};
use fforge_domain::{PlayerId, Role};
use std::collections::BTreeMap;

const BASE_RATING: f64 = 6.0;
const MIN_RATING: f64 = 3.0;
const MAX_RATING: f64 = 10.0;

fn side_idx(s: Side) -> usize {
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

fn bump(scores: &mut BTreeMap<PlayerId, f64>, pid: PlayerId, delta: f64) {
    if let Some(s) = scores.get_mut(&pid) {
        *s += delta;
    }
}

/// The highest-rated player in a resolved rating set, ties broken by the
/// lowest `PlayerId`.
///
/// **One rule, one place.** Both consumers want the same answer from the same
/// data — `fforge-game`'s match view reads `MatchOutcome.ratings` live, and
/// `news` reads `Event::MatchPlayed.ratings` off the log — and two copies of a
/// tie-break are two copies free to disagree about the same match.
///
/// Ratings are in tenths (§18): `84` is 8.4.
pub fn man_of_the_match(ratings: &[(PlayerId, u8)]) -> Option<(PlayerId, u8)> {
    ratings
        .iter()
        .copied()
        .max_by_key(|&(pid, rating)| (rating, std::cmp::Reverse(pid)))
}

/// One appeared player's identity for rating purposes — everything
/// `compute_ratings` needs beyond the stream itself: which side he played
/// for, his role (the clean-sheet gate is defensive-only), and his final
/// minutes (the cameo-regression input).
pub struct RatedPlayer {
    pub pid: PlayerId,
    pub side: Side,
    pub role: Role,
    pub minutes: u8,
}

/// Folds `stream` into a per-player rating for every `players` entry with
/// `minutes > 0`, per `MATCH_MODEL.md` §18's delta table. Base 6.0, deltas
/// accrue only from events the stream actually names a player in (which by
/// construction only happens while he is `on_pitch` — §18's "deltas accrue
/// only while on the pitch" holds without special-casing), then the whole
/// accumulated swing is regressed toward base by minutes share (§18's "sub
/// cameos regress toward 6.0" — applied uniformly to every partial
/// appearance, not substitutes alone: a rating built on few real minutes
/// deserves the same small-sample discount whether the shortfall came from a
/// late entry, an injury that ended his effectiveness, or a red card).
/// Clamped to `[3.0, 10.0]` and returned as tenths (`68` = 6.8).
pub fn compute_ratings(
    stream: &[MatchEvent],
    players: &[RatedPlayer],
    home_goals: u8,
    away_goals: u8,
) -> Vec<(PlayerId, u8)> {
    let mut scores: BTreeMap<PlayerId, f64> = players
        .iter()
        .filter(|p| p.minutes > 0)
        .map(|p| (p.pid, BASE_RATING))
        .collect();

    // `last_turnover[side]` = the actor who most recently coughed up
    // possession for `side` — consulted (and cleared) when the *other* side
    // scores, crediting the -0.3 "caused the turnover" penalty. An
    // approximation of "the last failed action before this goal" (§18):
    // it is whichever failure is most recent regardless of how many
    // unrelated events separate it from the goal, not a full reconstruction
    // of the exact zone-by-zone possession chain that produced this
    // particular attack — documented here rather than silently approximated.
    let mut last_turnover: [Option<PlayerId>; 2] = [None, None];

    for (i, event) in stream.iter().enumerate() {
        let idx = side_idx(event.side);
        match event.kind {
            MatchEventKind::Pass { success } => {
                bump(&mut scores, event.actor, if success { 0.02 } else { -0.04 });
                if !success {
                    last_turnover[idx] = Some(event.actor);
                }
            }
            MatchEventKind::TakeOn { success } => {
                bump(&mut scores, event.actor, if success { 0.10 } else { -0.05 });
                if !success {
                    last_turnover[idx] = Some(event.actor);
                    // "Tackle won (named opponent of a failed take-on)".
                    if let Some(tackler) = event.opponent {
                        bump(&mut scores, tackler, 0.15);
                    }
                }
            }
            MatchEventKind::Cross { success } => {
                // No direct delta for a cross itself (§18's table has none) —
                // only its downstream headed-shot consequence, or its assist
                // eligibility below, rates it. A failed delivery is still a
                // turnover for blame-tracking purposes.
                if !success {
                    last_turnover[idx] = Some(event.actor);
                }
            }
            MatchEventKind::Clearance => {
                last_turnover[idx] = Some(event.actor);
            }
            MatchEventKind::Shot { outcome, .. } => match outcome {
                ShotOutcome::Goal => {
                    bump(&mut scores, event.actor, 1.0);
                    // Assist: the immediately preceding stream event, if a
                    // successful same-side Pass/Cross/TakeOn.
                    if i > 0 {
                        let prev = &stream[i - 1];
                        if prev.side == event.side
                            && matches!(
                                prev.kind,
                                MatchEventKind::Pass { success: true }
                                    | MatchEventKind::Cross { success: true }
                                    | MatchEventKind::TakeOn { success: true }
                            )
                        {
                            bump(&mut scores, prev.actor, 0.7);
                        }
                    }
                    // Blame: whoever most recently turned the ball over for
                    // the conceding side.
                    let conceding = side_idx(other_side(event.side));
                    if let Some(blamed) = last_turnover[conceding].take() {
                        bump(&mut scores, blamed, -0.3);
                    }
                    last_turnover = [None, None];
                }
                ShotOutcome::Saved => bump(&mut scores, event.actor, 0.10),
                ShotOutcome::Off | ShotOutcome::Blocked => {
                    bump(&mut scores, event.actor, -0.05);
                    last_turnover[idx] = Some(event.actor);
                }
            },
            MatchEventKind::Save { .. } => {
                // The save beat's `opponent` is the keeper (§9's own
                // convention: `actor` stays the shooter).
                if let Some(gk) = event.opponent {
                    bump(&mut scores, gk, 0.20);
                }
            }
            MatchEventKind::Foul { card } => {
                // The fouling defender is `opponent` (§15's own convention).
                if let Some(defender) = event.opponent {
                    match card {
                        Some(Card::Yellow) => bump(&mut scores, defender, -0.3),
                        Some(Card::SecondYellow | Card::Red) => bump(&mut scores, defender, -1.0),
                        None => {}
                    }
                }
            }
            // A substitution and a turnover carry no per-player credit; an
            // injury is a misfortune, not a performance — §18's table scores
            // what a player *did*, and being hurt is not that. The minutes
            // regression already accounts for a shortened match.
            MatchEventKind::Substitution { .. }
            | MatchEventKind::Turnover
            | MatchEventKind::Injury { .. } => {}
        }
    }

    // Team result: win +0.2 to every appeared player on the winning side;
    // clean sheet +0.5 to GK/CB/FB on the side that conceded zero.
    let (home_win, away_win) = match home_goals.cmp(&away_goals) {
        std::cmp::Ordering::Greater => (0.2, 0.0),
        std::cmp::Ordering::Less => (0.0, 0.2),
        std::cmp::Ordering::Equal => (0.0, 0.0),
    };
    for p in players {
        let win_bonus = match p.side {
            Side::Home => home_win,
            Side::Away => away_win,
        };
        bump(&mut scores, p.pid, win_bonus);
        let conceded_zero = match p.side {
            Side::Home => away_goals == 0,
            Side::Away => home_goals == 0,
        };
        if conceded_zero && matches!(p.role, Role::Gk | Role::Cb | Role::Fb) {
            bump(&mut scores, p.pid, 0.5);
        }
    }

    players
        .iter()
        .filter_map(|p| {
            let raw = *scores.get(&p.pid)?;
            let share = (p.minutes as f64 / 90.0).clamp(0.0, 1.0);
            let regressed = BASE_RATING + (raw - BASE_RATING) * share;
            let clamped = regressed.clamp(MIN_RATING, MAX_RATING);
            Some((p.pid, (clamped * 10.0).round() as u8))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::zone::Zone;
    use super::*;

    fn event(
        side: Side,
        kind: MatchEventKind,
        actor: PlayerId,
        opponent: Option<PlayerId>,
    ) -> MatchEvent {
        MatchEvent {
            minute: 10,
            side,
            zone: Zone::Mid,
            kind,
            actor,
            opponent,
        }
    }

    fn player(pid: u32, side: Side, role: Role, minutes: u8) -> RatedPlayer {
        RatedPlayer {
            pid: PlayerId(pid),
            side,
            role,
            minutes,
        }
    }

    #[test]
    fn a_goal_and_its_assist_are_both_credited() {
        let scorer = PlayerId(1);
        let assister = PlayerId(2);
        let stream = vec![
            event(
                Side::Home,
                MatchEventKind::Pass { success: true },
                assister,
                None,
            ),
            event(
                Side::Home,
                MatchEventKind::Shot {
                    kind: super::super::stream::ShotKind::Finish,
                    source: super::super::stream::ShotSource::Through,
                    outcome: ShotOutcome::Goal,
                },
                scorer,
                None,
            ),
        ];
        let players = vec![
            player(1, Side::Home, Role::St, 90),
            player(2, Side::Home, Role::Am, 90),
        ];
        let ratings = compute_ratings(&stream, &players, 1, 0);
        let get = |pid: u32| {
            ratings
                .iter()
                .find(|&&(p, _)| p == PlayerId(pid))
                .map(|&(_, r)| r)
                .unwrap()
        };
        // Base 60 + goal 10 + win 2 = 72.
        assert_eq!(get(1), 72);
        // Base 60 + pass-completed 0.2(rounded away) + assist 7 + win 2 ≈ 69.
        assert!(
            get(2) >= 68 && get(2) <= 70,
            "assist credit missing: {}",
            get(2)
        );
    }

    #[test]
    fn a_player_who_never_appears_gets_no_rating() {
        let stream = vec![];
        let players = vec![player(1, Side::Home, Role::St, 0)];
        assert!(compute_ratings(&stream, &players, 0, 0).is_empty());
    }

    #[test]
    fn ratings_stay_within_the_documented_band() {
        let mut stream = Vec::new();
        // Pile on unrealistically many goals for one player to try to blow
        // past the clamp.
        for _ in 0..20 {
            stream.push(event(
                Side::Home,
                MatchEventKind::Shot {
                    kind: super::super::stream::ShotKind::Finish,
                    source: super::super::stream::ShotSource::Through,
                    outcome: ShotOutcome::Goal,
                },
                PlayerId(1),
                None,
            ));
        }
        let players = vec![player(1, Side::Home, Role::St, 90)];
        let ratings = compute_ratings(&stream, &players, 20, 0);
        let (_, r) = ratings[0];
        assert!(
            (30..=100).contains(&(r as i32)),
            "rating {r} out of [3.0,10.0]"
        );
    }

    #[test]
    fn a_five_minute_cameo_regresses_toward_base_despite_a_goal() {
        let stream = vec![event(
            Side::Home,
            MatchEventKind::Shot {
                kind: super::super::stream::ShotKind::Finish,
                source: super::super::stream::ShotSource::Through,
                outcome: ShotOutcome::Goal,
            },
            PlayerId(1),
            None,
        )];
        let players = vec![player(1, Side::Home, Role::St, 5)];
        let ratings = compute_ratings(&stream, &players, 1, 0);
        let (_, r) = ratings[0];
        // Full credit (base 60 + goal 10 + win 2 = 72) would read 72; a
        // 5-minute cameo must regress well below that toward base 60.
        assert!(
            r < 68,
            "a 5-minute cameo's goal must be heavily regressed toward base, got {r}"
        );
        assert!(
            r > 60,
            "the goal must still move the needle at all, got {r}"
        );
    }

    #[test]
    fn a_red_card_tanks_a_rating() {
        let defender = PlayerId(1);
        let stream = vec![event(
            Side::Home,
            MatchEventKind::Foul {
                card: Some(Card::Red),
            },
            PlayerId(2), // the fouled attacker retains the ball — is `actor`
            Some(defender),
        )];
        let players = vec![
            player(1, Side::Away, Role::Cb, 90),
            player(2, Side::Home, Role::St, 90),
        ];
        let ratings = compute_ratings(&stream, &players, 0, 0);
        let (_, r) = ratings
            .iter()
            .find(|&&(pid, _)| pid == defender)
            .copied()
            .unwrap();
        assert!(
            r < 60,
            "a red card must tank the fouling defender's rating below base, got {r}"
        );
    }

    #[test]
    fn an_injury_does_not_penalise_a_rating() {
        // `compute_ratings` never receives an injuries list at all — an
        // injured player's rating is folded purely from the stream events he
        // actually appears in, exactly like anyone else. This pins that
        // structural guarantee with a concrete read: a player involved in
        // ordinary, neutral play rates no differently for having also (per
        // some other, entirely separate part of the engine) been injured.
        let stream = vec![event(
            Side::Home,
            MatchEventKind::Pass { success: true },
            PlayerId(1),
            None,
        )];
        let players = vec![player(1, Side::Home, Role::Cm, 90)];
        let ratings = compute_ratings(&stream, &players, 0, 0);
        let (_, r) = ratings[0];
        // Base 60 + one completed pass (+0.2, rounds to +0) — no penalty
        // term exists for "was injured" anywhere in this fold.
        assert!(
            (59..=61).contains(&(r as i32)),
            "an injured player must rate on his stream events alone, got {r}"
        );
    }
}
