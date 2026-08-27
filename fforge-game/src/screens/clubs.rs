//! The new-game club picker.
//!
//! The first real decision of a save, and until now it was a name and one
//! number. A manager choosing a club is asking "how hard is this going to be,
//! and what am I given to do it with" — squad strength, standing, and money —
//! so those are the columns.
//!
//! **Colour axis: squad strength relative to this league** (R15) — the same
//! axis, and the same `Good`/`Ok`/`Muted` vocabulary, `screens::squad` uses for
//! player ability, because it is the same question one level up. The
//! `Expectation` column says it in words, so a `NO_COLOR` run loses nothing.
//!
//! The list is ordered strongest first, which makes the *ordering* a third
//! carrier of the same reading — and the pick index follows the order shown,
//! so `[1]` is always the row printed first.

use crate::render::sem::{Palette, Sem};
use crate::render::table::{Align, Cell, Col, Table};
use crate::render::{club_avg_ca, money};
use fforge_domain::{ClubId, World};
use std::fmt::Write as _;

/// The clubs, strongest squad first — the order the picker's numbers follow.
///
/// Sorting by mean CA rather than by reputation is deliberate: reputation is
/// the club's *past*, and what a new manager inherits is the squad. Ties break
/// on `ClubId` so the order is stable for a given seed, which the snapshot
/// tests depend on.
pub fn ordered(world: &World) -> Vec<ClubId> {
    let mut clubs = world.competition.clubs.clone();
    clubs.sort_by(|a, b| {
        club_avg_ca(world, *b)
            .partial_cmp(&club_avg_ca(world, *a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    clubs
}

/// Where a club sits in the league's pecking order, by rank rather than by an
/// absolute CA threshold — a division of weak clubs still has a favourite.
fn expectation(rank: usize, of: usize) -> (&'static str, Sem) {
    let quartile = (rank * 4) / of.max(1);
    match quartile {
        0 => ("Title contender", Sem::Good),
        1 => ("Europe, on a good year", Sem::Ok),
        2 => ("Mid-table", Sem::Ok),
        _ => ("Relegation fight", Sem::Muted),
    }
}

pub fn render(world: &World, ordered: &[ClubId], p: Palette) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            &format!("=== {} — pick your club ===", world.competition.name),
            Sem::Emphasis
        )
    );
    let mut t = Table::new(vec![
        Col::right("", 4),
        Col::left("Club", 22),
        Col::right("Squad", 5),
        Col::right("Rep", 4),
        Col::right("Balance", 8),
        Col::right("Wages", 8),
        Col {
            label: "Expectation".to_string(),
            width: 22,
            align: Align::Left,
        },
    ]);
    for (i, &cid) in ordered.iter().enumerate() {
        let club = world.club(cid);
        let (label, sem) = expectation(i, ordered.len());
        t.row_all(
            vec![
                Cell::new(format!("[{}]", i + 1)),
                Cell::new(club.name.clone()),
                Cell::new(format!("{:.0}", club_avg_ca(world, cid))),
                Cell::new(club.reputation.to_string()),
                Cell::new(money(club.finances.balance)),
                Cell::new(money(club.finances.wage_budget)),
                Cell::new(label),
            ],
            sem,
        );
    }
    out.push_str(&t.render(p));
    let _ = writeln!(
        out,
        "{}",
        p.paint(
            "  Squad = mean ability · Rep = club standing, 0–100 · Wages = annual wage-bill ceiling",
            Sem::Muted
        )
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quartiles_cover_a_twenty_club_league_without_a_gap() {
        let labels: Vec<&str> = (0..20).map(|r| expectation(r, 20).0).collect();
        assert_eq!(labels[0], "Title contender");
        assert_eq!(labels[19], "Relegation fight");
        // Monotone: the label may only ever get worse as the rank does.
        let rank_of = |l: &str| match l {
            "Title contender" => 0,
            "Europe, on a good year" => 1,
            "Mid-table" => 2,
            _ => 3,
        };
        for pair in labels.windows(2) {
            assert!(rank_of(pair[0]) <= rank_of(pair[1]));
        }
    }

    #[test]
    fn an_empty_or_single_club_league_does_not_divide_by_zero() {
        // `of` comes from a `Vec::len()`, so zero is reachable in principle
        // and must not panic. A lone club is its own favourite either way.
        assert_eq!(expectation(0, 1).0, "Title contender");
        assert_eq!(expectation(0, 0).0, "Title contender");
    }
}
