//! The Phase 2 "humble text match view" (`DESIGN.md` §9): the raw event
//! stream, unfiltered, paced line by line with a skip-to-full-time keypress
//! when there is a real terminal on both ends — followed by the resolved
//! consequences the stream cannot show.
//!
//! The line *building* is pure (`commentary_lines`, `aftermath`); only the
//! pacing and the raw-mode toggling live in the printing half. That split is
//! the same one `MatchEvent::commentary` already makes one level down.
//!
//! **Colour axis on the aftermath: consequence severity.** A card and an
//! injury cost you a player; a man of the match does not. Injuries and reds
//! read `Bad`, yellows `Warn`, the man of the match `Emphasis`. Every one of
//! them is labelled in words too, so a piped run loses nothing.

use crate::render::sem::{Palette, Sem};
use fforge_core::match_engine;
use fforge_core::{Card, CardOutcome, InjuryOutcome};
use fforge_domain::{PlayerId, World};
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// Pacing for the humble text match view's line-by-line playback.
const EVENT_DELAY: Duration = Duration::from_millis(120);

/// The stream's commentary, name-resolved. The `World` lookup lives here —
/// this crate owns the `World` and is the only one allowed to touch stdout, so
/// `commentary` itself stays name-resolved and I/O-free (`MATCH_MODEL.md` §9).
///
/// Two names per beat, not one: `MatchEvent::other_player` names the fouling
/// defender, the departing substitute, or the contesting opponent, and a card
/// with nobody's name on it is no use to a manager.
pub fn commentary_lines(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
) -> Vec<String> {
    outcome
        .stream
        .iter()
        .map(|event| {
            let side_name = match event.side {
                match_engine::Side::Home => home_name,
                match_engine::Side::Away => away_name,
            };
            let actor = world.player(event.actor).name.as_str();
            let other = event.other_player().map(|p| world.player(p).name.as_str());
            event.commentary(side_name, actor, other)
        })
        .collect()
}

/// What the match left behind: cards, injuries, and the man of the match.
///
/// The stream tells you these happened as they happened; a manager still wants
/// them collected in one place at full time, because that is the list he has
/// to pick a team around next week.
pub fn aftermath(world: &World, outcome: &match_engine::MatchOutcome, p: Palette) -> String {
    let mut out = String::new();
    if !outcome.cards.is_empty() {
        let _ = writeln!(out, "\nCards:");
        // Chronological — the order they were shown.
        let mut cards: Vec<&CardOutcome> = outcome.cards.iter().collect();
        cards.sort_by_key(|c| (c.minute, c.player));
        for c in cards {
            let (word, sem) = match c.card {
                Card::Yellow => ("booked", Sem::Warn),
                Card::SecondYellow => ("sent off (second yellow)", Sem::Bad),
                Card::Red => ("sent off (straight red)", Sem::Bad),
            };
            let _ = writeln!(
                out,
                "{}",
                p.paint(
                    &format!("  {}' {} — {word}.", c.minute, world.player(c.player).name),
                    sem
                )
            );
        }
    }
    if !outcome.injuries.is_empty() {
        let _ = writeln!(out, "\nInjuries:");
        let mut injuries: Vec<&InjuryOutcome> = outcome.injuries.iter().collect();
        injuries.sort_by_key(|i| i.player);
        for i in injuries {
            // A zero-day knock is not an absence, so it does not read as one —
            // and it does not read as an alarm either.
            let (text, sem) = if i.days_out == 0 {
                (
                    format!(
                        "  {} — a knock, no games missed.",
                        world.player(i.player).name
                    ),
                    Sem::Warn,
                )
            } else {
                (
                    format!(
                        "  {} — out for {} day(s).",
                        world.player(i.player).name,
                        i.days_out
                    ),
                    Sem::Bad,
                )
            };
            let _ = writeln!(out, "{}", p.paint(&text, sem));
        }
    }
    if let Some((pid, rating)) = man_of_the_match(outcome) {
        let _ = writeln!(
            out,
            "\n{}",
            p.paint(
                &format!(
                    "Man of the match: {} ({:.1}).",
                    world.player(pid).name,
                    rating as f64 / 10.0
                ),
                Sem::Emphasis
            )
        );
    }
    out
}

/// The highest-rated player on the pitch, ties broken by `PlayerId` so the
/// answer is deterministic — the same rule a replay would reach.
///
/// Ratings are in tenths (`MATCH_MODEL.md` §18): `68` is 6.8.
pub fn man_of_the_match(outcome: &match_engine::MatchOutcome) -> Option<(PlayerId, u8)> {
    outcome
        .ratings
        .iter()
        .copied()
        .max_by_key(|&(pid, rating)| (rating, std::cmp::Reverse(pid)))
}

/// Prints the humble text match view. Shared by the standalone friendly viewer
/// and, for the human's own fixture, the main game loop's matchday advance.
pub fn print_humble_text_view(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
    p: Palette,
) {
    println!(
        "\n{home_name} vs {away_name} — {} raw events, unfiltered (the humble text match view, DESIGN.md §9):",
        outcome.stream.len()
    );
    // Only worth pacing/skippable when there's an actual terminal on both
    // ends — piped output (tests, redirects) just gets the whole stream at
    // once, same as before this feature existed.
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    if tty {
        println!("(press any key to skip to full time)");
    }
    println!();

    let interactive = tty && crossterm::terminal::enable_raw_mode().is_ok();
    let mut skipping = false;
    for line in commentary_lines(world, home_name, away_name, outcome) {
        if interactive {
            // Raw mode turns off the terminal's own \n -> \r\n translation.
            print!("{line}\r\n");
            io::stdout().flush().ok();
        } else {
            println!("{line}");
        }
        if interactive && !skipping && key_pressed_within(EVENT_DELAY) {
            skipping = true;
        }
    }
    if interactive {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    println!(
        "\nFULL TIME: {home_name} {} - {} {away_name}",
        outcome.home_goals, outcome.away_goals
    );
    print!("{}", aftermath(world, outcome, p));
}

/// Blocks up to `delay`, watching for a keypress. Returns as soon as one
/// arrives (true) so the caller can stop pacing the rest of the stream;
/// returns false once `delay` elapses with nothing pressed.
fn key_pressed_within(delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        match crossterm::event::poll(remaining) {
            Ok(true) => {
                if matches!(
                    crossterm::event::read(),
                    Ok(crossterm::event::Event::Key(_))
                ) {
                    return true;
                }
                // Some other event (resize, focus change, ...) — keep waiting.
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::man_of_the_match;
    use fforge_core::match_engine::MatchOutcome;
    use fforge_domain::PlayerId;

    fn outcome_with(ratings: Vec<(PlayerId, u8)>) -> MatchOutcome {
        MatchOutcome {
            home_goals: 0,
            away_goals: 0,
            stream: Vec::new(),
            injuries: Vec::new(),
            cards: Vec::new(),
            ratings,
            minutes: Vec::new(),
        }
    }

    #[test]
    fn the_man_of_the_match_is_the_highest_rating() {
        let o = outcome_with(vec![
            (PlayerId(3), 68),
            (PlayerId(7), 84),
            (PlayerId(1), 71),
        ]);
        assert_eq!(man_of_the_match(&o), Some((PlayerId(7), 84)));
    }

    /// Ties break on the lowest `PlayerId`, so the answer never depends on the
    /// order the engine happened to emit ratings in.
    #[test]
    fn ties_break_deterministically() {
        let a = outcome_with(vec![(PlayerId(9), 80), (PlayerId(2), 80)]);
        let b = outcome_with(vec![(PlayerId(2), 80), (PlayerId(9), 80)]);
        assert_eq!(man_of_the_match(&a), Some((PlayerId(2), 80)));
        assert_eq!(man_of_the_match(&a), man_of_the_match(&b));
    }

    #[test]
    fn no_ratings_means_no_man_of_the_match() {
        assert_eq!(man_of_the_match(&outcome_with(Vec::new())), None);
    }
}
