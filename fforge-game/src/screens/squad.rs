//! The squad list: everyone on the books, grouped by natural role, strongest
//! first, with the best-role alternative flagged where it differs.

use crate::render::headline_ca;
use fforge_core::Session;
use fforge_domain::ROLE_WEIGHTS;
use std::fmt::Write as _;

pub fn render(session: &Session) -> String {
    let s = &session.state;
    let world = &s.world;
    let mut players: Vec<_> = world.club_players(s.player_club).collect();
    players.sort_by_key(|p| (p.natural_role, std::cmp::Reverse(headline_ca(p))));

    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n {:<3} {:<20} {:>3} {:>3}  Best role",
        "Pos", "Name", "Age", "CA"
    );
    for p in players {
        let (best, best_ca) = fforge_domain::best_role(&p.attributes, &ROLE_WEIGHTS);
        let alt = if best != p.natural_role {
            format!("{} ({})", best.short().trim(), best_ca)
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            " {:<3} {:<20} {:>3} {:>3}  {}",
            p.natural_role.short(),
            p.name,
            p.age(s.date),
            headline_ca(p),
            alt
        );
    }
    out
}
