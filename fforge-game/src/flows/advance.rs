//! Advancing the matchday: preview the human's own fixture, execute
//! `Command::AdvanceMatchday`, then report results and any transfer window
//! the advance closed.

use crate::Observers;
use crate::flows::match_view::print_humble_text_view;
use crate::render::sem::Palette;
use crate::render::{money, ordinal, result_line, table_position};
use fforge_core::{Command, Event, Session, player_match_preview};
use fforge_domain::{ClubId, World};

pub fn advance_flow(session: &mut Session, o: &mut Observers, p: Palette) {
    let md = session.state.current_matchday;
    // Computed from the pre-advance state, using the same lineup selection
    // and seed-derived RNG stream `AdvanceMatchday` is about to use, so it
    // can never disagree with the score actually recorded below.
    let preview = player_match_preview(&session.state);
    let events: Vec<Event> = match session.execute(Command::AdvanceMatchday, &mut o.all()) {
        Ok(ev) => ev.to_vec(),
        Err(e) => {
            println!("Cannot advance: {e}");
            return;
        }
    };
    let s = &session.state;
    if let Some(outcome) = &preview
        && let Some(f) = s
            .fixtures_of_matchday(md)
            .find(|f| f.home == s.player_club || f.away == s.player_club)
    {
        print_humble_text_view(
            &s.world,
            &s.world.club(f.home).name,
            &s.world.club(f.away).name,
            outcome,
            p,
        );
    }
    println!("\nMatchday {md} results:");
    for event in &events {
        if let Event::MatchPlayed {
            fixture,
            home_goals,
            away_goals,
            ..
        } = event
        {
            let f = s
                .schedule
                .iter()
                .find(|f| f.id == *fixture)
                .expect("fixture");
            println!(
                "{}",
                result_line(
                    &s.world,
                    s.player_club,
                    f.home,
                    f.away,
                    *home_goals,
                    *away_goals,
                    p
                )
            );
        }
    }
    println!(
        "\nYou are {} after matchday {md}.",
        ordinal(table_position(session, session.state.player_club))
    );
    print_transfer_window_outcome(&session.state.world, session.state.player_club, &events);
}

/// Reports a transfer window's outcome the moment `AdvanceMatchday` crosses
/// its close date (`TRANSFER_MODEL.md` §10): every deal involving the
/// human's own club, in the same event batch the window resolved in — no
/// separate polling, since `Event::TransferWindowClosed` only ever appears
/// alongside whatever `Event::TransferCompleted`s that window produced.
fn print_transfer_window_outcome(world: &World, mine: ClubId, events: &[Event]) {
    if !events
        .iter()
        .any(|e| matches!(e, Event::TransferWindowClosed { .. }))
    {
        return;
    }
    let transfers: Vec<&Event> = events
        .iter()
        .filter(|e| matches!(e, Event::TransferCompleted { .. }))
        .collect();
    println!(
        "\nTransfer window closed: {} deal(s) league-wide.",
        transfers.len()
    );
    let mut any_of_mine = false;
    for e in transfers {
        let Event::TransferCompleted {
            player,
            from,
            to,
            fee,
            ..
        } = e
        else {
            unreachable!("filtered to TransferCompleted above");
        };
        if *to != mine && *from != Some(mine) {
            continue;
        }
        any_of_mine = true;
        let name = &world.player(*player).name;
        if *to == mine {
            let seller = from
                .map(|c| world.club(c).name.clone())
                .unwrap_or_else(|| "a free transfer".to_string());
            println!("  IN:  {name} joins from {seller} for {}.", money(*fee));
        } else {
            println!(
                "  OUT: {name} joins {} for {}.",
                world.club(*to).name,
                money(*fee)
            );
        }
    }
    if !any_of_mine {
        println!("  No incoming or outgoing transfers for you this window.");
    }
}
