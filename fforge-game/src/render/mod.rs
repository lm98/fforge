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

use fforge_core::{Session, league_table};
use fforge_domain::{ClubId, Player, ROLE_WEIGHTS, World, current_ability};
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
    use super::ordinal;

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
}
