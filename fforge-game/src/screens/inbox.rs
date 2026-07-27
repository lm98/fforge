//! The inbox: `fforge-core::news`'s Trace, surfaced.
//!
//! **Colour axis: salience** (R15) — must-see / worth reading / background.
//! Salience is an importance axis with no good/bad direction, so it maps to
//! `Emphasis` / `Ok` / `Muted` and never to the diverging pair. No red here at
//! all: an inbox entry reports something that already happened, and R15 keeps
//! red for a live alarm.
//!
//! Nothing is colour-only: the list is *ordered* by salience, must-see items
//! carry a `!` marker, and unread ones carry `*`.
//!
//! **Nothing is pre-rendered anywhere but the renderer.** Every line comes from
//! `TemplateRenderer`, reading the structured `NewsKind` — this screen never
//! formats a `NewsKind` itself. That is the Batch 2 R2 decision holding: the
//! Phase-5 journalist renderer is a *peer* implementation of `NewsRenderer`
//! authoring from the same structure, not a patch over someone else's strings.

use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::Session;
use fforge_core::news::{Audience, NewsItem, NewsObserver, NewsRenderer, TemplateRenderer};
use std::fmt::Write as _;

/// How many notable items one screenful shows.
const MAX_SHOWN: usize = 20;

/// How many *background* items ride along underneath. This is the number that
/// matters: a 20-club league produces 10 results a matchday, all of them
/// `Audience::League`, so an inbox that merely sorts by salience is still 80%
/// other clubs' scorelines. Capping the background band separately is what
/// turns the screen from a log dump into an inbox.
const BACKGROUND_SHOWN: usize = 6;

/// At or above this, an item is must-see. Matches `news`'s own scale: the
/// player's own club's events sit at 60–80, everyone else's at 15–25.
const MUST_SEE: u8 = 60;

/// Below this, an item is background — league noise you can skim past.
const BACKGROUND: u8 = 40;

/// `unread` is how many of the newest inbox entries the player has not opened
/// yet — tracked by the caller, because "read" is a fact about *this player's
/// session*, never a fact of the recorded game (nothing about it belongs in
/// the log, exactly like the news items themselves).
pub fn render(session: &Session, news: &NewsObserver, unread: usize, p: Palette) -> String {
    let s = &session.state;
    let audience = Audience::Club(s.player_club);
    // Newest first, unfiltered by salience — the screen orders by salience
    // itself, and dropping the league's background entirely would leave the
    // inbox showing only your own club, which is not an inbox.
    let newest_first = news.inbox(audience, 0);
    // The newest `unread` entries are exactly the ones not yet seen: `inbox`
    // only ever grows at the front.
    let unread_refs: Vec<&NewsItem> = newest_first.iter().take(unread).copied().collect();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            &format!(
                "=== Inbox — {} · {} ({} unread of {}) ===",
                s.world.club(s.player_club).name,
                s.date,
                unread,
                newest_first.len()
            ),
            Sem::Emphasis
        )
    );

    if newest_first.is_empty() {
        let _ = writeln!(out, "{}", p.paint("  Nothing yet.", Sem::Muted));
        return out;
    }

    let mut ordered = newest_first.clone();
    // Salience first, then recency — R15's table assigns this screen the
    // salience axis, and an inbox sorted by importance is the whole reason to
    // have salience at all.
    ordered.sort_by_key(|i| (std::cmp::Reverse(i.salience), std::cmp::Reverse(i.date)));
    let (notable, background): (Vec<&NewsItem>, Vec<&NewsItem>) = ordered
        .iter()
        .copied()
        .partition(|i| i.salience >= BACKGROUND);
    let shown: Vec<&NewsItem> = notable
        .iter()
        .take(MAX_SHOWN)
        .chain(background.iter().take(BACKGROUND_SHOWN))
        .copied()
        .collect();
    let hidden = ordered.len() - shown.len();

    let renderer = TemplateRenderer;
    let mut t = Table::new(vec![
        Col::left("", 2),
        Col::left("Date", 14),
        Col::left("", 0),
    ]);
    for item in &shown {
        let is_unread = unread_refs.iter().any(|u| std::ptr::eq(*u, *item));
        let marker = match (is_unread, item.salience >= MUST_SEE) {
            (true, true) => "*!",
            (true, false) => "* ",
            (false, true) => " !",
            (false, false) => "",
        };
        t.row_all(
            vec![
                Cell::new(marker),
                Cell::new(item.date.to_string()),
                // The one and only place a `NewsKind` becomes a string.
                Cell::new(renderer.render(item, &s.world)),
            ],
            salience_sem(item.salience),
        );
    }
    out.push_str(&t.render(p));

    if hidden > 0 {
        let _ = writeln!(
            out,
            "{}",
            p.paint(
                &format!("  ...and {hidden} lower-salience item(s) not shown."),
                Sem::Muted
            )
        );
    }
    let _ = writeln!(out, "{}", p.paint("  * unread   ! must-see", Sem::Muted));
    out
}

fn salience_sem(salience: u8) -> Sem {
    if salience >= MUST_SEE {
        Sem::Emphasis
    } else if salience >= BACKGROUND {
        Sem::Ok
    } else {
        Sem::Muted
    }
}

/// How many entries the player's inbox currently holds — the caller's read
/// cursor is a count against this, so it lives next to the filter that
/// produces it rather than being re-derived at the call site.
pub fn len(session: &Session, news: &NewsObserver) -> usize {
    news.inbox(Audience::Club(session.state.player_club), 0)
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salience_bands_run_background_to_must_see() {
        // The scale `news` actually emits: 15/25 for other clubs, 55-80 for
        // the player's own.
        assert_eq!(salience_sem(15), Sem::Muted);
        assert_eq!(salience_sem(25), Sem::Muted);
        assert_eq!(salience_sem(55), Sem::Ok);
        assert_eq!(salience_sem(70), Sem::Emphasis);
        assert_eq!(salience_sem(80), Sem::Emphasis);
    }
}
