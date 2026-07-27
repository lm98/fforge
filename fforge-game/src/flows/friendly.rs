//! The standalone friendly: an unrecorded match between any two clubs (no
//! `Command`, no `Event`, no fold mutation) rendered through the humble text
//! match view.
//!
//! **Fate decided (R17):** kept and wired back, not deleted. The reason is
//! U6's tactics picker — a friendly is the only place a manager can try a
//! shape without spending a matchday on it, which turns this from a leftover
//! demo into a tactics sandbox. It is reachable from the menu as of U7; until
//! that task lands it stays `#[allow(dead_code)]` rather than being deleted
//! and re-added.

use crate::flows::match_view::print_humble_text_view;
use crate::input::prompt_number;
use fforge_core::{Session, match_engine};
use fforge_domain::PlayerId;
use std::collections::BTreeMap;

#[allow(dead_code)]
pub fn watch_friendly_flow(session: &Session) {
    let world = &session.state.world;
    let clubs = world.competition.clubs.clone();

    println!("\nPick the home club:");
    for (i, &cid) in clubs.iter().enumerate() {
        println!("[{:>2}] {}", i + 1, world.club(cid).name);
    }
    let Some(hi) = prompt_number("Home club: ", 1, clubs.len()) else {
        return;
    };
    println!("\nPick the away club:");
    for (i, &cid) in clubs.iter().enumerate() {
        println!("[{:>2}] {}", i + 1, world.club(cid).name);
    }
    let Some(ai) = prompt_number("Away club: ", 1, clubs.len()) else {
        return;
    };
    let home_club = clubs[hi - 1];
    let away_club = clubs[ai - 1];
    let home_name = world.club(home_club).name.clone();
    let away_name = world.club(away_club).name.clone();

    let suspended = session.state.suspended_players();
    let home_lineup =
        match_engine::ai_pick_lineup_available(world, home_club, session.state.date, &suspended);
    let away_lineup =
        match_engine::ai_pick_lineup_available(world, away_club, session.state.date, &suspended);

    // A friendly is never recorded through Session::execute — no Command, no
    // Event, no fold mutation — so an ad-hoc wall-clock seed is fine here
    // (this crate's one sanctioned exception, same as prompt_seed).
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xF00D);
    let mut rng = fforge_core::rng::Rng::seed_from(seed);
    let mut consistency_rng = fforge_core::rng::Rng::seed_from(seed.wrapping_add(1));
    let mut injury_rng = fforge_core::rng::Rng::seed_from(seed.wrapping_add(2));
    let mut foul_rng = fforge_core::rng::Rng::seed_from(seed.wrapping_add(3));
    // A real GameState is available even for an unrecorded friendly, so this
    // reads the same accumulated condition a real fixture would.
    let conditions: BTreeMap<PlayerId, f64> = home_lineup
        .players
        .iter()
        .chain(&away_lineup.players)
        .map(|&pid| (pid, session.state.condition(pid)))
        .collect();
    let outcome = match_engine::play_match(
        world,
        &home_lineup,
        &away_lineup,
        &mut rng,
        &mut consistency_rng,
        &mut injury_rng,
        &mut foul_rng,
        &match_engine::Knobs::default(),
        &conditions,
        session.state.date,
    );

    print_humble_text_view(world, &home_name, &away_name, &outcome);
}
