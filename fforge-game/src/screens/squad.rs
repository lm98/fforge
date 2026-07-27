//! The squad list: everyone on the books, grouped by natural role, strongest
//! first, with the best-role alternative flagged where it differs.
//!
//! **Colour axis: ability relative to the squad** (R15). Top quartile reads
//! `Good`, the middle two read `Ok`, the bottom quartile recedes to `Muted` —
//! "who is a first choice here, who is fringe", answered at a glance.
//!
//! Nothing about that is colour-only: the CA column carries the exact number
//! and the list is already ordered by it inside each role, so a `NO_COLOR` run
//! shows the same fact one step slower.

use crate::render::headline_ca;
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::Session;
use fforge_domain::ROLE_WEIGHTS;

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let world = &s.world;
    let mut players: Vec<_> = world.club_players(s.player_club).collect();
    players.sort_by_key(|p| (p.natural_role, std::cmp::Reverse(headline_ca(p))));

    let bands = Bands::of(players.iter().map(|p| headline_ca(p)));

    let mut t = Table::new(vec![
        Col::left("Pos", 3),
        Col::left("Name", 20),
        Col::right("Age", 3),
        Col::right("CA", 3),
        Col::left("Best role", 0),
    ]);
    for player in players {
        let (best, best_ca) = fforge_domain::best_role(&player.attributes, &ROLE_WEIGHTS);
        let alt = if best != player.natural_role {
            format!("{} ({})", best.short().trim(), best_ca)
        } else {
            String::new()
        };
        let ca = headline_ca(player);
        t.row_all(
            vec![
                Cell::new(player.natural_role.short()),
                Cell::new(player.name.clone()),
                Cell::new(player.age(s.date).to_string()),
                Cell::new(ca.to_string()),
                Cell::new(alt),
            ],
            bands.sem(ca),
        );
    }
    format!("\n{}", t.render(p))
}

/// The squad's own CA quartiles — "relative to the squad" means relative to
/// *this* squad, so a mid-table side's best player still reads as its best
/// player.
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
}
