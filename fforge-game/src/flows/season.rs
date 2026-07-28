//! The season boundary: read the final table, then roll into the next season
//! or stop here.
//!
//! `Command::StartNextSeason` runs the offseason development ticks across the
//! summer break and lays out a fresh schedule on the *developed* world
//! (`DEVELOPMENT_MODEL.md`) — so the interesting thing about a rollover is not
//! that it happened, it is what it did to the squad. This flow reports that:
//! whose ability moved over the summer, and by how much.
//!
//! **Colour axis: direction of development.** Growth reads `Good`, decline
//! `Warn`; the signed delta column says the same thing without ink. Nothing
//! here is an alarm, so no red — a veteran losing a point of pace is the model
//! working, not a problem.

use crate::Observers;
use crate::flows::save::do_save;
use crate::input::{prompt_choice, prompt_menu};
use crate::render::headline_ca;
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use crate::screens;
use fforge_core::{Command, Session};
use fforge_domain::PlayerId;
use std::collections::BTreeMap;

/// How many movers to report in each direction. Enough to see the shape of the
/// summer without turning the boundary into another squad screen.
const MOVERS_SHOWN: usize = 5;

/// Returns `true` if the player rolled into a new season (the game loop
/// continues), `false` to leave the game.
pub fn season_end_flow(session: &mut Session, o: &mut Observers, p: Palette) -> bool {
    print!("{}", screens::season_end::render(session, &o.telemetry, p));
    loop {
        println!("  ── [enter] Start next season ──          [w] Save   [q] Quit");
        match prompt_menu("> ", &["", "w", "q"]).as_str() {
            "" => return start_next_season(session, o, p),
            "w" => do_save(session),
            _ => {
                if prompt_choice("Save before quitting? [y/n] ", &["y", "n"]) == "y" {
                    do_save(session);
                }
                return false;
            }
        }
    }
}

fn start_next_season(session: &mut Session, o: &mut Observers, p: Palette) -> bool {
    // Snapshot the squad's ability *before* the offseason ticks fold in, so
    // the report is a real before/after rather than a re-derivation.
    let before = squad_ability(session);
    if let Err(e) = session.execute(Command::StartNextSeason, &mut o.all()) {
        println!("Cannot start the next season: {e}");
        return false;
    }
    let after = squad_ability(session);
    println!(
        "\n=== {} — season {} ===",
        session.state.world.club(session.state.player_club).name,
        session.state.date.year()
    );
    print!("{}", summer_report(session, &before, &after, p));
    true
}

/// Headline CA for every player currently on the human's books.
fn squad_ability(session: &Session) -> BTreeMap<PlayerId, u8> {
    let s = &session.state;
    s.world
        .club_players(s.player_club)
        .map(|player| (player.id, headline_ca(player)))
        .collect()
}

/// Who moved over the summer. Players who joined or left across the boundary
/// are simply absent from one side and skipped — a new arrival has no "before"
/// to compare against, and reporting him as +70 would be nonsense.
fn summer_report(
    session: &Session,
    before: &BTreeMap<PlayerId, u8>,
    after: &BTreeMap<PlayerId, u8>,
    p: Palette,
) -> String {
    let mut moved: Vec<(PlayerId, i16)> = after
        .iter()
        .filter_map(|(&pid, &now)| {
            let then = *before.get(&pid)?;
            let delta = now as i16 - then as i16;
            (delta != 0).then_some((pid, delta))
        })
        .collect();
    if moved.is_empty() {
        return "  Nobody's ability moved over the summer.\n".to_string();
    }
    moved.sort_by_key(|&(pid, delta)| (std::cmp::Reverse(delta), pid));

    // Biggest risers first, then the biggest fallers — `moved` is already
    // sorted descending, so the fallers are its tail read backwards.
    let risers: Vec<(PlayerId, i16)> = moved
        .iter()
        .copied()
        .filter(|(_, d)| *d > 0)
        .take(MOVERS_SHOWN)
        .collect();
    let fallers: Vec<(PlayerId, i16)> = moved
        .iter()
        .rev()
        .copied()
        .filter(|(_, d)| *d < 0)
        .take(MOVERS_SHOWN)
        .collect();

    let mut t = Table::new(vec![
        Col::left("Name", 20),
        Col::right("CA", 3),
        Col::right("", 5),
    ]);
    for &(pid, delta) in risers.iter().chain(fallers.iter()) {
        let player = session.state.world.player(pid);
        t.row_all(
            vec![
                Cell::new(player.name.clone()),
                Cell::new(after[&pid].to_string()),
                Cell::new(format!("{delta:+}")),
            ],
            if delta > 0 { Sem::Good } else { Sem::Warn },
        );
    }
    format!("\n Development over the summer:\n{}", t.render(p))
}
