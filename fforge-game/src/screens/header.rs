//! The per-turn status header: who you are, when it is, where you sit, and
//! what is still outstanding for the next matchday.
//!
//! **Colour axis: outstanding decisions** (R15) — what still wants your
//! attention before you advance: an unset lineup, unread inbox items. A
//! submitted lineup reads `Ok` and costs no ink; anything left undone reads
//! `Warn`.
//!
//! The note text says the same thing in words, which is the non-colour
//! carrier — colour only makes it findable in one glance rather than one read.
//!
//! The title line's `Emphasis` is structural, not a value on the axis: it says
//! "this line is a heading", which is true regardless of state. R15's one-axis
//! rule is about *encoding data* in colour, and a heading encodes none.

use crate::render::sem::{Palette, Sem};
use crate::render::table_position;
use fforge_core::Session;
use std::fmt::Write as _;

pub fn render(session: &Session, unread: usize, p: Palette) -> String {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let pos = table_position(session, s.player_club);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            &format!(
                "=== {} · Matchday {}/{} · {} · position {} ===",
                club.name, s.current_matchday, s.last_matchday, s.date, pos
            ),
            Sem::Emphasis
        )
    );
    let (lineup_note, sem) = if s.pending_lineup.is_some() {
        ("lineup set for next matchday", Sem::Ok)
    } else if s.last_lineup.is_some() {
        ("no new lineup — last XI will be reused", Sem::Warn)
    } else {
        ("no lineup set — assistant will auto-pick", Sem::Warn)
    };
    let _ = writeln!(out, "{}", p.paint(&format!("    ({lineup_note})"), sem));
    if unread > 0 {
        let _ = writeln!(
            out,
            "{}",
            p.paint(
                &format!("    ({unread} unread inbox item(s) — [i])"),
                Sem::Warn
            )
        );
    }
    if !s.pending_transfer_decisions.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            p.paint(
                &format!(
                    "    ({} transfer decision(s) pending for the next window close)",
                    s.pending_transfer_decisions.len()
                ),
                Sem::Ok
            )
        );
    }
    out
}
