//! The per-turn status panel: who you are, when it is, where you sit, who is
//! next — and what is still outstanding before you advance.
//!
//! **Colour axis: outstanding decisions** (R15) — what still wants your
//! attention before you advance: an unset lineup, unread inbox items. A
//! submitted lineup reads `Ok` and costs no ink; anything left undone reads
//! `Warn`, and every one of those lines is also prefixed `!`, which is the
//! non-colour carrier.
//!
//! The panel's *standing* readings — position, points, form, next opponent —
//! deliberately carry **no** colour at all. Form is the case worth naming: a
//! W/D/L strip is exactly the sort of thing a screen wants to tint green and
//! red, and doing so here would put a second axis on one panel, which is the
//! ambiguity R15 exists to prevent (`UI_TOOLKIT_EVIDENCE.md` §4b saw the same
//! pressure on the squad screen twice). The letters carry themselves.
//!
//! The frame and the club name are `Emphasis`: structural, not a value on the
//! axis. R15's one-axis rule is about *encoding data* in colour, and a heading
//! encodes none.

use crate::render::sem::{Palette, Sem};
use crate::render::{date_long, form_strip, next_fixture, ordinal, table_row};
use fforge_core::Session;
use std::fmt::Write as _;

/// Inner width of the panel, in characters. Chosen to sit inside an 80-column
/// terminal with room for the two frame characters and a little air.
const W: usize = 68;

/// How many matches the form strip looks back over.
const FORM_WINDOW: usize = 5;

pub fn render(session: &Session, unread: usize, p: Palette) -> String {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let mut out = String::new();

    // --- the title rule: club on the left, date on the right ---------------
    let left = format!("─ {} ", club.name.to_uppercase());
    let right = format!(" {} ─", date_long(s.date));
    let fill = W
        .saturating_sub(left.chars().count() + right.chars().count())
        .max(1);
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            &format!("┌{left}{}{right}┐", "─".repeat(fill)),
            Sem::Emphasis
        )
    );

    // --- standing readings -------------------------------------------------
    let standing = match table_row(session, s.player_club) {
        Some(row) => format!(
            "{} · Matchday {} of {} · {} on {} pts",
            s.world.competition.name,
            s.current_matchday,
            s.last_matchday,
            ordinal(crate::render::table_position(session, s.player_club)),
            row.points(),
        ),
        None => format!(
            "{} · Matchday {} of {}",
            s.world.competition.name, s.current_matchday, s.last_matchday
        ),
    };
    let _ = writeln!(out, "{}", body(&standing, p));

    let next = match next_fixture(session, s.player_club) {
        Some((opponent, home)) => format!(
            "Next  {} {}",
            if home { "vs" } else { "away to" },
            s.world.club(opponent).name
        ),
        None => "Next  —  season complete".to_string(),
    };
    // Both readings on one line, the form strip right-aligned against the
    // frame: two facts, one glance, and the panel stays four lines tall.
    let form = format!("Form  {}", form_strip(session, s.player_club, FORM_WINDOW));
    let gap = W
        .saturating_sub(2 + next.chars().count() + form.chars().count() + 1)
        .max(2);
    let _ = writeln!(
        out,
        "{}",
        body(&format!("{next}{}{form}", " ".repeat(gap)), p)
    );

    // --- what is still outstanding (the colour axis) -----------------------
    for (text, sem) in outstanding(session, unread) {
        let _ = writeln!(out, "{}", body_sem(&text, sem, p));
    }

    let _ = writeln!(
        out,
        "{}",
        p.paint(&format!("└{}┘", "─".repeat(W)), Sem::Emphasis)
    );
    out
}

/// Every line the panel's colour axis speaks on, in the order a manager wants
/// to be nagged: the team sheet first, then the inbox, then the standing
/// commitments that need no action.
fn outstanding(session: &Session, unread: usize) -> Vec<(String, Sem)> {
    let s = &session.state;
    let mut lines = Vec::new();
    lines.push(if s.pending_lineup.is_some() {
        ("  team sheet submitted".to_string(), Sem::Ok)
    } else if s.last_lineup.is_some() {
        (
            "! no new team sheet — last XI will be reused".to_string(),
            Sem::Warn,
        )
    } else {
        (
            "! no team sheet — the assistant will pick".to_string(),
            Sem::Warn,
        )
    });
    if unread > 0 {
        let plural = if unread == 1 { "" } else { "s" };
        lines.push((
            format!("! {unread} unread inbox item{plural} — [i]"),
            Sem::Warn,
        ));
    }
    if !s.pending_transfer_decisions.is_empty() {
        let n = s.pending_transfer_decisions.len();
        let plural = if n == 1 { "" } else { "s" };
        lines.push((
            format!("  {n} transfer decision{plural} lodged for the window close"),
            Sem::Ok,
        ));
    }
    lines
}

/// One framed body line, uncoloured content. **Pad before you paint**: the
/// frame characters are laid out against the finished, padded text, because an
/// escape sequence has zero visual width and several bytes of it
/// (`render::table`'s rule, applied by hand here — this is a panel, not a
/// table).
fn body(text: &str, p: Palette) -> String {
    body_sem(text, Sem::Ok, p)
}

fn body_sem(text: &str, sem: Sem, p: Palette) -> String {
    let pad = W.saturating_sub(text.chars().count() + 2);
    format!(
        "{} {}{} {}",
        p.paint("│", Sem::Emphasis),
        p.paint(text, sem),
        " ".repeat(pad),
        p.paint("│", Sem::Emphasis)
    )
}
