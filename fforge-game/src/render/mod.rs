//! Presentation helpers shared by every screen and flow.
//!
//! - [`sem`] — the `Sem` semantic vocabulary and the *only* mapping from it to
//!   a colour. Nothing outside that module names a colour.
//! - [`table`] — column layout. Pads before it paints, which is the only
//!   ordering that survives colour.
//! - **Formatting** — turning already-resolved values into the exact strings
//!   the screens emit (`ordinal`, `result_line`).
//! - **Small derived readings** the screens quote (`headline_ca`,
//!   `club_avg_ca`, `table_position`). These are pure functions of state; they
//!   sit here rather than in `screens/` because flows quote them too.
//!
//! Nothing in here prints.

pub mod sem;
pub mod table;

use fforge_core::{Session, TableRow, league_table};
use fforge_domain::{ClubId, GameDate, Money, Player, ROLE_WEIGHTS, World, current_ability};
use sem::{Palette, Sem};

/// A player's headline CA: his ability *in his own natural role*, which is the
/// number every squad list quotes.
pub fn headline_ca(p: &Player) -> u8 {
    current_ability(&p.attributes, p.natural_role, &ROLE_WEIGHTS)
}

/// Mean headline CA across a club's whole squad — the new-game club picker's
/// one-number strength summary.
pub fn club_avg_ca(world: &World, club: ClubId) -> f64 {
    let players: Vec<_> = world.club_players(club).collect();
    let sum: u32 = players
        .iter()
        .map(|p| current_ability(&p.attributes, p.natural_role, &ROLE_WEIGHTS) as u32)
        .sum();
    sum as f64 / players.len() as f64
}

/// 1-based league position, or 0 if the club somehow isn't in the table.
pub fn table_position(session: &Session, club: ClubId) -> usize {
    let s = &session.state;
    league_table(&s.world, &s.schedule, &s.results)
        .iter()
        .position(|r| r.club == club)
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// A club's own row in the league table, if it has one.
pub fn table_row(session: &Session, club: ClubId) -> Option<TableRow> {
    let s = &session.state;
    league_table(&s.world, &s.schedule, &s.results)
        .into_iter()
        .find(|r| r.club == club)
}

/// One completed match from a club's point of view.
pub struct FormResult {
    /// `'W'`, `'D'` or `'L'` — the whole reading, and it carries itself
    /// without colour.
    pub letter: char,
    pub opponent: ClubId,
    pub home: bool,
    pub scored: u8,
    pub conceded: u8,
}

/// A club's completed matches this season, oldest first.
pub fn results_so_far(session: &Session, club: ClubId) -> Vec<FormResult> {
    let s = &session.state;
    let mut out: Vec<(u8, FormResult)> = Vec::new();
    for f in &s.schedule {
        let Some(&(hg, ag)) = s.results.get(&f.id) else {
            continue;
        };
        let home = f.home == club;
        if !home && f.away != club {
            continue;
        }
        let (scored, conceded) = if home { (hg, ag) } else { (ag, hg) };
        let letter = match scored.cmp(&conceded) {
            std::cmp::Ordering::Greater => 'W',
            std::cmp::Ordering::Equal => 'D',
            std::cmp::Ordering::Less => 'L',
        };
        out.push((
            f.matchday,
            FormResult {
                letter,
                opponent: if home { f.away } else { f.home },
                home,
                scored,
                conceded,
            },
        ));
    }
    out.sort_by_key(|(md, _)| *md);
    out.into_iter().map(|(_, r)| r).collect()
}

/// The last `n` results as the `W D L` strip every football screen carries,
/// oldest on the left. Fewer than `n` played reads as dashes in the empty
/// slots, so the strip keeps its width from matchday 1 and the header below it
/// never shifts.
pub fn form_strip(session: &Session, club: ClubId, n: usize) -> String {
    let all = results_so_far(session, club);
    let recent = &all[all.len().saturating_sub(n)..];
    let mut cells: Vec<char> = vec!['-'; n.saturating_sub(recent.len())];
    cells.extend(recent.iter().map(|r| r.letter));
    cells
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The club's next scheduled opponent: who, and whether it is at home.
pub fn next_fixture(session: &Session, club: ClubId) -> Option<(ClubId, bool)> {
    let s = &session.state;
    s.fixtures_of_matchday(s.current_matchday)
        .find(|f| f.home == club || f.away == club)
        .map(|f| {
            if f.home == club {
                (f.away, true)
            } else {
                (f.home, false)
            }
        })
}

/// One result line. Your own club's results are marked with `>` **and**
/// coloured `Mine`; the marker is the non-colour carrier, so a piped run loses
/// nothing (R15).
///
/// Returned without a trailing newline; the caller decides.
pub fn result_line(
    world: &World,
    mine: ClubId,
    home: ClubId,
    away: ClubId,
    hg: u8,
    ag: u8,
    p: Palette,
) -> String {
    let is_mine = home == mine || away == mine;
    let marker = if is_mine { ">" } else { " " };
    // Laid out whole, then painted whole — no padding happens after the paint,
    // which is the rule `render::table` exists to enforce elsewhere.
    let line = format!(
        "{marker} {:<22} {:>2} - {:<2} {}",
        world.club(home).name,
        hg,
        ag,
        world.club(away).name
    );
    let line = line.trim_end();
    p.paint(line, if is_mine { Sem::Mine } else { Sem::Ok })
}

/// Money for humans: `1_500_000` → `1.5M`, `250_000` → `250k`, `900` → `900`.
///
/// Wage bills and transfer fees run to seven and eight digits, and a column of
/// raw integers is unreadable at a glance — you end up counting digits to tell
/// 1_200_000 from 12_000_000. Precision is never the point on these screens;
/// magnitude is.
pub fn money(m: Money) -> String {
    let sign = if m.0 < 0 { "-" } else { "" };
    let v = m.0.unsigned_abs();
    if v >= 1_000_000 {
        let millions = v as f64 / 1_000_000.0;
        // 1.5M, but 15M rather than 15.0M — a decimal only earns its place
        // while it is still a significant digit.
        if millions < 10.0 {
            format!("{sign}{millions:.1}M")
        } else {
            format!("{sign}{}M", millions.round() as u64)
        }
    } else if v >= 10_000 {
        format!("{sign}{}k", v / 1_000)
    } else {
        format!("{sign}{v}")
    }
}

/// The sim calendar's month lengths. `date::DAYS_PER_YEAR` is a flat 365, so
/// the twelve civil month lengths of a non-leap year tile it *exactly* — the
/// mapping is total and lossless, not an approximation.
const MONTHS: [(&str, u16); 12] = [
    ("Jan", 31),
    ("Feb", 28),
    ("Mar", 31),
    ("Apr", 30),
    ("May", 31),
    ("Jun", 30),
    ("Jul", 31),
    ("Aug", 31),
    ("Sep", 30),
    ("Oct", 31),
    ("Nov", 30),
    ("Dec", 31),
];

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// `GameDate` → `(day-of-month, month name)`.
fn day_and_month(d: GameDate) -> (u16, &'static str) {
    let mut doy = d.day_of_year();
    for (name, len) in MONTHS {
        if doy < len {
            return (doy + 1, name);
        }
        doy -= len;
    }
    // Unreachable: the twelve lengths sum to DAYS_PER_YEAR and `day_of_year`
    // is `rem_euclid` of it. Degrading to the last day of the year rather than
    // panicking is the right failure for a presentation function.
    (31, "Dec")
}

/// A date a football manager would recognise: `9 Aug 2026`.
///
/// `GameDate`'s own `Display` is `2026, day 220`, which is the right shape for
/// a log line and the wrong one for a game screen — a manager thinks in
/// August, not in day 220. The mapping lives *here*, in layer 5, and not on
/// the domain type, because it is a presentation choice: the sim itself has no
/// months and does not want any.
pub fn date(d: GameDate) -> String {
    let (day, month) = day_and_month(d);
    format!("{day} {month} {}", d.year())
}

/// The same date with its weekday: `Sat 9 Aug 2026`.
///
/// Worth the three extra characters on the status header specifically, because
/// the schedule spaces matchdays exactly seven days apart — so every matchday
/// in a save falls on the same weekday, and seeing `Sat` there is the cheapest
/// possible cue that the thing on screen is a football season.
pub fn date_long(d: GameDate) -> String {
    format!("{} {}", WEEKDAYS[d.days.rem_euclid(7) as usize], date(d))
}

/// `1` → `1st`, `2` → `2nd`, ... including the 11th/12th/13th exceptions.
pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::{MONTHS, date, date_long, money, ordinal};
    use fforge_domain::{GameDate, Money, date::DAYS_PER_YEAR};

    #[test]
    fn money_reads_as_magnitude_not_digits() {
        assert_eq!(money(Money(0)), "0");
        assert_eq!(money(Money(900)), "900");
        assert_eq!(money(Money(9_999)), "9999");
        assert_eq!(money(Money(250_000)), "250k");
        assert_eq!(money(Money(1_500_000)), "1.5M");
        assert_eq!(money(Money(12_000_000)), "12M");
        assert_eq!(money(Money(-2_500_000)), "-2.5M");
    }

    #[test]
    fn ordinal_handles_the_teens() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(4), "4th");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(12), "12th");
        assert_eq!(ordinal(13), "13th");
        assert_eq!(ordinal(21), "21st");
    }

    #[test]
    fn the_months_tile_the_sim_year_exactly() {
        let total: u16 = MONTHS.iter().map(|(_, len)| len).sum();
        assert_eq!(total as i64, DAYS_PER_YEAR);
    }

    #[test]
    fn every_day_of_the_year_lands_in_a_real_month() {
        // Total, not approximate: the last day of the year must be 31 Dec and
        // the first 1 Jan, with no gap or overlap anywhere between.
        for doy in 0..DAYS_PER_YEAR as u16 {
            let rendered = date(GameDate::from_year_day(2026, doy));
            assert!(rendered.ends_with(" 2026"), "{rendered}");
        }
        assert_eq!(date(GameDate::from_year_day(2026, 0)), "1 Jan 2026");
        assert_eq!(date(GameDate::from_year_day(2026, 31)), "1 Feb 2026");
        assert_eq!(date(GameDate::from_year_day(2026, 220)), "9 Aug 2026");
        assert_eq!(date(GameDate::from_year_day(2026, 364)), "31 Dec 2026");
    }

    #[test]
    fn a_season_of_matchdays_all_falls_on_the_same_weekday() {
        // The schedule spaces matchdays seven days apart, so this is what a
        // player actually sees: Saturday football, week after week. If the
        // weekday ever drifts down a season, this is the test that says so.
        let opening = GameDate::from_year_day(2026, 220);
        assert_eq!(date_long(opening), "Sat 9 Aug 2026");
        for md in 0..38i64 {
            let d = opening.add_days(md * 7);
            assert!(date_long(d).starts_with("Sat "), "{}", date_long(d));
        }
    }
}
