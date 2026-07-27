//! The league table, in league order, with the human's club marked.

use fforge_core::{Session, league_table};
use std::fmt::Write as _;

pub fn render(session: &Session) -> String {
    let s = &session.state;
    let table = league_table(&s.world, &s.schedule, &s.results);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n     {:<22} {:>2} {:>3} {:>3} {:>3} {:>4} {:>4} {:>4} {:>4}",
        "Club", "", "W", "D", "L", "GF", "GA", "GD", "Pts"
    );
    for (i, row) in table.iter().enumerate() {
        let marker = if row.club == s.player_club { ">" } else { " " };
        let _ = writeln!(
            out,
            "{marker}{:>3}. {:<22} {:>2} {:>3} {:>3} {:>3} {:>4} {:>4} {:>+4} {:>4}",
            i + 1,
            s.world.club(row.club).name,
            row.played,
            row.won,
            row.drawn,
            row.lost,
            row.goals_for,
            row.goals_against,
            row.goal_diff(),
            row.points()
        );
    }
    out
}
