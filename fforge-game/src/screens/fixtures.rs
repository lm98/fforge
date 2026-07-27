//! This matchday's fixtures, plus the previous matchday's results once there
//! is a previous matchday to show.

use crate::render::result_line;
use fforge_core::Session;
use std::fmt::Write as _;

pub fn render(session: &Session) -> String {
    let s = &session.state;
    let mut out = String::new();
    let _ = writeln!(out, "\nMatchday {} fixtures:", s.current_matchday);
    for f in s.fixtures_of_matchday(s.current_matchday) {
        let star = if f.home == s.player_club || f.away == s.player_club {
            " <— your match"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {:<22} vs {:<22}{}",
            s.world.club(f.home).name,
            s.world.club(f.away).name,
            star
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
                    result_line(&s.world, s.player_club, f.home, f.away, hg, ag)
                );
            }
        }
    }
    out
}
