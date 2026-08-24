//! The squad list: everyone on the books, grouped by natural role, strongest
//! first — with what each one earns, when his deal runs out, and what he is
//! worth. Followed by the depth summary the market's hard stabilizers are
//! judged against.
//!
//! **Colour axis: ability relative to the squad** (R15). Top quartile reads
//! `Good`, the middle two `Ok`, the bottom quartile recedes to `Muted` — "who
//! is a first choice here, who is fringe", answered at a glance. Relative to
//! *this* squad, so a mid-table side's best player still reads as its best.
//!
//! **The depth block uses `Bad` and nothing else, and that is not a second
//! axis.** The two colour sets are disjoint by construction: the player list
//! never emits red, the depth block never emits anything else. A red on this
//! screen therefore has exactly one meaning — a `club_ai` hard stabilizer is
//! breached (`TRANSFER_MODEL.md` §11: `≥ 2` GK, squad size inside
//! `[squad_min, squad_max]`) — which is precisely the "role uncovered" alarm
//! R15 reserves red for. R15's failure mode is one visual encoding meaning two
//! things; disjoint sets cannot produce it.
//!
//! Nothing here is colour-only. Ability is in the `CA` column and in the
//! ordering; contract urgency is in the `Contract` column and its `!`/`!!`
//! glyph; a stabilizer breach is in the depth block's `need` column and its
//! `!` glyph; recent form is in `Rtg`/`Form` and carries no ink at all, since
//! it is a *second* reading of quality and two hues for two flavours of "how
//! good is he" is exactly the ambiguity the one-axis rule forbids.

use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use crate::render::{headline_ca, money};
use fforge_core::{
    DevKnobs, MarketContext, SQUAD_TEMPLATE, Session, UtilityKnobs, ValueKnobs, value_all,
};
use fforge_domain::{GameDate, Money, ROLE_WEIGHTS, Role, date::DAYS_PER_YEAR};
use std::fmt::Write as _;

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let world = &s.world;
    let mut players: Vec<_> = world.club_players(s.player_club).collect();
    players.sort_by_key(|p| (p.natural_role, std::cmp::Reverse(headline_ca(p))));

    // The same omniscient §2.6 valuation every club prices against — see
    // `VALUATION_NOTE`.
    let vk = ValueKnobs::default();
    let dev = DevKnobs::default();
    let ctx = MarketContext::from_world(world, &vk, &s.recent_ratings);
    let valuations = value_all(world, s.date, &ctx, &vk, &dev);

    let bands = Bands::of(players.iter().map(|p| headline_ca(p)));

    let mut t = Table::new(vec![
        Col::left("Pos", 3),
        Col::left("Name", 20),
        Col::right("Age", 3),
        Col::right("CA", 3),
        Col::left("Potential", POTENTIAL_COL_WIDTH),
        Col::right("Wage", 7),
        Col::right("Contract", 10),
        Col::right("Value*", 7),
        Col::right("Rtg", 4),
        Col::right("Form", 4),
        Col::left("Best role", 0),
    ]);
    for player in &players {
        let (best, best_ca) = fforge_domain::best_role(&player.attributes, &ROLE_WEIGHTS);
        let alt = if best != player.natural_role {
            format!("{} ({})", best.short().trim(), best_ca)
        } else {
            String::new()
        };
        let ca = headline_ca(player);
        let (wage, contract) = match &player.contract {
            Some(c) => (money(c.wage), contract_cell(c.expires, s.date)),
            // A player on the books with no contract is a free agent the club
            // has not signed — rare, but the fold permits it, so say so rather
            // than printing a blank.
            None => ("—".to_string(), "unsigned !!".to_string()),
        };
        t.row_all(
            vec![
                Cell::new(player.natural_role.short()),
                Cell::new(player.name.clone()),
                Cell::new(player.age(s.date).to_string()),
                Cell::new(ca.to_string()),
                Cell::new(potential_label(player.character.potential)),
                Cell::new(wage),
                Cell::new(contract),
                Cell::new(money(
                    valuations.get(&player.id).copied().unwrap_or(Money(0)),
                )),
                Cell::new(latest_rating(s.recent_ratings.get(&player.id))),
                Cell::new(form(s.recent_ratings.get(&player.id))),
                Cell::new(alt),
            ],
            bands.sem(ca),
        );
    }

    let mut out = format!("\n{}", t.render(p));
    let _ = writeln!(out, "{}", p.paint(VALUATION_NOTE, Sem::Muted));
    out.push_str(&depth_block(session, p));
    out
}

/// Valuations here are the **omniscient ground truth** (`TRANSFER_MODEL.md`
/// §2.6): every club in v1 prices off the same central `value()`, and there is
/// no scouting fog-of-war until Phase 5. Labelled rather than left implicit, so
/// the column does not quietly become a promise the fogged game has to break.
const VALUATION_NOTE: &str =
    " * Value is the market's ground truth — no scouting error yet (Phase 5 adds it).";

/// Wide enough for the longest label (`"can become special"`, 19 chars) —
/// `render::table::pad` never truncates an over-long cell, so a column too
/// narrow for its widest label doesn't clip, it silently shears every column
/// after it instead. `potential_label_fits_its_column` pins this.
const POTENTIAL_COL_WIDTH: usize = 19;

/// A plain-language read on a player's PA (`ATTRIBUTE_SCHEMA.md` §4: the
/// hidden ceiling on best-role CA development) — the same omniscient
/// ground-truth channel `VALUATION_NOTE` already opens for `Value`, and no
/// more scouting fog-of-war than that column has until Phase 5 adds it.
/// Thresholds are non-overlapping by construction: `[80, 85)` promising,
/// `[85, 90)` great potential, `[90, 100]` can become special.
fn potential_label(pa: u8) -> &'static str {
    if pa >= 90 {
        "can become special"
    } else if pa >= 85 {
        "great potential"
    } else if pa >= 80 {
        "promising"
    } else {
        ""
    }
}

/// The most recent match rating, or `—` for a player who has not played.
/// Ratings are recorded in tenths (`MATCH_MODEL.md` §18): `74` is 7.4.
fn latest_rating(recent: Option<&Vec<u8>>) -> String {
    match recent.and_then(|r| r.last()) {
        Some(&rating) => format!("{:.1}", rating as f64 / 10.0),
        None => "—".to_string(),
    }
}

/// Mean of the rolling form window `GameState::recent_ratings` already keeps
/// (`RATING_FORM_WINDOW`-capped) — read, never re-derived, so this and the
/// `form_mult` the transfer market prices with are the same number
/// (`TRANSFER_MODEL.md` §2.5).
///
/// Uncoloured on purpose: this screen's one axis is ability (see the module
/// docs), and form is a *second* reading of quality. Two hues for two flavours
/// of "how good is he" is precisely the ambiguity R15 forbids, so form gets a
/// column and no ink.
fn form(recent: Option<&Vec<u8>>) -> String {
    match recent.filter(|r| !r.is_empty()) {
        Some(r) => {
            let mean = r.iter().map(|&x| x as f64).sum::<f64>() / r.len() as f64;
            format!("{:.1}", mean / 10.0)
        }
        None => "—".to_string(),
    }
}

/// Years left on a deal, with an urgency glyph. The glyph — not a colour — is
/// what carries urgency on this screen, because the row's colour is already
/// spoken for by the ability axis. A monospace column of `!!` is if anything a
/// louder prompt than a hue, and it survives `NO_COLOR` intact.
fn contract_cell(expires: GameDate, today: GameDate) -> String {
    let years = (expires.days - today.days) as f64 / DAYS_PER_YEAR as f64;
    if years <= 0.0 {
        "expired !!".to_string()
    } else if years < 0.5 {
        // Inside the last half-season: `TRANSFER_MODEL.md` §2.4's contract
        // multiplier is biting hard, so he is visibly losing value too.
        format!("{years:.1}y !!")
    } else if years < 1.0 {
        format!("{years:.1}y !")
    } else {
        format!("{years:.1}y")
    }
}

/// Depth against `worldgen::SQUAD_TEMPLATE`, plus the two hard stabilizers
/// `club_ai` enforces on every AI club and a human can otherwise breach in
/// silence (`TRANSFER_MODEL.md` §11).
fn depth_block(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let knobs = UtilityKnobs::default();
    let squad: Vec<_> = s.world.club_players(s.player_club).collect();

    let mut out = String::new();
    let _ = writeln!(out, "\n Squad depth by natural role:");
    let mut t = Table::new(vec![
        Col::left("Role", 5),
        Col::right("Have", 4),
        Col::right("Template", 8),
        Col::left("", 0),
    ]);
    for &(role, template) in SQUAD_TEMPLATE.iter() {
        let have = squad.iter().filter(|pl| pl.natural_role == role).count();
        let floor = hard_floor(role, &knobs);
        let breached = floor.is_some_and(|f| have < f);
        let note = match (floor.filter(|_| breached), have.cmp(&template)) {
            (Some(f), _) => format!("! below the hard minimum of {f}"),
            (None, std::cmp::Ordering::Less) => format!("{} short of template", template - have),
            (None, std::cmp::Ordering::Greater) => format!("{} over template", have - template),
            (None, std::cmp::Ordering::Equal) => String::new(),
        };
        let cells = vec![
            Cell::new(role.short()),
            Cell::new(have.to_string()),
            Cell::new(template.to_string()),
            Cell::new(note),
        ];
        if breached {
            t.row_all(cells, Sem::Bad);
        } else {
            t.row(cells);
        }
    }
    out.push_str(&t.render(p));

    let size = squad.len();
    let bounds = format!(
        " Squad size {size} (the market's hard bounds are {}–{})",
        knobs.squad_min, knobs.squad_max
    );
    let out_of_bounds = size < knobs.squad_min || size > knobs.squad_max;
    let _ = writeln!(
        out,
        "{}",
        if out_of_bounds {
            p.paint(&format!("{bounds} !"), Sem::Bad)
        } else {
            bounds
        }
    );
    out
}

/// The hard per-role minimum, where one exists — today only goalkeepers
/// (`club_ai`'s `min_goalkeepers`). Read from `UtilityKnobs` rather than
/// re-stated, so the CLI cannot drift from what the market enforces.
fn hard_floor(role: Role, knobs: &UtilityKnobs) -> Option<usize> {
    match role {
        Role::Gk => Some(knobs.min_goalkeepers),
        _ => None,
    }
}

/// The squad's own CA quartiles — "relative to the squad" means relative to
/// *this* squad, so a mid-table side's best player still reads as its best.
struct Bands {
    q1: u8,
    q3: u8,
}

impl Bands {
    fn of(cas: impl Iterator<Item = u8>) -> Bands {
        let mut sorted: Vec<u8> = cas.collect();
        sorted.sort_unstable();
        if sorted.is_empty() {
            return Bands { q1: 0, q3: u8::MAX };
        }
        let at = |frac: f64| sorted[((sorted.len() - 1) as f64 * frac).round() as usize];
        Bands {
            q1: at(0.25),
            q3: at(0.75),
        }
    }

    fn sem(&self, ca: u8) -> Sem {
        if ca >= self.q3 {
            Sem::Good
        } else if ca <= self.q1 {
            Sem::Muted
        } else {
            Sem::Ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_split_a_squad_into_thirds_by_quartile() {
        let bands = Bands::of([50, 55, 60, 65, 70, 75, 80, 85, 90].into_iter());
        assert_eq!(bands.sem(90), Sem::Good);
        assert_eq!(bands.sem(70), Sem::Ok);
        assert_eq!(bands.sem(50), Sem::Muted);
    }

    /// A squad where everyone is identical has no "relative to the squad"
    /// signal at all; whatever it does, it must not panic or claim a spread.
    #[test]
    fn a_uniform_squad_does_not_panic() {
        let bands = Bands::of([70, 70, 70].into_iter());
        assert_eq!(bands.sem(70), Sem::Good);
    }

    #[test]
    fn an_empty_squad_does_not_panic() {
        let bands = Bands::of(std::iter::empty());
        assert_eq!(bands.sem(70), Sem::Ok);
    }

    #[test]
    fn form_reads_the_window_and_the_latest_reads_the_last() {
        let window = vec![70u8, 64, 82];
        assert_eq!(latest_rating(Some(&window)), "8.2");
        // (7.0 + 6.4 + 8.2) / 3 = 7.2
        assert_eq!(form(Some(&window)), "7.2");
    }

    #[test]
    fn a_player_who_has_not_played_shows_no_rating() {
        assert_eq!(latest_rating(None), "—");
        assert_eq!(form(None), "—");
        assert_eq!(form(Some(&Vec::new())), "—");
    }

    #[test]
    fn potential_label_fits_its_column() {
        for pa in 0..=u8::MAX {
            assert!(
                potential_label(pa).chars().count() <= POTENTIAL_COL_WIDTH,
                "potential_label({pa}) exceeds POTENTIAL_COL_WIDTH"
            );
        }
    }

    #[test]
    fn potential_bands_are_non_overlapping() {
        assert_eq!(potential_label(79), "");
        assert_eq!(potential_label(80), "promising");
        assert_eq!(potential_label(84), "promising");
        assert_eq!(potential_label(85), "great potential");
        assert_eq!(potential_label(89), "great potential");
        assert_eq!(potential_label(90), "can become special");
        assert_eq!(potential_label(100), "can become special");
    }

    #[test]
    fn contract_urgency_escalates_and_is_glyph_carried() {
        let today = GameDate::from_year_day(2026, 200);
        let years = |y: f64| today.add_days((y * DAYS_PER_YEAR as f64) as i64);
        assert_eq!(contract_cell(years(3.0), today), "3.0y");
        assert_eq!(contract_cell(years(0.8), today), "0.8y !");
        assert_eq!(contract_cell(years(0.2), today), "0.2y !!");
        assert_eq!(contract_cell(today, today), "expired !!");
    }
}
