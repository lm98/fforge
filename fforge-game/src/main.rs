//! fforge-game — layer 5: the CLI presentation.
//!
//! This binary is the only place allowed to touch stdin/stdout and the wall
//! clock (a default seed when the player doesn't supply one — the seed is
//! then *recorded* in `GameStarted`, so the core never sees a clock).
//!
//! The split is by role, not by feature (R17):
//!
//! | module | holds |
//! |---|---|
//! | `main` | entry, `game_loop`, the menu, the observer bundle |
//! | `screens` | read-only renders — each a pure function returning `String` |
//! | `flows` | multi-step interactions (lineup, transfers, advance, friendly) |
//! | `render` | the `Sem` colour vocabulary, table layout, shared formatting |
//! | `input` | the only functions that touch stdin |

mod flows;
mod input;
mod render;
mod screens;

use flows::advance::advance_flow;
use flows::friendly::watch_friendly_flow;
use flows::lineup::set_lineup_flow;
use flows::new_game::{load_flow, new_game_flow};
use flows::save::do_save;
use flows::season::season_end_flow;
use flows::transfers::transfer_flow;
use input::{prompt_choice, prompt_menu};
use render::sem::{Palette, Sem};
use std::fmt::Write as _;

use fforge_core::{EventObserver, SeasonTelemetry, Session, news::NewsObserver};

pub const SAVE_PATH: &str = "savegame.fml";

/// The passive event-stream consumers the game loop keeps alongside its
/// `Session`, bundled so every `execute` call site notifies *all* of them —
/// an observer that misses events silently produces a wrong inbox rather than
/// a compile error, so there is exactly one place that list is written down.
pub struct Observers {
    pub telemetry: SeasonTelemetry,
    pub news: NewsObserver,
}

impl Default for Observers {
    fn default() -> Observers {
        Observers {
            telemetry: SeasonTelemetry::default(),
            news: NewsObserver::new(),
        }
    }
}

impl Observers {
    /// Borrow both as `EventObserver`s. Pass as `&mut o.all()` to
    /// `Session::{from_events, execute}`.
    pub fn all(&mut self) -> [&mut dyn EventObserver; 2] {
        [&mut self.telemetry, &mut self.news]
    }
}

fn main() {
    // Resolved once, here at the edge, and passed down as data — nothing
    // deeper in the tree reads the environment or the terminal (R15).
    let args: Vec<String> = std::env::args().collect();
    let palette = Palette::from_environment(&args);

    print!("{}", screens::title::banner(palette));
    loop {
        let save_present = std::path::Path::new(SAVE_PATH).exists();
        print!("{}", screens::title::menu(save_present, SAVE_PATH, palette));
        match prompt_choice("  > ", &["1", "2", "q"]).as_str() {
            "1" => {
                if let Some((session, observers)) = new_game_flow(palette) {
                    game_loop(session, observers, palette);
                }
            }
            "2" => match load_flow() {
                Some((session, observers)) => game_loop(session, observers, palette),
                None => println!("No save found at ./{SAVE_PATH} (or it failed to load)."),
            },
            _ => {
                println!("Goodbye.");
                return;
            }
        }
    }
}

fn game_loop(mut session: Session, mut o: Observers, p: Palette) {
    // How many inbox entries the player has already opened. A CLI-local read
    // cursor, deliberately not recorded: "read" is a fact about this session,
    // never a fact of the game — the same reasoning that keeps the news items
    // themselves out of the log.
    let mut inbox_seen = 0usize;
    loop {
        // State-condition news (`news::check_conditions`) is a query over
        // state, not events, so it has to be pumped explicitly. Once per loop
        // iteration is once per command: every path below executes at most one
        // command before coming back here.
        o.news.check_conditions(&session.state);
        let unread = screens::inbox::len(&session, &o.news).saturating_sub(inbox_seen);

        if session.state.season_over() {
            if season_end_flow(&mut session, &mut o, p) {
                // Rolled over: the fresh schedule is folded in, so fall
                // straight back into the normal loop on matchday 1.
                continue;
            }
            return;
        }
        print!("{}", screens::header::render(&session, unread, p));
        print!("{}", menu(unread, p));
        match prompt_menu(
            "> ",
            &[
                "", "s", "l", "f", "t", "x", "$", "m", "i", "r", "k", "w", "q",
            ],
        )
        .as_str()
        {
            // `Advance` is the only entry a player hits *every* turn, so it is
            // the default action rather than one of ten equals (R14).
            "" => advance_flow(&mut session, &mut o, p),
            "s" => print!("{}", screens::squad::render(&session, p)),
            "l" => set_lineup_flow(&mut session, &mut o, p),
            "f" => print!("{}", screens::availability::render(&session, p)),
            "t" => print!("{}", screens::table::render(&session, p)),
            "x" => print!("{}", screens::fixtures::render(&session, p)),
            "$" => print!("{}", screens::finances::render(&session, p)),
            "m" => transfer_flow(&mut session, &mut o, p),
            "i" => {
                print!("{}", screens::inbox::render(&session, &o.news, unread, p));
                inbox_seen = screens::inbox::len(&session, &o.news);
            }
            "r" => print!("{}", screens::stats::render(&o.telemetry)),
            "k" => watch_friendly_flow(&session, p),
            "w" => do_save(&session),
            _ => {
                if prompt_choice("Save before quitting? [y/n] ", &["y", "n"]) == "y" {
                    do_save(&session);
                }
                return;
            }
        }
    }
}

/// R14's grouped menu.
///
/// **Presentational grouping only — the keyspace stays flat.** No sub-menus:
/// they would add a navigation step to every action to save one line of screen,
/// and every action here stays exactly one keystroke from the main screen.
///
/// Mnemonic letters rather than numbers, because a numbered menu renumbers every
/// time an entry lands and `[s]` for squad does not.
fn menu(unread: usize, p: Palette) -> String {
    let inbox = if unread > 0 {
        format!("[i] Inbox ({unread})")
    } else {
        "[i] Inbox".to_string()
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "  {}  [s] Squad   [l] Lineup & tactics   [f] Fitness & availability",
        p.paint("SQUAD", Sem::Muted)
    );
    let _ = writeln!(
        out,
        "  {}   [t] Table   [x] Fixtures   [$] Finances   [m] Transfers",
        p.paint("CLUB", Sem::Muted)
    );
    let _ = writeln!(
        out,
        "  {}   {inbox}   [r] Reports   [k] Friendly (tactics sandbox)",
        p.paint("DESK", Sem::Muted)
    );
    let _ = writeln!(
        out,
        "  ── {} ──          [w] Save   [q] Quit",
        p.paint("[enter] Advance matchday", Sem::Emphasis)
    );
    out
}
