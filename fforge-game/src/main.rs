//! fm-game — layer 5: the CLI presentation.
//!
//! This binary is the only place allowed to touch stdin/stdout and the wall
//! clock (a default seed when the player doesn't supply one — the seed is
//! then *recorded* in `GameStarted`, so the core never sees a clock).

use fforge_core::{Command, Event, SeasonTelemetry, Session,
                  WorldGenConfig,
                  league_table, load_log, match_engine, player_match_preview, save_log,
};
use fforge_domain::{ClubId, FORMATIONS, Lineup, PlayerId, ROLE_WEIGHTS, Role, World,
                    XI,
                    current_ability,
};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Pacing for the humble text match view's line-by-line playback.
const EVENT_DELAY: Duration = Duration::from_millis(120);

const SAVE_PATH: &str = "savegame.fml";

fn main() {
    println!("==========================================");
    println!("   FOOTBALL FORGE — walking skeleton (Phase 1)");
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

// ---------------------------------------------------------------- new / load

fn new_game_flow() -> Option<(Session, SeasonTelemetry)> {
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
    let mut telemetry = SeasonTelemetry::default();
    let session = Session::from_events(vec![opening], &mut [&mut telemetry]);
    Some((session, telemetry))
}

fn load_flow() -> Option<(Session, SeasonTelemetry)> {
    let log = load_log(Path::new(SAVE_PATH)).ok()?;
    let mut telemetry = SeasonTelemetry::default();
    let session = Session::from_events(log, &mut [&mut telemetry]);
    println!(
        "Loaded: {} — matchday {}/{}.",
        session.state.world.club(session.state.player_club).name,
        session.state.current_matchday.min(session.state.last_matchday),
        session.state.last_matchday
    );
    Some((session, telemetry))
}

fn prompt_seed() -> u64 {
    let raw = read_line("World seed (blank = random): ");
    if raw.trim().is_empty() {
        // Wall clock is fine *here* (presentation edge): the chosen seed is
        // recorded in GameStarted, so replay never re-derives it.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xF00D)
    } else {
        raw.trim().parse().unwrap_or_else(|_| {
            // Non-numeric seeds are hashed FNV-style so "juventus" works too.
            raw.trim()
                .bytes()
                .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                    (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
                })
        })
    }
}

// ------------------------------------------------------------------ the loop

fn game_loop(mut session: Session, mut telemetry: SeasonTelemetry) {
    loop {
        if session.state.season_over() {
            season_end_screen(&session, &telemetry);
            if prompt_choice("Save the finished season? [y/n] ", &["y", "n"]) == "y" {
                do_save(&session);
            }
            return;
        }
        header(&session);
        println!(
            "[1] Squad  [2] Table  [3] Fixtures  [4] Set lineup  [5] Advance matchday\n[6] League stats  [7] Save  [8] Save & quit  [0] Quit without saving"
        );
        match prompt_choice("> ", &["1", "2", "3", "4", "5", "6", "7", "8", "0"]).as_str() {
            "1" => squad_screen(&session),
            "2" => table_screen(&session),
            "3" => fixtures_screen(&session),
            "4" => set_lineup_flow(&mut session, &mut telemetry),
            "5" => advance_flow(&mut session, &mut telemetry),
            "6" => stats_screen(&telemetry),
            "7" => do_save(&session),
            "8" => {
                do_save(&session);
                return;
            }
            _ => return,
        }
    }
}

fn header(session: &Session) {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let pos = table_position(session, s.player_club);
    println!(
        "\n=== {} · Matchday {}/{} · {} · position {} ===",
        club.name, s.current_matchday, s.last_matchday, s.date, pos
    );
    let lineup_note = if s.pending_lineup.is_some() {
        "lineup set for next matchday"
    } else if s.last_lineup.is_some() {
        "no new lineup — last XI will be reused"
    } else {
        "no lineup set — assistant will auto-pick"
    };
    println!("    ({lineup_note})");
}

// ------------------------------------------------------------------- screens

fn squad_screen(session: &Session) {
    let s = &session.state;
    let world = &s.world;
    let mut players: Vec<_> = world.club_players(s.player_club).collect();
    players.sort_by_key(|p| (p.natural_role, std::cmp::Reverse(headline_ca(p))));
    println!("\n {:<3} {:<20} {:>3}  {:<4} {:>3}  Best role", "Pos", "Name", "Age", "", "CA");
    for p in players {
        let (best, best_ca) = fforge_domain::best_role(&p.attributes, &ROLE_WEIGHTS);
        let alt = if best != p.natural_role {
            format!("{} ({})", best.short().trim(), best_ca)
        } else {
            String::new()
        };
        println!(
            " {:<3} {:<20} {:>3}  {:<4} {:>3}  {}",
            p.natural_role.short(),
            p.name,
            p.age(s.date),
            "",
            headline_ca(p),
            alt
        );
    }
}

fn headline_ca(p: &fforge_domain::Player) -> u8 {
    current_ability(&p.attributes, p.natural_role, &ROLE_WEIGHTS)
}

fn table_screen(session: &Session) {
    let s = &session.state;
    let table = league_table(&s.world, &s.schedule, &s.results);
    println!(
        "\n     {:<22} {:>2} {:>3} {:>3} {:>3} {:>4} {:>4} {:>4} {:>4}",
        "Club", "", "W", "D", "L", "GF", "GA", "GD", "Pts"
    );
    for (i, row) in table.iter().enumerate() {
        let marker = if row.club == s.player_club { ">" } else { " " };
        println!(
            "{marker}{:>3}. {:<22} {:>2} {:>3} {:>3} {:>3} {:>4} {:>4} {:>+4} {:>4}",
            i + 1,
            s.world.club(row.club).name,
            row.played,
            row.won,
            row.drawn,
            row.lost,
            row.goals_for,
            row.goals_against,
            row.goal_diff(),
            row.points()
        );
    }
}

fn fixtures_screen(session: &Session) {
    let s = &session.state;
    println!("\nMatchday {} fixtures:", s.current_matchday);
    for f in s.fixtures_of_matchday(s.current_matchday) {
        let star = if f.home == s.player_club || f.away == s.player_club {
            " <— your match"
        } else {
            ""
        };
        println!(
            "  {:<22} vs {:<22}{}",
            s.world.club(f.home).name,
            s.world.club(f.away).name,
            star
        );
    }
    if s.current_matchday > 1 {
        let prev = s.current_matchday - 1;
        println!("\nMatchday {prev} results:");
        for f in s.fixtures_of_matchday(prev) {
            if let Some(&(hg, ag)) = s.results.get(&f.id) {
                print_result(&s.world, s.player_club, f.home, f.away, hg, ag);
            }
        }
    }
}

fn stats_screen(telemetry: &SeasonTelemetry) {
    println!("\nLeague-wide telemetry (the calibration harness embryo):");
    println!("  matches played : {}", telemetry.matches);
    println!("  goals per match: {:.2}", telemetry.goals_per_match());
    if telemetry.matches > 0 {
        println!(
            "  home/draw/away : {:.0}% / {:.0}% / {:.0}%",
            100.0 * telemetry.home_wins as f64 / telemetry.matches as f64,
            100.0 * telemetry.draws as f64 / telemetry.matches as f64,
            100.0 * telemetry.away_wins as f64 / telemetry.matches as f64
        );
    }
}

// ------------------------------------------------------------------- lineup

fn set_lineup_flow(session: &mut Session, telemetry: &mut SeasonTelemetry) {
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
        Ok(_) => println!("Lineup submitted for matchday {}.", session.state.current_matchday),
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

// ------------------------------------------------------------------ friendly

/// The Phase 2 "humble text match view" (`DESIGN.md` §9) — a standalone,
/// unrecorded friendly (no `Command`, no `Event`, no fold mutation) that
/// proves the event stream can tell a match's story before any graphical
/// renderer exists. It just prints the stream, unfiltered.
///
/// Not currently reachable from `game_loop`'s menu — the "watch a friendly"
/// option was removed — but kept for now rather than deleted.
#[allow(dead_code)]
fn watch_friendly_flow(session: &Session) {
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

    let home_lineup = match_engine::ai_pick_lineup(world, home_club);
    let away_lineup = match_engine::ai_pick_lineup(world, away_club);

    // A friendly is never recorded through Session::execute — no Command, no
    // Event, no fold mutation — so an ad-hoc wall-clock seed is fine here
    // (this crate's one sanctioned exception, same as prompt_seed).
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xF00D);
    let mut rng = fforge_core::rng::Rng::seed_from(seed);
    let outcome = match_engine::play_match(world, &home_lineup, &away_lineup, &mut rng);

    print_humble_text_view(world, &home_name, &away_name, &outcome);
}

/// Prints the humble text match view (`DESIGN.md` §9): the raw event stream,
/// unfiltered, followed by the final score. Shared by the standalone
/// friendly viewer and, for the human's own fixture, the main game loop's
/// matchday advance.
fn print_humble_text_view(
    world: &fforge_domain::World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
) {
    println!(
        "\n{home_name} vs {away_name} — {} raw events, unfiltered (the humble text match view, DESIGN.md §9):",
        outcome.stream.len()
    );
    // Only worth pacing/skippable when there's an actual terminal on both
    // ends — piped output (tests, redirects) just gets the whole stream at
    // once, same as before this feature existed.
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    if tty {
        println!("(press any key to skip to full time)");
    }
    println!();

    let interactive = tty && crossterm::terminal::enable_raw_mode().is_ok();
    let mut skipping = false;
    for event in &outcome.stream {
        let side_name = match event.side {
            match_engine::Side::Home => home_name,
            match_engine::Side::Away => away_name,
        };
        // The name lookup lives here — this crate owns the `World` and is the
        // only one allowed to touch stdout; `commentary` stays name-resolved
        // and I/O-free (MATCH_MODEL.md §9).
        let actor = world.player(event.actor).name.as_str();
        let line = event.commentary(side_name, actor);
        if interactive {
            // Raw mode turns off the terminal's own \n -> \r\n translation.
            print!("{line}\r\n");
            io::stdout().flush().ok();
        } else {
            println!("{line}");
        }
        if interactive && !skipping && key_pressed_within(EVENT_DELAY) {
            skipping = true;
        }
    }
    if interactive {
        let _ = crossterm::terminal::disable_raw_mode();
    }
    println!("\nFULL TIME: {home_name} {} - {} {away_name}", outcome.home_goals, outcome.away_goals);
}

/// Blocks up to `delay`, watching for a keypress. Returns as soon as one
/// arrives (true) so the caller can stop pacing the rest of the stream;
/// returns false once `delay` elapses with nothing pressed.
fn key_pressed_within(delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        match crossterm::event::poll(remaining) {
            Ok(true) => {
                if matches!(crossterm::event::read(), Ok(crossterm::event::Event::Key(_))) {
                    return true;
                }
                // Some other event (resize, focus change, ...) — keep waiting.
            }
            _ => return false,
        }
    }
}

// ------------------------------------------------------------------ advance

fn advance_flow(session: &mut Session, telemetry: &mut SeasonTelemetry) {
    let md = session.state.current_matchday;
    // Computed from the pre-advance state, using the same lineup selection
    // and seed-derived RNG stream `AdvanceMatchday` is about to use, so it
    // can never disagree with the score actually recorded below.
    let preview = player_match_preview(&session.state);
    let events: Vec<Event> = match session.execute(Command::AdvanceMatchday, &mut [&mut *telemetry])
    {
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
            let f = s.schedule.iter().find(|f| f.id == *fixture).expect("fixture");
            print_result(&s.world, s.player_club, f.home, f.away, *home_goals, *away_goals);
        }
    }
    println!(
        "\nYou are {} after matchday {md}.",
        ordinal(table_position(session, s.player_club))
    );
}

fn season_end_screen(session: &Session, telemetry: &SeasonTelemetry) {
    let s = &session.state;
    println!("\n================ SEASON OVER ================");
    table_screen(session);
    if let Some(champ) = s.champion {
        println!("\nChampions: {}", s.world.club(champ).name);
    }
    let pos = table_position(session, s.player_club);
    println!(
        "You finished {} with {}.",
        ordinal(pos),
        s.world.club(s.player_club).name
    );
    stats_screen(telemetry);
    println!("(Multi-season continuity arrives with Phase 3 — development needs it.)");
}

// ------------------------------------------------------------------ helpers

fn print_result(world: &World, mine: ClubId, home: ClubId, away: ClubId, hg: u8, ag: u8) {
    let marker = if home == mine || away == mine { ">" } else { " " };
    println!(
        "{marker} {:<22} {:>2} - {:<2} {}",
        world.club(home).name,
        hg,
        ag,
        world.club(away).name
    );
}

fn table_position(session: &Session, club: ClubId) -> usize {
    let s = &session.state;
    league_table(&s.world, &s.schedule, &s.results)
        .iter()
        .position(|r| r.club == club)
        .map(|i| i + 1)
        .unwrap_or(0)
}

fn club_avg_ca(world: &World, club: ClubId) -> f64 {
    let players: Vec<_> = world.club_players(club).collect();
    let sum: u32 = players
        .iter()
        .map(|p| current_ability(&p.attributes, p.natural_role, &ROLE_WEIGHTS) as u32)
        .sum();
    sum as f64 / players.len() as f64
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn do_save(session: &Session) {
    match save_log(Path::new(SAVE_PATH), &session.log) {
        Ok(()) => println!("Saved to ./{SAVE_PATH} ({} events).", session.log.len()),
        Err(e) => println!("Save failed: {e}"),
    }
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf).is_err() {
        return String::new();
    }
    buf.trim().to_string()
}

fn prompt_choice(prompt: &str, allowed: &[&str]) -> String {
    loop {
        let input = read_line(prompt);
        if allowed.contains(&input.as_str()) {
            return input;
        }
        println!("Options: {}", allowed.join(", "));
    }
}

fn prompt_number(prompt: &str, lo: usize, hi: usize) -> Option<usize> {
    loop {
        let input = read_line(prompt);
        if input == "q" {
            return None;
        }
        match input.parse::<usize>() {
            Ok(n) if (lo..=hi).contains(&n) => return Some(n),
            _ => println!("Enter a number {lo}–{hi} (or q to abort)."),
        }
    }
}