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
//! | `main` | entry, `game_loop`, the menu |
//! | `screens` | read-only renders — each a pure function returning `String` |
//! | `flows` | multi-step interactions (lineup, transfers, advance, friendly) |
//! | `render` | shared formatting and derived readings (the `Sem` colour vocabulary joins at U2) |
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

use fforge_core::{SeasonTelemetry, Session};

pub const SAVE_PATH: &str = "savegame.fml";

fn main() {
    println!("==========================================");
    println!("   FOOTBALL FORGE");
    println!("==========================================");
    loop {
        println!("\n[1] New game   [2] Load game   [0] Quit");
        match prompt_choice("> ", &["1", "2", "0"]).as_str() {
            "1" => {
                if let Some((session, telemetry)) = new_game_flow() {
                    game_loop(session, telemetry);
                }
            }
            "2" => match load_flow() {
                Some((session, telemetry)) => game_loop(session, telemetry),
                None => println!("No save found at ./{SAVE_PATH} (or it failed to load)."),
            },
            _ => {
                println!("Goodbye.");
                return;
            }
        }
    }
}

fn game_loop(mut session: Session, mut telemetry: SeasonTelemetry) {
    loop {
        if session.state.season_over() {
            print!("{}", screens::season_end::render(&session, &telemetry));
            if prompt_choice("Save the finished season? [y/n] ", &["y", "n"]) == "y" {
                do_save(&session);
            }
            return;
        }
        print!("{}", screens::header::render(&session));
        println!(
            "[1] Squad  [2] Table  [3] Fixtures  [4] Set lineup  [5] Advance matchday\n[6] League stats  [7] Save  [8] Save & quit  [9] Transfers  [0] Quit without saving"
        );
        match prompt_choice("> ", &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]).as_str() {
            "1" => print!("{}", screens::squad::render(&session)),
            "2" => print!("{}", screens::table::render(&session)),
            "3" => print!("{}", screens::fixtures::render(&session)),
            "4" => set_lineup_flow(&mut session, &mut telemetry),
            "5" => advance_flow(&mut session, &mut telemetry),
            "6" => print!("{}", screens::stats::render(&telemetry)),
            "7" => do_save(&session),
            "8" => {
                do_save(&session);
                return;
            }
            "9" => transfer_flow(&mut session, &mut telemetry),
            _ => return,
        }
    }
}
