//! The bench and the substitution plan — a condition→action rule-list builder,
//! sibling of the transfer draft builder (`flows::transfers`).
//!
//! **This is the hardest interface in the game, and R18 says so in advance.**
//! A `SubRule` is a composite value (a list of conditions plus one action),
//! rules are evaluated in list order, and the whole thing is authored before
//! kickoff with no chance to correct it mid-match (`MATCH_MODEL.md` §16: the
//! plan *is* the decision, precisely so evaluation stays RNG- and I/O-free).
//! So the editor needs everything a terminal is worst at at once: a persistent
//! representation of a form, per-row editing, reordering, and undo. What it
//! got instead is a small invented command language — `d N`, `u N` — the same
//! one the transfer shortlist had to invent, for the same reason.
//!
//! Two deliberate concessions to that, both of which exist to reduce how much
//! the player must hold in his head:
//!
//! - **The plan is always shown in full, rendered back as English**, before
//!   every prompt. There is no "current rule" hidden in the editor's state.
//! - **Rules are seeded from last week's plan**, so the common case is
//!   reviewing a plan rather than authoring one.
//!
//! Colour axis: **whether a rule can still fire.** A rule naming a player who
//! is no longer on the team sheet is a silent no-op in the engine (§16: "not a
//! validation error, since a plan is authored before kickoff"), which is
//! exactly the kind of quiet failure a manager should be told about. Stale
//! rules read `Warn` and are labelled `(stale)`; live ones read plain.

use crate::input::{prompt_choice, prompt_number, read_line};
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::match_engine::SUB_CHECKPOINTS;
use fforge_domain::{
    BENCH_SIZE, MAX_SUBSTITUTIONS, Mentality, PlayerId, Pressing, ROLE_WEIGHTS, ScoreState,
    SubAction, SubCondition, SubRule, Tempo, Width, World, current_ability,
};
use std::fmt::Write as _;

/// The bench and plan being edited, alongside the XI they are attached to.
pub struct Plan {
    pub bench: Vec<PlayerId>,
    pub rules: Vec<SubRule>,
}

/// Edit the bench and the substitution plan for a team sheet whose XI is
/// already chosen. Returns `None` if the player aborts the whole team sheet.
pub fn edit(
    world: &World,
    squad: &[PlayerId],
    xi: &[PlayerId],
    start: Plan,
    p: Palette,
) -> Option<Plan> {
    let mut plan = start;
    // Anyone in the squad who isn't starting is benchable.
    let eligible: Vec<PlayerId> = squad.iter().copied().filter(|q| !xi.contains(q)).collect();
    loop {
        print!("{}", describe(world, xi, &plan, p));
        println!(
            "  [b] pick bench   [a] auto-fill bench   [n] new rule\n  [d N] drop rule N   [u N] move rule N up   [c] clear rules   [k] done   [q] abort"
        );
        let input = read_line("> ");
        let mut parts = input.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let idx = parts.next().and_then(|n| n.parse::<usize>().ok());
        match (cmd, idx) {
            ("b", _) => pick_bench(world, &eligible, &mut plan),
            ("a", _) => {
                auto_fill_bench(world, &eligible, &mut plan);
                println!("Bench auto-filled with the best remaining players.");
            }
            ("n", _) => {
                if let Some(rule) = new_rule(world, xi, &plan) {
                    plan.rules.push(rule);
                }
            }
            ("d", Some(i)) if (1..=plan.rules.len()).contains(&i) => {
                plan.rules.remove(i - 1);
            }
            ("u", Some(i)) if (2..=plan.rules.len()).contains(&i) => {
                plan.rules.swap(i - 1, i - 2);
            }
            ("c", _) => plan.rules.clear(),
            ("k", _) => return Some(plan),
            ("q", _) => return None,
            _ => println!("Commands: b, a, n, 'd N', 'u N', c, k, q."),
        }
    }
}

fn describe(world: &World, xi: &[PlayerId], plan: &Plan, p: Palette) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\nBench ({}/{}):{}",
        plan.bench.len(),
        BENCH_SIZE,
        if plan.bench.is_empty() {
            " empty — no substitutions are possible.".to_string()
        } else {
            String::new()
        }
    );
    if !plan.bench.is_empty() {
        let mut t = Table::new(vec![
            Col::right("#", 3),
            Col::left("Pos", 4),
            Col::left("Name", 20),
            Col::right("CA", 3),
        ])
        .indent("  ");
        for (i, &pid) in plan.bench.iter().enumerate() {
            let player = world.player(pid);
            t.row(vec![
                Cell::new((i + 1).to_string()),
                Cell::new(player.natural_role.short()),
                Cell::new(player.name.clone()),
                Cell::new(
                    current_ability(&player.attributes, player.natural_role, &ROLE_WEIGHTS)
                        .to_string(),
                ),
            ]);
        }
        out.push_str(&t.render(p));
    }

    let _ = writeln!(
        out,
        "\nSubstitution plan ({} rule(s), tried in order at half-time, {}, and immediately after any injury or card):",
        plan.rules.len(),
        SUB_CHECKPOINTS
            .iter()
            .map(|m| format!("{m:.0}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if plan.rules.is_empty() {
        let _ = writeln!(
            out,
            "{}",
            p.paint(
                "  No rules — nobody comes on and nothing changes.",
                Sem::Muted
            )
        );
    }
    for (i, rule) in plan.rules.iter().enumerate() {
        let stale = !rule_is_live(rule, xi, &plan.bench);
        let line = format!(
            "  {}. {}{}",
            i + 1,
            render_rule(world, rule),
            if stale { "   (stale)" } else { "" }
        );
        let _ = writeln!(
            out,
            "{}",
            p.paint(&line, if stale { Sem::Warn } else { Sem::Ok })
        );
    }

    let substitutions = plan
        .rules
        .iter()
        .filter(|r| matches!(r.action, SubAction::Substitute { .. }))
        .count();
    if substitutions > MAX_SUBSTITUTIONS {
        let _ = writeln!(
            out,
            "{}",
            p.paint(
                &format!(
                    "  ! {substitutions} substitution rules, but only {MAX_SUBSTITUTIONS} can be used — the later ones may never fire."
                ),
                Sem::Warn
            )
        );
    }
    out
}

/// A rule is live if every player it names is still on the team sheet. The
/// engine treats a rule naming a departed player as a silent no-op (§16); the
/// editor is where that can still be caught.
fn rule_is_live(rule: &SubRule, xi: &[PlayerId], bench: &[PlayerId]) -> bool {
    let dressed = |pid: PlayerId| xi.contains(&pid) || bench.contains(&pid);
    let action_ok = match rule.action {
        SubAction::Substitute {
            player_out,
            player_in,
        } => xi.contains(&player_out) && bench.contains(&player_in),
        _ => true,
    };
    action_ok
        && rule.conditions.iter().all(|c| match c {
            SubCondition::PlayerConditionBelow(pid, _) | SubCondition::PlayerInjured(pid) => {
                dressed(*pid)
            }
            _ => true,
        })
}

/// The rule rendered back as English — the editor's answer to having no
/// persistent form widget: the plan is always fully visible, always in prose.
fn render_rule(world: &World, rule: &SubRule) -> String {
    let name = |pid: PlayerId| world.player(pid).name.clone();
    let action = match rule.action {
        SubAction::Substitute {
            player_out,
            player_in,
        } => format!("bring {} on for {}", name(player_in), name(player_out)),
        SubAction::SetMentality(m) => format!("switch to {m:?} mentality"),
        SubAction::SetTempo(t) => format!("switch to {t:?} tempo"),
        SubAction::SetWidth(w) => format!("switch to {w:?} width"),
        SubAction::SetPressing(pr) => format!("switch to {pr:?} pressing"),
    };
    if rule.conditions.is_empty() {
        return format!("always: {action}");
    }
    let conditions: Vec<String> = rule
        .conditions
        .iter()
        .map(|c| match c {
            SubCondition::MinuteAtLeast(m) => format!("it is {m}' or later"),
            SubCondition::Score(ScoreState::Trailing) => "we are behind".to_string(),
            SubCondition::Score(ScoreState::Level) => "it is level".to_string(),
            SubCondition::Score(ScoreState::Leading) => "we are ahead".to_string(),
            SubCondition::PlayerConditionBelow(pid, pct) => {
                format!("{} is under {pct}% fitness", name(*pid))
            }
            SubCondition::PlayerInjured(pid) => format!("{} is hurt", name(*pid)),
            SubCondition::ManDown => "we are a man down".to_string(),
        })
        .collect();
    format!("if {} — {action}", conditions.join(" and "))
}

fn pick_bench(world: &World, eligible: &[PlayerId], plan: &mut Plan) {
    loop {
        let mut sorted: Vec<PlayerId> = eligible.to_vec();
        sorted.sort_by_key(|&pid| {
            let player = world.player(pid);
            (
                player.natural_role,
                std::cmp::Reverse(current_ability(
                    &player.attributes,
                    player.natural_role,
                    &ROLE_WEIGHTS,
                )),
            )
        });
        println!(
            "\nBench ({}/{}). Toggle a player:",
            plan.bench.len(),
            BENCH_SIZE
        );
        for (i, &pid) in sorted.iter().enumerate() {
            let player = world.player(pid);
            println!(
                "  [{:>2}] {:<4} {:<20} {:>3}{}",
                i + 1,
                player.natural_role.short(),
                player.name,
                current_ability(&player.attributes, player.natural_role, &ROLE_WEIGHTS),
                if plan.bench.contains(&pid) {
                    "  (on the bench)"
                } else {
                    ""
                }
            );
        }
        println!("  [q] done");
        let input = read_line("> ");
        if input.trim() == "q" {
            return;
        }
        match input.trim().parse::<usize>() {
            Ok(i) if (1..=sorted.len()).contains(&i) => {
                let pid = sorted[i - 1];
                if let Some(at) = plan.bench.iter().position(|&b| b == pid) {
                    plan.bench.remove(at);
                } else if plan.bench.len() >= BENCH_SIZE {
                    println!("The bench is full ({BENCH_SIZE}). Drop someone first.");
                } else {
                    plan.bench.push(pid);
                }
            }
            _ => println!("Pick a listed number or 'q'."),
        }
    }
}

/// Fill the bench to capacity with the strongest remaining players, one per
/// role first so the cover is spread — the mechanical half of the job, mirroring
/// the XI picker's own `[a]`.
fn auto_fill_bench(world: &World, eligible: &[PlayerId], plan: &mut Plan) {
    let ca = |pid: PlayerId| {
        let player = world.player(pid);
        current_ability(&player.attributes, player.natural_role, &ROLE_WEIGHTS)
    };
    let mut remaining: Vec<PlayerId> = eligible
        .iter()
        .copied()
        .filter(|q| !plan.bench.contains(q))
        .collect();
    remaining.sort_by_key(|&pid| (std::cmp::Reverse(ca(pid)), pid));

    // First pass: the best available in each role not yet covered on the bench.
    let mut covered: Vec<_> = plan
        .bench
        .iter()
        .map(|&pid| world.player(pid).natural_role)
        .collect();
    for &pid in &remaining {
        if plan.bench.len() >= BENCH_SIZE {
            return;
        }
        let role = world.player(pid).natural_role;
        if !covered.contains(&role) {
            covered.push(role);
            plan.bench.push(pid);
        }
    }
    // Second pass: best of the rest.
    for &pid in &remaining {
        if plan.bench.len() >= BENCH_SIZE {
            return;
        }
        if !plan.bench.contains(&pid) {
            plan.bench.push(pid);
        }
    }
}

fn new_rule(world: &World, xi: &[PlayerId], plan: &Plan) -> Option<SubRule> {
    println!("\nWhat should the rule do?");
    println!("  [1] Bring a substitute on");
    println!("  [2] Change mentality   [3] Change tempo   [4] Change width   [5] Change pressing");
    let action = match prompt_choice("> ", &["1", "2", "3", "4", "5", "q"]).as_str() {
        "1" => substitute_action(world, xi, plan)?,
        "2" => SubAction::SetMentality(pick_level(
            "Mentality",
            &[
                ("Defensive", Mentality::Defensive),
                ("Balanced", Mentality::Balanced),
                ("Attacking", Mentality::Attacking),
            ],
        )?),
        "3" => SubAction::SetTempo(pick_level(
            "Tempo",
            &[
                ("Patient", Tempo::Patient),
                ("Balanced", Tempo::Balanced),
                ("Direct", Tempo::Direct),
            ],
        )?),
        "4" => SubAction::SetWidth(pick_level(
            "Width",
            &[
                ("Narrow", Width::Narrow),
                ("Balanced", Width::Balanced),
                ("Wide", Width::Wide),
            ],
        )?),
        "5" => SubAction::SetPressing(pick_level(
            "Pressing",
            &[
                ("Deep", Pressing::Deep),
                ("Balanced", Pressing::Balanced),
                ("High", Pressing::High),
            ],
        )?),
        _ => return None,
    };

    let mut conditions: Vec<SubCondition> = Vec::new();
    loop {
        println!(
            "\nConditions so far: {}",
            if conditions.is_empty() {
                "none — the rule fires at the first decision point.".to_string()
            } else {
                render_rule(
                    world,
                    &SubRule {
                        conditions: conditions.clone(),
                        action,
                    },
                )
            }
        );
        println!("  [1] from a given minute   [2] scoreline   [3] a player's fitness drops");
        println!("  [4] a player is hurt      [5] we are a man down   [d] done   [q] cancel rule");
        match prompt_choice("> ", &["1", "2", "3", "4", "5", "d", "q"]).as_str() {
            "1" => {
                if let Some(m) = prompt_number("From which minute? ", 1, 90) {
                    conditions.push(SubCondition::MinuteAtLeast(m as u8));
                }
            }
            "2" => {
                if let Some(s) = pick_level(
                    "Scoreline",
                    &[
                        ("Trailing", ScoreState::Trailing),
                        ("Level", ScoreState::Level),
                        ("Leading", ScoreState::Leading),
                    ],
                ) {
                    conditions.push(SubCondition::Score(s));
                }
            }
            "3" => {
                if let Some(pid) = pick_dressed_player(world, xi, plan, "Whose fitness?")
                    && let Some(pct) = prompt_number("Below what percent? ", 1, 99)
                {
                    conditions.push(SubCondition::PlayerConditionBelow(pid, pct as u8));
                }
            }
            "4" => {
                if let Some(pid) = pick_dressed_player(world, xi, plan, "Who?") {
                    conditions.push(SubCondition::PlayerInjured(pid));
                }
            }
            "5" => conditions.push(SubCondition::ManDown),
            "d" => return Some(SubRule { conditions, action }),
            _ => return None,
        }
    }
}

fn substitute_action(world: &World, xi: &[PlayerId], plan: &Plan) -> Option<SubAction> {
    if plan.bench.is_empty() {
        println!("Nobody on the bench — pick a bench first ([b]).");
        return None;
    }
    let player_out = pick_from(world, xi, "Who comes off?")?;
    let player_in = pick_from(world, &plan.bench, "Who comes on?")?;
    Some(SubAction::Substitute {
        player_out,
        player_in,
    })
}

/// Any dressed player — starters and bench alike. A condition may name a
/// substitute (his fitness only starts mattering once he is on), which is why
/// this is a wider set than `pick_from(xi)`.
fn pick_dressed_player(
    world: &World,
    xi: &[PlayerId],
    plan: &Plan,
    prompt: &str,
) -> Option<PlayerId> {
    let dressed: Vec<PlayerId> = xi.iter().chain(&plan.bench).copied().collect();
    pick_from(world, &dressed, prompt)
}

fn pick_from(world: &World, from: &[PlayerId], prompt: &str) -> Option<PlayerId> {
    println!("\n{prompt}");
    for (i, &pid) in from.iter().enumerate() {
        let player = world.player(pid);
        println!(
            "  [{:>2}] {:<4} {:<20} {:>3}",
            i + 1,
            player.natural_role.short(),
            player.name,
            current_ability(&player.attributes, player.natural_role, &ROLE_WEIGHTS)
        );
    }
    let i = prompt_number("> ", 1, from.len())?;
    Some(from[i - 1])
}

fn pick_level<T: Copy>(what: &str, levels: &[(&str, T)]) -> Option<T> {
    println!("\n{what}:");
    for (i, (label, _)) in levels.iter().enumerate() {
        println!("  [{}] {label}", i + 1);
    }
    let i = prompt_number("> ", 1, levels.len())?;
    Some(levels[i - 1].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: PlayerId = PlayerId(1);
    const IN: PlayerId = PlayerId(2);
    const GONE: PlayerId = PlayerId(3);

    fn sub_rule(conditions: Vec<SubCondition>) -> SubRule {
        SubRule {
            conditions,
            action: SubAction::Substitute {
                player_out: OUT,
                player_in: IN,
            },
        }
    }

    #[test]
    fn a_rule_naming_only_dressed_players_is_live() {
        assert!(rule_is_live(
            &sub_rule(vec![SubCondition::MinuteAtLeast(60)]),
            &[OUT],
            &[IN]
        ));
    }

    /// The engine silently no-ops a rule whose players have gone (§16). The
    /// editor is the only place that can still say so.
    #[test]
    fn a_substitution_naming_someone_off_the_sheet_is_stale() {
        // Player coming on is not on the bench.
        assert!(!rule_is_live(&sub_rule(vec![]), &[OUT], &[GONE]));
        // Player coming off is not in the XI.
        assert!(!rule_is_live(&sub_rule(vec![]), &[GONE], &[IN]));
    }

    #[test]
    fn a_condition_naming_someone_off_the_sheet_is_stale() {
        assert!(!rule_is_live(
            &sub_rule(vec![SubCondition::PlayerInjured(GONE)]),
            &[OUT],
            &[IN]
        ));
    }

    /// A tactics-change rule names nobody, so it can never go stale.
    #[test]
    fn a_tactics_rule_is_always_live() {
        let rule = SubRule {
            conditions: vec![SubCondition::ManDown],
            action: SubAction::SetMentality(Mentality::Defensive),
        };
        assert!(rule_is_live(&rule, &[], &[]));
    }
}
