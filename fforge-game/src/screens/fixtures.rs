//! This matchday's fixtures, plus the previous matchday's results once there
//! is a previous matchday to show.
//!
//! **Colour axis: `Mine`** (R15). Twenty fixtures a matchday, exactly one of
//! which you are managing — that is the only distinction worth an encoding
//! here, and it is the same axis the table uses for the same reason.
//!
//! The `<— your match` tag and the results' `>` marker carry it without colour.

use crate::render::result_line;
use crate::render::sem::{Palette, Sem};
use fforge_core::Session;
use std::fmt::Write as _;

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let mut out = String::new();
    let _ = writeln!(out, "\nMatchday {} fixtures:", s.current_matchday);
    for f in s.fixtures_of_matchday(s.current_matchday) {
        let mine = f.home == s.player_club || f.away == s.player_club;
        // Laid out whole, then painted whole: no padding happens after the
        // paint, so the escapes cannot shear the columns.
        let line = format!(
            "  {:<22} vs {:<22}{}",
            s.world.club(f.home).name,
            s.world.club(f.away).name,
            if mine { " <— your match" } else { "" }
        );
        let _ = writeln!(
            out,
            "{}",
            p.paint(line.trim_end(), if mine { Sem::Mine } else { Sem::Ok })
        );
    }
    if s.current_matchday > 1 {
        let prev = s.current_matchday - 1;
        let _ = writeln!(out, "\nMatchday {prev} results:");
        for f in s.fixtures_of_matchday(prev) {
            if let Some(&(hg, ag)) = s.results.get(&f.id) {
                let _ = writeln!(
                    out,
                    "{}",
                    result_line(&s.world, s.player_club, f.home, f.away, hg, ag, p)
                );
            }
        }
    }
    out
}
