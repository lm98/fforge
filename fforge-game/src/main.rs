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
use flows::lineup::set_lineup_flow;
use flows::new_game::{load_flow, new_game_flow};
use flows::save::do_save;
use flows::transfers::transfer_flow;
use input::prompt_choice;
use render::sem::Palette;

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

    println!("==========================================");
    println!("   FOOTBALL FORGE");
    println!("==========================================");
    loop {
        println!("\n[1] New game   [2] Load game   [0] Quit");
        match prompt_choice("> ", &["1", "2", "0"]).as_str() {
            "1" => {
                if let Some((session, observers)) = new_game_flow() {
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
            print!("{}", screens::season_end::render(&session, &o.telemetry, p));
            if prompt_choice("Save the finished season? [y/n] ", &["y", "n"]) == "y" {
                do_save(&session);
            }
            return;
        }
        print!("{}", screens::header::render(&session, unread, p));
        println!(
            "[1] Squad  [2] Table  [3] Fixtures  [4] Set lineup  [5] Advance matchday\n[6] League stats  [7] Save  [8] Save & quit  [9] Transfers  [$] Finances  [i] Inbox  [0] Quit without saving"
        );
        match prompt_choice(
            "> ",
            &["1", "2", "3", "4", "5", "6", "7", "8", "9", "$", "i", "0"],
        )
        .as_str()
        {
            "1" => print!("{}", screens::squad::render(&session, p)),
            "2" => print!("{}", screens::table::render(&session, p)),
            "3" => print!("{}", screens::fixtures::render(&session, p)),
            "4" => set_lineup_flow(&mut session, &mut o),
            "5" => advance_flow(&mut session, &mut o, p),
            "6" => print!("{}", screens::stats::render(&o.telemetry)),
            "7" => do_save(&session),
            "8" => {
                do_save(&session);
                return;
            }
            "9" => transfer_flow(&mut session, &mut o, p),
            "$" => print!("{}", screens::finances::render(&session, p)),
            "i" => {
                print!("{}", screens::inbox::render(&session, &o.news, unread, p));
                inbox_seen = screens::inbox::len(&session, &o.news);
            }
            _ => return,
        }
    }
}
