//! Formation + XI selection, submitted as `Command::SubmitLineup`.

use crate::input::{prompt_choice, prompt_number, read_line};
use crate::render::headline_ca;
use fforge_core::{Command, SeasonTelemetry, Session, match_engine};
use fforge_domain::{
    FORMATIONS, Lineup, PlayerId, ROLE_WEIGHTS, Role, Tactics, World, XI, current_ability,
};

pub fn set_lineup_flow(session: &mut Session, telemetry: &mut SeasonTelemetry) {
    let s = &session.state;
    let world = s.world.clone();
    let squad = world.club(s.player_club).players.clone();

    println!("\nPick a formation:");
    for (i, f) in FORMATIONS.iter().enumerate() {
        let roles: Vec<&str> = f.slots.iter().map(|r| r.short().trim()).collect();
        println!("[{}] {:<7} {}", i + 1, f.name, roles.join("-"));
    }
    let Some(fi) = prompt_number("Formation: ", 1, FORMATIONS.len()) else {
        return;
    };
    let formation = &FORMATIONS[fi - 1];

    let mut chosen: Vec<PlayerId> = Vec::with_capacity(XI);
    let mut slot = 0usize;
    while slot < XI {
        let role = formation.slots[slot];
        let mut candidates: Vec<PlayerId> = squad
            .iter()
            .copied()
            .filter(|p| !chosen.contains(p))
            .collect();
        candidates.sort_by_key(|&pid| {
            let p = world.player(pid);
            (
                std::cmp::Reverse(current_ability(&p.attributes, role, &ROLE_WEIGHTS)),
                pid,
            )
        });

        println!(
            "\nSlot {}/{} — {} ({}):",
            slot + 1,
            XI,
            role.name(),
            role.short().trim()
        );
        for (i, &pid) in candidates.iter().take(8).enumerate() {
            let p = world.player(pid);
            println!(
                "  [{}] {:<20} {:>3} CA here  (natural {} {})",
                i + 1,
                p.name,
                current_ability(&p.attributes, role, &ROLE_WEIGHTS),
                p.natural_role.short().trim(),
                headline_ca(p)
            );
        }
        println!("  [a] auto-fill this and all remaining slots   [q] abort");
        let input = read_line("> ");
        match input.trim() {
            "q" => return,
            "a" => {
                auto_fill(&world, formation.slots, &squad, &mut chosen, slot);
                break;
            }
            n => match n.parse::<usize>() {
                Ok(i) if (1..=candidates.len().min(8)).contains(&i) => {
                    chosen.push(candidates[i - 1]);
                    slot += 1;
                }
                _ => println!("Pick a listed number, 'a', or 'q'."),
            },
        }
    }

    let mut players = [PlayerId(0); XI];
    players.copy_from_slice(&chosen);
    let lineup = Lineup {
        formation: (fi - 1) as u8,
        players,
        // No tactics UI yet — the human's team sheet plays neutral until a
        // later batch adds the picker.
        tactics: Tactics::neutral(),
        // No bench/substitution UI yet either (MATCH_MODEL.md §16, T12) —
        // the human's team sheet plays unsubstituted until a later batch
        // adds the picker.
        bench: Vec::new(),
        sub_plan: Vec::new(),
    };
    println!(
        "\nTeam sheet ({}), strength {:.1}:",
        formation.name,
        match_engine::lineup_strength(&world, &lineup)
    );
    for (i, &pid) in lineup.players.iter().enumerate() {
        let p = world.player(pid);
        println!(
            "  {} {:<20} ({} CA here)",
            formation.slots[i].short(),
            p.name,
            current_ability(&p.attributes, formation.slots[i], &ROLE_WEIGHTS)
        );
    }
    if prompt_choice("Confirm? [y/n] ", &["y", "n"]) != "y" {
        return;
    }
    match session.execute(Command::SubmitLineup(lineup), &mut [&mut *telemetry]) {
        Ok(_) => println!(
            "Lineup submitted for matchday {}.",
            session.state.current_matchday
        ),
        Err(e) => println!("Rejected: {e}"),
    }
}

fn auto_fill(
    world: &World,
    slots: [Role; XI],
    squad: &[PlayerId],
    chosen: &mut Vec<PlayerId>,
    from_slot: usize,
) {
    for &role in slots.iter().skip(from_slot) {
        let best = squad
            .iter()
            .copied()
            .filter(|p| !chosen.contains(p))
            .max_by_key(|&pid| {
                (
                    current_ability(&world.player(pid).attributes, role, &ROLE_WEIGHTS),
                    std::cmp::Reverse(pid),
                )
            })
            .expect("squad larger than XI");
        chosen.push(best);
    }
}
