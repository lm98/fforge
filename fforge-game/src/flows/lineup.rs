//! The team sheet: formation, XI, and tactics, submitted together as one
//! `Command::SubmitLineup`.
//!
//! Tactics ride the same `Lineup` decision value as the XI
//! (`TACTICS_MODEL.md` §6), so they are one flow, not two menu entries — see
//! `flows::tactics` for the picker itself.

use crate::Observers;
use crate::flows::subs::{self, Plan};
use crate::flows::tactics;
use crate::input::{prompt_choice, prompt_number, read_line};
use crate::render::headline_ca;
use crate::render::sem::Palette;
use fforge_core::{Command, Session, match_engine};
use fforge_domain::{
    FORMATIONS, Lineup, PlayerId, ROLE_WEIGHTS, Role, Tactics, World, XI, current_ability,
};

pub fn set_lineup_flow(session: &mut Session, o: &mut Observers, p: Palette) {
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

    // The picker starts from last week's shape (or neutral, first time out) —
    // *not* the assistant's read. Seeding it with `suggested` made [a] a
    // silent no-op on the common path (the picker already showed the
    // suggestion), which read as "assistant's pick does nothing". Starting
    // from the previous shape means [a] is a real, visible action: it
    // actually moves every instruction onto the assistant's recommendation.
    let suggested = tactics::assistant_pick(session);
    let start = session
        .state
        .last_lineup
        .as_ref()
        .map(|l| l.tactics)
        .unwrap_or_else(Tactics::neutral);
    let Some(chosen_tactics) = tactics::pick(start, suggested, p) else {
        return;
    };

    // The bench and the plan ride the same `Lineup` value as the XI and the
    // tactics (`MATCH_MODEL.md` §16), so they are the third and last step of
    // one submission, not a separate menu entry. Seeded from last week's plan:
    // the common case is reviewing a plan, not authoring one.
    let previous = session.state.last_lineup.as_ref();
    let Some(plan) = subs::edit(
        &world,
        &squad,
        &chosen,
        Plan {
            bench: previous.map(|l| l.bench.clone()).unwrap_or_default(),
            rules: previous.map(|l| l.sub_plan.clone()).unwrap_or_default(),
        },
        p,
    ) else {
        return;
    };

    let mut players = [PlayerId(0); XI];
    players.copy_from_slice(&chosen);
    let lineup = Lineup {
        formation: (fi - 1) as u8,
        players,
        tactics: chosen_tactics,
        bench: plan.bench,
        sub_plan: plan.rules,
    };
    println!(
        "\nTeam sheet ({}, {}), strength {:.1}:",
        formation.name,
        tactics::summary(chosen_tactics),
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
    if !lineup.bench.is_empty() {
        println!(
            "  Bench: {}",
            lineup
                .bench
                .iter()
                .map(|&pid| world.player(pid).name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!("  Substitution plan: {} rule(s).", lineup.sub_plan.len());
    if prompt_choice("Confirm? [y/n] ", &["y", "n"]) != "y" {
        return;
    }
    match session.execute(Command::SubmitLineup(lineup), &mut o.all()) {
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
