//! The tactics picker, part of the lineup flow.
//!
//! Tactics and the XI ride the same `Lineup` decision value
//! (`TACTICS_MODEL.md` §6), so they belong in one flow rather than two menu
//! entries: a team sheet is one submission, and splitting it would let a player
//! submit an XI and silently keep last week's shape.
//!
//! **No effect magnitudes on screen.** Every level is described by direction
//! and cost in plain language, never by a number. Two reasons: `TACTICS_MODEL.md`
//! §3 mixes two lever classes whose magnitudes have already been re-fitted twice
//! (§5's T7-R finding, §9 items 6–7), so any number here is a hostage to the
//! next calibration pass; and a manager wants to know *what a shape does*, not
//! what multiplier it applies.

use crate::input::read_line;
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::{Session, match_engine};
use fforge_domain::{ClubId, Mentality, Pressing, Tactics, Tempo, Width};

/// The assistant's recommendation for the upcoming fixture — the same
/// `ai_pick_tactics` policy every AI side runs (`TACTICS_MODEL.md` §7), so the
/// human starts from a real shape rather than four `Balanced` shrugs.
///
/// `None` when the club has no fixture this matchday, since the policy reads the
/// strength gap against a specific opponent.
pub fn assistant_pick(session: &Session) -> Option<Tactics> {
    let s = &session.state;
    let f = s
        .fixtures_of_matchday(s.current_matchday)
        .find(|f| f.home == s.player_club || f.away == s.player_club)?;
    let is_home = f.home == s.player_club;
    let opponent: ClubId = if is_home { f.away } else { f.home };
    Some(match_engine::ai_pick_tactics(
        &s.world,
        s.player_club,
        opponent,
        is_home,
        &match_engine::AiTacticKnobs::default(),
    ))
}

/// Interactive picker. Returns `None` if the player aborts, which aborts the
/// whole team-sheet submission — the XI and the tactics are one decision.
pub fn pick(start: Tactics, suggested: Option<Tactics>, p: Palette) -> Option<Tactics> {
    let mut t = start;
    loop {
        print!("{}", describe(t, suggested, p));
        println!(
            "  [m] mentality  [t] tempo  [w] width  [r] pressing   (each cycles)\n  [a] assistant's pick   [n] all balanced   [d] done   [q] abort"
        );
        match read_line("> ").as_str() {
            "m" => t.mentality = cycle_mentality(t.mentality),
            "t" => t.tempo = cycle_tempo(t.tempo),
            "w" => t.width = cycle_width(t.width),
            "r" => t.pressing = cycle_pressing(t.pressing),
            "a" => {
                if let Some(s) = suggested {
                    t = s;
                } else {
                    println!("No fixture to advise on — pick a shape yourself.");
                }
            }
            "n" => t = Tactics::neutral(),
            "d" => return Some(t),
            "q" => return None,
            _ => println!("Pick m, t, w, r, a, n, d, or q."),
        }
    }
}

/// The four instructions, their current level, and what that level means.
///
/// **Colour axis: departure from neutral** — an instruction set to something
/// other than `Balanced` is a deliberate choice and reads `Emphasis`; the ones
/// left alone recede to `Muted`. Not a good/bad axis, because none of these is
/// better than another: `TACTICS_MODEL.md` §9 item 6's whole finding is that
/// non-dominance is squad-conditional, and colouring a level as "good" would
/// assert exactly the dominance that was fitted out.
///
/// The `<` marker on the assistant's own choice is the non-colour carrier.
fn describe(t: Tactics, suggested: Option<Tactics>, p: Palette) -> String {
    let mut table = Table::new(vec![
        Col::left("Instruction", 11),
        Col::left("Setting", 10),
        Col::left("", 2),
        Col::left("", 0),
    ]);
    let rows: [(&str, String, bool, &str); 4] = [
        (
            "Mentality",
            format!("{:?}", t.mentality),
            suggested.is_some_and(|s| s.mentality == t.mentality),
            mentality_meaning(t.mentality),
        ),
        (
            "Tempo",
            format!("{:?}", t.tempo),
            suggested.is_some_and(|s| s.tempo == t.tempo),
            tempo_meaning(t.tempo),
        ),
        (
            "Width",
            format!("{:?}", t.width),
            suggested.is_some_and(|s| s.width == t.width),
            width_meaning(t.width),
        ),
        (
            "Pressing",
            format!("{:?}", t.pressing),
            suggested.is_some_and(|s| s.pressing == t.pressing),
            pressing_meaning(t.pressing),
        ),
    ];
    for (name, setting, is_suggested, meaning) in rows {
        let deliberate = setting != "Balanced";
        table.row_all(
            vec![
                Cell::new(name),
                Cell::new(setting),
                Cell::new(if is_suggested { "<" } else { "" }),
                Cell::new(meaning),
            ],
            if deliberate {
                Sem::Emphasis
            } else {
                Sem::Muted
            },
        );
    }
    let mut out = String::from("\nTactics:\n");
    out.push_str(&table.render(p));
    out.push_str(&match suggested {
        Some(s) => format!(
            "  < marks the assistant's pick for this fixture ({}).\n",
            summary(s)
        ),
        None => "  No fixture this matchday, so the assistant has no read to offer.\n".to_string(),
    });
    out
}

/// One-line summary, for the confirmation screen and the assistant's note.
pub fn summary(t: Tactics) -> String {
    format!(
        "{:?} / {:?} / {:?} / {:?}",
        t.mentality, t.tempo, t.width, t.pressing
    )
}

fn mentality_meaning(m: Mentality) -> &'static str {
    match m {
        Mentality::Defensive => "keep men back; you create less and concede less",
        Mentality::Balanced => "no deliberate risk either way",
        Mentality::Attacking => "commit men forward; more chances at both ends",
    }
}

fn tempo_meaning(t: Tempo) -> &'static str {
    match t {
        Tempo::Patient => "work it forward in safer passes; slower, surer build-up",
        Tempo::Balanced => "no deliberate preference",
        Tempo::Direct => "move it forward fast, accepting more turnovers",
    }
}

fn width_meaning(w: Width) -> &'static str {
    match w {
        Width::Narrow => "come through the middle rather than the flanks",
        Width::Balanced => "no deliberate preference",
        Width::Wide => "go round the outside and cross",
    }
}

fn pressing_meaning(p: Pressing) -> &'static str {
    match p {
        Pressing::Deep => "defend from your own block; legs last the ninety",
        Pressing::Balanced => "no deliberate preference",
        Pressing::High => "contest their build-up high up the pitch; tiring",
    }
}

fn cycle_mentality(m: Mentality) -> Mentality {
    match m {
        Mentality::Defensive => Mentality::Balanced,
        Mentality::Balanced => Mentality::Attacking,
        Mentality::Attacking => Mentality::Defensive,
    }
}

fn cycle_tempo(t: Tempo) -> Tempo {
    match t {
        Tempo::Patient => Tempo::Balanced,
        Tempo::Balanced => Tempo::Direct,
        Tempo::Direct => Tempo::Patient,
    }
}

fn cycle_width(w: Width) -> Width {
    match w {
        Width::Narrow => Width::Balanced,
        Width::Balanced => Width::Wide,
        Width::Wide => Width::Narrow,
    }
}

fn cycle_pressing(p: Pressing) -> Pressing {
    match p {
        Pressing::Deep => Pressing::Balanced,
        Pressing::Balanced => Pressing::High,
        Pressing::High => Pressing::Deep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cycling any instruction three times returns it to where it started —
    /// which is what makes "each cycles" a safe thing to tell the player.
    #[test]
    fn every_instruction_cycles_through_all_three_levels() {
        let mut t = Tactics::neutral();
        for _ in 0..3 {
            t.mentality = cycle_mentality(t.mentality);
            t.tempo = cycle_tempo(t.tempo);
            t.width = cycle_width(t.width);
            t.pressing = cycle_pressing(t.pressing);
        }
        assert_eq!(t, Tactics::neutral());
    }

    /// No level's description may quote an effect magnitude — `TACTICS_MODEL.md`
    /// §3's numbers have been re-fitted twice already, and a number on screen is
    /// a hostage to the next pass.
    #[test]
    fn no_meaning_quotes_a_number() {
        let meanings: Vec<&str> = [
            mentality_meaning(Mentality::Defensive),
            mentality_meaning(Mentality::Balanced),
            mentality_meaning(Mentality::Attacking),
            tempo_meaning(Tempo::Patient),
            tempo_meaning(Tempo::Balanced),
            tempo_meaning(Tempo::Direct),
            width_meaning(Width::Narrow),
            width_meaning(Width::Balanced),
            width_meaning(Width::Wide),
            pressing_meaning(Pressing::Deep),
            pressing_meaning(Pressing::Balanced),
            pressing_meaning(Pressing::High),
        ]
        .to_vec();
        for m in meanings {
            assert!(
                !m.chars().any(|c| c.is_ascii_digit()),
                "tactics description quotes a magnitude: {m:?}"
            );
        }
    }
}
