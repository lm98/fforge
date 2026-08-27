//! This matchday's fixtures, plus the previous matchday's results once there
//! is a previous matchday to show.
//!
//! **Colour axis: `Mine`** (R15). Twenty fixtures a matchday, exactly one of
//! which you are managing — that is the only distinction worth an encoding
//! here, and it is the same axis the table uses for the same reason.
//!
//! The `<— your match` tag and the results' `>` marker carry it without colour.

use crate::render::sem::{Palette, Sem};
use crate::render::{result_line, results_so_far};
use fforge_core::Session;
use std::fmt::Write as _;

/// How far back the personal record block looks. Six is a season's worth of
/// context without turning the screen into a scrolling ledger by matchday 30.
const RECORD_WINDOW: usize = 6;

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
    out.push_str(&record_block(session, p));
    out
}

/// Your own last few results, newest first — the run of form the table's
/// single position number flattens away. Every row is one of yours, so this
/// block is uniformly `Mine`: the axis does not change, it just stops
/// distinguishing.
fn record_block(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let all = results_so_far(session, s.player_club);
    if all.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\nYour last {} — newest first:",
        RECORD_WINDOW.min(all.len())
    );
    for r in all.iter().rev().take(RECORD_WINDOW) {
        let line = format!(
            "  {}  {}  {:<22} {}-{}",
            r.letter,
            if r.home { "(H)" } else { "(A)" },
            s.world.club(r.opponent).name,
            r.scored,
            r.conceded
        );
        let _ = writeln!(out, "{}", p.paint(line.trim_end(), Sem::Mine));
    }
    out
}
