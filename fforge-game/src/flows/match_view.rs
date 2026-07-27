//! The Phase 2 "humble text match view" (`DESIGN.md` §9): the raw event
//! stream, unfiltered, paced line-by-line with a skip-to-full-time keypress
//! when there is a real terminal on both ends.
//!
//! The line *building* is pure (`commentary_lines`); only the pacing and the
//! raw-mode toggling live in the printing half. That split is the same one
//! `MatchEvent::commentary` already makes one level down.

use fforge_core::match_engine;
use fforge_domain::World;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// Pacing for the humble text match view's line-by-line playback.
const EVENT_DELAY: Duration = Duration::from_millis(120);

/// The stream's commentary, name-resolved. The `World` lookup lives here —
/// this crate owns the `World` and is the only one allowed to touch stdout, so
/// `commentary` itself stays name-resolved and I/O-free (`MATCH_MODEL.md` §9).
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
            event.commentary(side_name, actor)
        })
        .collect()
}

/// Prints the humble text match view. Shared by the standalone friendly viewer
/// and, for the human's own fixture, the main game loop's matchday advance.
pub fn print_humble_text_view(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
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
