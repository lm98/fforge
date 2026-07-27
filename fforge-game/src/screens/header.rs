//! The per-turn status header: who you are, when it is, where you sit, and
//! what is still outstanding for the next matchday.

use crate::render::table_position;
use fforge_core::Session;
use std::fmt::Write as _;

pub fn render(session: &Session) -> String {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let pos = table_position(session, s.player_club);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n=== {} · Matchday {}/{} · {} · position {} ===",
        club.name, s.current_matchday, s.last_matchday, s.date, pos
    );
    let lineup_note = if s.pending_lineup.is_some() {
        "lineup set for next matchday"
    } else if s.last_lineup.is_some() {
        "no new lineup — last XI will be reused"
    } else {
        "no lineup set — assistant will auto-pick"
    };
    let _ = writeln!(out, "    ({lineup_note})");
    if !s.pending_transfer_decisions.is_empty() {
        let _ = writeln!(
            out,
            "    ({} transfer decision(s) pending for the next window close)",
            s.pending_transfer_decisions.len()
        );
    }
    out
}
