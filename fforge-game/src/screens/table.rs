//! The league table, in league order, with the human's club marked.
//!
//! **Colour axis: `Mine`, and nothing else** (R15). A league table's meaning is
//! already positional — first is first — so colouring it by form or by points
//! would be decoration competing with the ordering. The one thing the ordering
//! cannot say is which row is yours.
//!
//! The `>` marker in the leading column carries the same fact without colour.

use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::{Session, league_table};

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let table = league_table(&s.world, &s.schedule, &s.results);
    let mut t = Table::new(vec![
        Col::left("", 1),
        Col::right("", 3),
        Col::left("Club", 22),
        Col::right("", 2),
        Col::right("W", 3),
        Col::right("D", 3),
        Col::right("L", 3),
        Col::right("GF", 4),
        Col::right("GA", 4),
        Col::right("GD", 4),
        Col::right("Pts", 4),
    ])
    .indent("");
    for (i, row) in table.iter().enumerate() {
        let mine = row.club == s.player_club;
        let cells = vec![
            Cell::new(if mine { ">" } else { " " }),
            Cell::new(format!("{}.", i + 1)),
            Cell::new(s.world.club(row.club).name.clone()),
            Cell::new(row.played.to_string()),
            Cell::new(row.won.to_string()),
            Cell::new(row.drawn.to_string()),
            Cell::new(row.lost.to_string()),
            Cell::new(row.goals_for.to_string()),
            Cell::new(row.goals_against.to_string()),
            Cell::new(format!("{:+}", row.goal_diff())),
            Cell::new(row.points().to_string()),
        ];
        if mine {
            t.row_all(cells, Sem::Mine);
        } else {
            t.row(cells);
        }
    }
    format!("\n{}", t.render(p))
}
