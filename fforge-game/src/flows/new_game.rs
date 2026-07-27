//! Starting a new game (world seed, club pick) and loading a saved one.

use crate::input::{prompt_number, prompt_seed};
use crate::render::club_avg_ca;
use crate::{Observers, SAVE_PATH};
use fforge_core::{Event, Session, WorldGenConfig, load_log};
use std::path::Path;

pub fn new_game_flow() -> Option<(Session, Observers)> {
    let seed = prompt_seed();
    let cfg = WorldGenConfig::default();
    let (world, schedule, start_date) = fforge_core::generate(seed, &cfg);

    println!("\nWorld seed: {seed}");
    println!("League: {} — pick your club:\n", world.competition.name);
    println!("     {:<22} {:>7}", "Club", "Avg CA");
    let clubs = world.competition.clubs.clone();
    for (i, &cid) in clubs.iter().enumerate() {
        println!(
            "[{:>2}] {:<22} {:>7}",
            i + 1,
            world.club(cid).name,
            format!("{:.0}", club_avg_ca(&world, cid))
        );
    }
    let pick = prompt_number("Club number: ", 1, clubs.len())? - 1;
    let player_club = clubs[pick];
    if let Some(old_boss) = world.manager_of(player_club) {
        println!(
            "\nYou replace {} as manager of {}. Good luck.",
            old_boss.name,
            world.club(player_club).name
        );
    }

    let opening = Event::GameStarted {
        seed,
        start_date,
        player_club,
        world,
        schedule,
    };
    let mut observers = Observers::default();
    let session = Session::from_events(vec![opening], &mut observers.all());
    Some((session, observers))
}

pub fn load_flow() -> Option<(Session, Observers)> {
    let log = load_log(Path::new(SAVE_PATH)).ok()?;
    let mut observers = Observers::default();
    // Replay feeds every event back through both observers, so the
    // event-derived half of the inbox rebuilds for free. The state-condition
    // half does not: `check_conditions` runs at command boundaries, and a cold
    // load has none to replay — it re-fires for whatever is true *now*, which
    // is the same asymmetry match commentary already has (`fforge-core::news`).
    let session = Session::from_events(log, &mut observers.all());
    println!(
        "Loaded: {} — matchday {}/{}.",
        session.state.world.club(session.state.player_club).name,
        session
            .state
            .current_matchday
            .min(session.state.last_matchday),
        session.state.last_matchday
    );
    Some((session, observers))
}
