//! The friendly: an unrecorded match (no `Command`, no `Event`, no fold
//! mutation) rendered through the humble text match view.
//!
//! **This is the tactics sandbox** — the reason R17's "wire it back or delete
//! it" was resolved in favour of wiring it back (U1). Your club, an opponent you
//! choose, and any shape you want to try, for the cost of nothing: no matchday
//! spent, no fatigue, no injuries carried, nothing recorded. It is the only
//! place a manager can find out what `Defensive`/`Direct` actually does to his
//! squad before staking three points on it.
//!
//! Because nothing here is recorded, the seed is drawn from the wall clock —
//! this crate's second and last sanctioned clock read (the first is
//! `input::prompt_seed`). There is no `Event`, so there is nothing for a replay
//! to have to reproduce.

use crate::flows::match_view::print_humble_text_view;
use crate::flows::tactics;
use crate::input::prompt_number;
use crate::render::sem::Palette;
use fforge_core::{Session, match_engine};
use fforge_domain::{Lineup, PlayerId};
use std::collections::BTreeMap;

pub fn watch_friendly_flow(session: &Session, p: Palette) {
    let s = &session.state;
    let world = &s.world;
    let mine = s.player_club;

    let opponents: Vec<_> = world
        .competition
        .clubs
        .iter()
        .copied()
        .filter(|&c| c != mine)
        .collect();
    println!(
        "\nFriendly at {} — nothing here is recorded. Pick the opposition:",
        world.club(mine).name
    );
    for (i, &cid) in opponents.iter().enumerate() {
        println!("[{:>2}] {}", i + 1, world.club(cid).name);
    }
    let Some(oi) = prompt_number("Opponent: ", 1, opponents.len()) else {
        return;
    };
    let opponent = opponents[oi - 1];

    let suspended = s.suspended_players();
    // Your own XI is whatever you have already submitted, so the sandbox tries
    // *your* team rather than the assistant's. Only the tactics are re-asked.
    let base: Lineup = s
        .pending_lineup
        .clone()
        .or_else(|| s.last_lineup.clone())
        .unwrap_or_else(|| match_engine::ai_pick_lineup_available(world, mine, s.date, &suspended));
    let Some(chosen) = tactics::pick(base.tactics, tactics::assistant_pick(session), p) else {
        return;
    };
    let mut home_lineup = base;
    home_lineup.tactics = chosen;

    let away_lineup =
        match_engine::ai_pick_lineup_vs(world, opponent, mine, false, s.date, &suspended);

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
        .map(|&pid| (pid, s.condition(pid)))
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
        s.date,
    );

    println!(
        "\nYou played {}; they played {}.",
        tactics::summary(chosen),
        tactics::summary(away_lineup.tactics)
    );
    print_humble_text_view(
        world,
        &world.club(mine).name,
        &world.club(opponent).name,
        &outcome,
    );
}
