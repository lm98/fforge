//! Watching a match.
//!
//! `DESIGN.md` §9's **humble text match view** — the raw stream, unfiltered,
//! printed in order — is still here and is still what proves the stream can
//! tell a match's story minute by minute. It is now one of three modes rather
//! than the only one, because 867 beats of "picks a pass in midfield" is a
//! proof, not a broadcast:
//!
//! | mode | what it is |
//! |---|---|
//! | **Highlights** (default) | the beats that change something — shots, goals, cards, injuries, substitutions — paced by the gap between them, with a running scoreline and a half-time break |
//! | **Full commentary** | the humble text match view, unchanged: every beat, in order |
//! | **Straight to full time** | no stream at all, just the result and what it left behind |
//!
//! **Nothing is filtered out of the model — only out of the telling.** All
//! three modes read the same `MatchOutcome` and end at the same scoreline;
//! `highlights` is a pure function over the stream, and the stream itself is
//! untouched. That matters beyond taste: `DESIGN.md` §9 built the stream for
//! narratability precisely so a *consumer* could choose its own altitude, and
//! this is the first consumer that does.
//!
//! The line *building* is pure (`commentary_lines`, `highlights`, `stats`,
//! `aftermath`); only the pacing, the mode prompt and the raw-mode toggling
//! live in the printing half. That split is the same one
//! `MatchEvent::commentary` already makes one level down, and it is what lets
//! the reel be tested without a terminal.
//!
//! **Colour axis: consequence severity.** A red card or an injury costs you a
//! player; a yellow might; a shot off target costs nothing. Injuries and reds
//! read `Bad`, yellows `Warn`, and every one of them is labelled in words too,
//! so a piped run loses nothing. Goals and the period markers read `Emphasis`,
//! which is *structural* rather than a value on that axis — it says "this beat
//! is the spine of the match", which is true whichever side scored. Colouring
//! goals by side would be a second axis on one screen, and R15 exists to stop
//! exactly that.

use crate::input::prompt_choice;
use crate::render::sem::{Palette, Sem};
use fforge_core::match_engine::{self, MatchEvent, MatchEventKind, ShotKind, ShotOutcome, Side};
use fforge_core::{Card, CardOutcome, InjuryOutcome};
use fforge_domain::World;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// Pacing for the full commentary's line-by-line playback.
const EVENT_DELAY: Duration = Duration::from_millis(120);

/// Pacing for the highlight reel: `PACE_PER_MINUTE` of real time per minute of
/// match time, clamped so a flurry stays readable and a quiet half hour does
/// not become a coffee break. A reel is ~30 beats, so this lands a match at
/// roughly half a minute of watching.
const PACE_PER_MINUTE: u64 = 70;
const PACE_MIN: Duration = Duration::from_millis(220);
const PACE_MAX: Duration = Duration::from_millis(1_400);

/// The minute the interval falls on. The engine's stream runs 0–90 with no
/// half-time beat of its own, so the break is inserted by the teller.
const HALF_TIME: u8 = 45;

/// Which telling of the match the player asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Highlights,
    Full,
    Result,
}

// ---------------------------------------------------------------------------
// The pure half: turning a `MatchOutcome` into lines.
// ---------------------------------------------------------------------------

/// One printable beat of the reel. `minute` is what the pacer reads to decide
/// how long to hold before this beat; `text` may be several lines (a goal
/// brings its scoreboard with it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub minute: u8,
    pub text: String,
    pub sem: Sem,
}

/// Does this beat change anything a manager would want to see?
///
/// The rule, stated once: **a beat is a highlight if it was a real chance, or
/// if it changed the eleven.** Goals, saves, chances from inside the box,
/// cards, injuries and substitutions qualify; passes, take-ons, clearances,
/// turnovers and plain fouls do not.
///
/// Three exclusions are deliberate and each removes a *duplicate*, not a fact:
///
/// - a `Save` beat is dropped because the shot it answers already says
///   "saved!";
/// - a successful `Cross` is dropped because the header it sets up arrives as
///   its own `Shot` beat one line later;
/// - a long shot that misses or is blocked is dropped because a speculative
///   effort from 25 yards is not a chance. The engine produces a lot of them —
///   they were half the reel before this clause — and every one is still
///   counted in the shot totals `stats` reports, so nothing is hidden, only
///   un-narrated. A long shot *saved* or *scored* stays: those are moments.
fn is_highlight(kind: &MatchEventKind) -> bool {
    match kind {
        MatchEventKind::Shot {
            kind: ShotKind::LongShot,
            outcome: ShotOutcome::Off | ShotOutcome::Blocked,
            ..
        } => false,
        MatchEventKind::Shot { .. }
        | MatchEventKind::Foul { card: Some(_) }
        | MatchEventKind::Injury { .. }
        | MatchEventKind::Substitution { .. } => true,
        _ => false,
    }
}

/// How heavily a beat lands — the colour axis, resolved in one place.
fn beat_sem(kind: &MatchEventKind) -> Sem {
    match kind {
        MatchEventKind::Shot {
            outcome: ShotOutcome::Goal,
            ..
        } => Sem::Emphasis,
        MatchEventKind::Foul {
            card: Some(Card::Red | Card::SecondYellow),
        }
        | MatchEventKind::Injury { days_out: 1.. } => Sem::Bad,
        MatchEventKind::Foul {
            card: Some(Card::Yellow),
        }
        | MatchEventKind::Injury { days_out: 0 } => Sem::Warn,
        _ => Sem::Ok,
    }
}

/// The stream's commentary, name-resolved. The `World` lookup lives here —
/// this crate owns the `World` and is the only one allowed to touch stdout, so
/// `commentary` itself stays name-resolved and I/O-free (`MATCH_MODEL.md` §9).
///
/// Two names per beat, not one: `MatchEvent::other_player` names the fouling
/// defender, the departing substitute, or the contesting opponent, and a card
/// with nobody's name on it is no use to a manager.
pub fn commentary_lines(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
) -> Vec<String> {
    outcome
        .stream
        .iter()
        .map(|event| one_line(world, home_name, away_name, event))
        .collect()
}

fn one_line(world: &World, home_name: &str, away_name: &str, event: &MatchEvent) -> String {
    let side_name = match event.side {
        Side::Home => home_name,
        Side::Away => away_name,
    };
    let actor = world.player(event.actor).name.as_str();
    let other = event.other_player().map(|p| world.player(p).name.as_str());
    event.commentary(side_name, actor, other)
}

/// The highlight reel: kick-off, the beats that matter, the interval, and the
/// final whistle — in order, with the scoreline carried in a gutter so it is
/// never more than one line away.
pub fn highlights(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
) -> Vec<Beat> {
    let mut reel = Vec::new();
    reel.push(Beat {
        minute: 0,
        text: format!("  KICK-OFF   {home_name} vs {away_name}"),
        sem: Sem::Emphasis,
    });

    let (mut hg, mut ag) = (0u8, 0u8);
    let mut interval_done = false;
    for event in &outcome.stream {
        if !interval_done && event.minute >= HALF_TIME {
            interval_done = true;
            reel.push(interval_beat(outcome, home_name, away_name, hg, ag));
        }
        if !is_highlight(&event.kind) {
            continue;
        }
        let scored = matches!(
            event.kind,
            MatchEventKind::Shot {
                outcome: ShotOutcome::Goal,
                ..
            }
        );
        if scored {
            match event.side {
                Side::Home => hg += 1,
                Side::Away => ag += 1,
            }
        }
        let line = one_line(world, home_name, away_name, event);
        let mut text = format!(" {hg}-{ag}   {line}");
        if scored {
            // The one beat that earns extra ink: the score it just changed,
            // spelled out under it, so the reader never has to reconstruct the
            // gutter's arithmetic mid-celebration.
            let _ = write!(
                text,
                "\n       {}\n        {home_name} {hg} - {ag} {away_name}\n       {}",
                "─".repeat(44),
                "─".repeat(44)
            );
        }
        reel.push(Beat {
            minute: event.minute,
            text,
            sem: beat_sem(&event.kind),
        });
    }
    if !interval_done {
        reel.push(interval_beat(outcome, home_name, away_name, hg, ag));
    }
    reel.push(Beat {
        minute: 90,
        text: format!(
            "  ═══ FULL TIME ═══   {home_name} {} - {} {away_name}",
            outcome.home_goals, outcome.away_goals
        ),
        sem: Sem::Emphasis,
    });
    reel
}

fn interval_beat(
    outcome: &match_engine::MatchOutcome,
    home_name: &str,
    away_name: &str,
    hg: u8,
    ag: u8,
) -> Beat {
    let first_half = stats(outcome, |m| m < HALF_TIME);
    Beat {
        minute: HALF_TIME,
        text: format!(
            "  ─── HALF-TIME ───   {home_name} {hg} - {ag} {away_name}\n{}",
            first_half.one_line()
        ),
        sem: Sem::Emphasis,
    }
}

/// One side's countable readings over some window of the match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SideStats {
    /// On-ball beats — the possession denominator.
    pub touches: u32,
    pub shots: u32,
    pub on_target: u32,
    /// Fouls this side *committed*. The stream's `Foul` beat belongs to the
    /// fouled side (`MATCH_MODEL.md` §15), so this is counted from the other
    /// side's beats — the one place in here where the reading is not simply
    /// "count my own events".
    pub fouls: u32,
    pub yellows: u32,
    pub reds: u32,
}

/// Both sides' readings, plus the arithmetic that only makes sense across the
/// pair (possession share).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchStats {
    pub home: SideStats,
    pub away: SideStats,
}

impl MatchStats {
    /// Home possession as a whole percentage. A match with no touches at all
    /// (an empty window) reads 50 rather than dividing by zero.
    pub fn home_possession(&self) -> u32 {
        let total = self.home.touches + self.away.touches;
        if total == 0 {
            return 50;
        }
        (self.home.touches * 100).div_ceil(total).min(100)
    }

    /// The compact reading the half-time break carries.
    pub fn one_line(&self) -> String {
        let hp = self.home_possession();
        format!(
            "        Possession {hp}% – {}%    Shots {} ({}) – {} ({})",
            100 - hp,
            self.home.shots,
            self.home.on_target,
            self.away.shots,
            self.away.on_target
        )
    }
}

/// Count the stream over the minutes `window` accepts.
pub fn stats(outcome: &match_engine::MatchOutcome, window: impl Fn(u8) -> bool) -> MatchStats {
    let mut st = MatchStats::default();
    for e in outcome.stream.iter().filter(|e| window(e.minute)) {
        let (mine, theirs) = match e.side {
            Side::Home => (&mut st.home, &mut st.away),
            Side::Away => (&mut st.away, &mut st.home),
        };
        match e.kind {
            MatchEventKind::Pass { .. }
            | MatchEventKind::TakeOn { .. }
            | MatchEventKind::Cross { .. } => mine.touches += 1,
            MatchEventKind::Shot { outcome: o, .. } => {
                mine.touches += 1;
                mine.shots += 1;
                if matches!(o, ShotOutcome::Goal | ShotOutcome::Saved) {
                    mine.on_target += 1;
                }
            }
            // `side` is the fouled side; the offence belongs to the other one.
            MatchEventKind::Foul { card } => {
                theirs.fouls += 1;
                match card {
                    Some(Card::Yellow) => theirs.yellows += 1,
                    Some(Card::SecondYellow) | Some(Card::Red) => theirs.reds += 1,
                    None => {}
                }
            }
            _ => {}
        }
    }
    st
}

/// The full-time reading, laid out as the two-column sheet every football
/// broadcast ends on. No colour: this block has no axis — every row is a raw
/// count with no direction to act on, which is the same call
/// `screens::stats` makes.
pub fn stats_block(home_name: &str, away_name: &str, st: &MatchStats, p: Palette) -> String {
    let hp = st.home_possession();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(&row("", home_name, away_name), Sem::Emphasis)
    );
    let mut line = |label: &str, home: String, away: String| {
        let _ = writeln!(out, "{}", row(label, &home, &away));
    };
    line("Possession", format!("{hp}%"), format!("{}%", 100 - hp));
    line(
        "Shots",
        st.home.shots.to_string(),
        st.away.shots.to_string(),
    );
    line(
        "On target",
        st.home.on_target.to_string(),
        st.away.on_target.to_string(),
    );
    line(
        "Fouls",
        st.home.fouls.to_string(),
        st.away.fouls.to_string(),
    );
    if st.home.yellows + st.away.yellows > 0 {
        line(
            "Yellow cards",
            st.home.yellows.to_string(),
            st.away.yellows.to_string(),
        );
    }
    if st.home.reds + st.away.reds > 0 {
        line(
            "Red cards",
            st.home.reds.to_string(),
            st.away.reds.to_string(),
        );
    }
    out
}

/// One `home | label | away` line of the full-time sheet. Home right-aligned
/// against the centre, the label centred in a fixed gutter, away left-aligned
/// away from it — so the pair reads as two columns leaning on the label
/// between them. Laid out whole and trimmed, so no row carries trailing
/// padding into a snapshot.
fn row(label: &str, home: &str, away: &str) -> String {
    const HOME_W: usize = 22;
    const LABEL_W: usize = 16;
    let spaces = |n: usize| " ".repeat(n);
    let l = label.chars().count().min(LABEL_W);
    let left = (LABEL_W - l) / 2;
    format!(
        "  {}{home}   {}{label}{}   {away}",
        spaces(HOME_W.saturating_sub(home.chars().count())),
        spaces(left),
        spaces(LABEL_W - l - left),
    )
    .trim_end()
    .to_string()
}

/// What the match left behind: cards, injuries, and the man of the match.
///
/// The stream tells you these happened as they happened; a manager still wants
/// them collected in one place at full time, because that is the list he has
/// to pick a team around next week.
pub fn aftermath(world: &World, outcome: &match_engine::MatchOutcome, p: Palette) -> String {
    let mut out = String::new();
    if !outcome.cards.is_empty() {
        let _ = writeln!(out, "\nCards:");
        // Chronological — the order they were shown.
        let mut cards: Vec<&CardOutcome> = outcome.cards.iter().collect();
        cards.sort_by_key(|c| (c.minute, c.player));
        for c in cards {
            let (word, sem) = match c.card {
                Card::Yellow => ("booked", Sem::Warn),
                Card::SecondYellow => ("sent off (second yellow)", Sem::Bad),
                Card::Red => ("sent off (straight red)", Sem::Bad),
            };
            let _ = writeln!(
                out,
                "{}",
                p.paint(
                    &format!("  {}' {} — {word}.", c.minute, world.player(c.player).name),
                    sem
                )
            );
        }
    }
    if !outcome.injuries.is_empty() {
        let _ = writeln!(out, "\nInjuries:");
        let mut injuries: Vec<&InjuryOutcome> = outcome.injuries.iter().collect();
        injuries.sort_by_key(|i| i.player);
        for i in injuries {
            // A zero-day knock is not an absence, so it does not read as one —
            // and it does not read as an alarm either.
            let (text, sem) = if i.days_out == 0 {
                (
                    format!(
                        "  {} — a knock, no games missed.",
                        world.player(i.player).name
                    ),
                    Sem::Warn,
                )
            } else {
                (
                    format!(
                        "  {} — out for {} day(s).",
                        world.player(i.player).name,
                        i.days_out
                    ),
                    Sem::Bad,
                )
            };
            let _ = writeln!(out, "{}", p.paint(&text, sem));
        }
    }
    if let Some((pid, rating)) = match_engine::man_of_the_match(&outcome.ratings) {
        let _ = writeln!(
            out,
            "\n{}",
            p.paint(
                &format!(
                    "Man of the match: {} ({:.1}).",
                    world.player(pid).name,
                    rating as f64 / 10.0
                ),
                Sem::Emphasis
            )
        );
    }
    out
}

// ---------------------------------------------------------------------------
// The printing half: pacing, the mode prompt, raw mode.
// ---------------------------------------------------------------------------

/// Shows a match. Shared by the standalone friendly viewer and, for the
/// human's own fixture, the main game loop's matchday advance.
///
/// The mode is only *asked for* when there is a real terminal on both ends. A
/// piped run gets the full commentary unpaced and unprompted — the same output
/// it got before there were modes at all, which keeps redirects, replays and
/// scripted runs stable.
pub fn present_match(
    world: &World,
    home_name: &str,
    away_name: &str,
    outcome: &match_engine::MatchOutcome,
    p: Palette,
) {
    let tty = io::stdin().is_terminal() && io::stdout().is_terminal();
    let mode = if tty {
        ask_mode(home_name, away_name, p)
    } else {
        ViewMode::Full
    };

    match mode {
        ViewMode::Highlights => play(
            highlights(world, home_name, away_name, outcome)
                .into_iter()
                .map(|b| (b.minute, p.paint(&b.text, b.sem)))
                .collect(),
            tty,
            Pace::ByMinute,
        ),
        ViewMode::Full => {
            println!(
                "\n{home_name} vs {away_name} — {} raw events, unfiltered (the humble text match view, DESIGN.md §9):\n",
                outcome.stream.len()
            );
            play(
                commentary_lines(world, home_name, away_name, outcome)
                    .into_iter()
                    .map(|line| (0, line))
                    .collect(),
                tty,
                Pace::Fixed,
            );
            println!(
                "\n{}",
                p.paint(
                    &format!(
                        "  ═══ FULL TIME ═══   {home_name} {} - {} {away_name}",
                        outcome.home_goals, outcome.away_goals
                    ),
                    Sem::Emphasis
                )
            );
        }
        ViewMode::Result => println!(
            "\n{}",
            p.paint(
                &format!(
                    "  ═══ FULL TIME ═══   {home_name} {} - {} {away_name}",
                    outcome.home_goals, outcome.away_goals
                ),
                Sem::Emphasis
            )
        ),
    }

    print!(
        "{}",
        stats_block(home_name, away_name, &stats(outcome, |_| true), p)
    );
    print!("{}", aftermath(world, outcome, p));
}

fn ask_mode(home_name: &str, away_name: &str, p: Palette) -> ViewMode {
    println!(
        "\n{}",
        p.paint(&format!("  {home_name} vs {away_name}"), Sem::Emphasis)
    );
    println!("  [enter] Watch highlights   [f] Full commentary   [s] Straight to full time");
    match prompt_choice("  > ", &["", "f", "s"]).as_str() {
        "f" => ViewMode::Full,
        "s" => ViewMode::Result,
        _ => ViewMode::Highlights,
    }
}

/// How long to hold between beats.
#[derive(Debug, Clone, Copy)]
enum Pace {
    /// One fixed delay per line — the full commentary, where every beat is one
    /// tick of the same clock.
    Fixed,
    /// Proportional to the match minutes that passed since the last beat, so a
    /// scramble reads as a scramble and a quiet spell as a quiet spell.
    ByMinute,
}

/// Prints `lines` (each already coloured), pacing them and stopping the pacing
/// the moment a key is pressed. Falls back to printing everything at once when
/// there is no terminal to pace for.
fn play(lines: Vec<(u8, String)>, tty: bool, pace: Pace) {
    if !tty {
        for (_, line) in lines {
            println!("{line}");
        }
        return;
    }
    println!("  (press any key to skip ahead)\n");
    let interactive = crossterm::terminal::enable_raw_mode().is_ok();
    let mut skipping = false;
    let mut last_minute = 0u8;
    for (minute, line) in lines {
        let delay = match pace {
            Pace::Fixed => EVENT_DELAY,
            Pace::ByMinute => {
                let gap = minute.saturating_sub(last_minute) as u64;
                Duration::from_millis(gap * PACE_PER_MINUTE).clamp(PACE_MIN, PACE_MAX)
            }
        };
        last_minute = minute;
        if interactive {
            // Raw mode turns off the terminal's own \n -> \r\n translation.
            for physical in line.split('\n') {
                print!("{physical}\r\n");
            }
            io::stdout().flush().ok();
        } else {
            println!("{line}");
        }
        if interactive && !skipping && key_pressed_within(delay) {
            skipping = true;
        }
    }
    if interactive {
        let _ = crossterm::terminal::disable_raw_mode();
    }
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
                if matches!(
                    crossterm::event::read(),
                    Ok(crossterm::event::Event::Key(_))
                ) {
                    return true;
                }
                // Some other event (resize, focus change, ...) — keep waiting.
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fforge_core::{Session, WorldGenConfig, new_game, player_match_preview};
    use fforge_domain::ClubId;

    /// The same fixed seed the screen snapshots use, so a failure here and a
    /// snapshot diff there are talking about the same match.
    const SEED: u64 = 0xF00D_BEEF;

    /// The human's opening fixture, resolved. `player_match_preview` runs the
    /// real engine off the real state, so this is a genuine stream and not a
    /// hand-built one — the counting rules below are worth nothing if the
    /// thing they count is a fake.
    fn opening_match() -> (fforge_domain::World, match_engine::MatchOutcome) {
        let log = new_game(SEED, &WorldGenConfig::default(), ClubId(0));
        let session = Session::from_events(log, &mut []);
        let outcome = player_match_preview(&session.state).expect("an opening fixture");
        (session.state.world.clone(), outcome)
    }

    #[test]
    fn the_reel_is_a_subset_of_the_stream_and_keeps_its_order() {
        let (world, outcome) = opening_match();
        let reel = highlights(&world, "Home", "Away", &outcome);
        assert!(
            reel.len() < outcome.stream.len(),
            "a reel that keeps every beat is not a reel"
        );
        let mut last = 0u8;
        for beat in &reel {
            assert!(
                beat.minute >= last,
                "the reel ran backwards at {}'",
                beat.minute
            );
            last = beat.minute;
        }
    }

    #[test]
    fn every_goal_survives_the_filter() {
        // The one thing the reel may never drop. Counted from the stream, so
        // this stays true whatever else `is_highlight` learns to exclude.
        let (world, outcome) = opening_match();
        let goals = outcome
            .stream
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    MatchEventKind::Shot {
                        outcome: ShotOutcome::Goal,
                        ..
                    }
                )
            })
            .count();
        let narrated = highlights(&world, "Home", "Away", &outcome)
            .iter()
            .filter(|b| b.text.contains("GOAL!"))
            .count();
        assert_eq!(narrated, goals);
        assert_eq!(
            goals as u8,
            outcome.home_goals + outcome.away_goals,
            "the stream's goals and the recorded score disagree"
        );
    }

    #[test]
    fn the_running_gutter_ends_on_the_recorded_score() {
        // The gutter is recomputed from the stream rather than read off the
        // outcome, so it is free to drift from it. This is the test that says
        // it doesn't.
        let (world, outcome) = opening_match();
        let reel = highlights(&world, "Home", "Away", &outcome);
        let last_gutter = reel
            .iter()
            .rev()
            .find_map(|b| b.text.split_whitespace().next().filter(|t| t.contains('-')))
            .expect("at least one gutter line");
        assert_eq!(
            last_gutter,
            format!("{}-{}", outcome.home_goals, outcome.away_goals)
        );
    }

    #[test]
    fn possession_is_a_share_of_one_hundred() {
        let (_, outcome) = opening_match();
        let st = stats(&outcome, |_| true);
        assert!((1..=99).contains(&st.home_possession()), "{st:?}");
        // The empty window must not divide by zero.
        assert_eq!(stats(&outcome, |_| false).home_possession(), 50);
    }

    #[test]
    fn fouls_are_counted_against_the_offender_not_the_fouled_side() {
        // `MATCH_MODEL.md` §15: a `Foul` beat's `side` is the *fouled* side.
        // Getting this backwards would silently swap both teams' discipline
        // columns, and nothing else in the game would notice.
        let (_, outcome) = opening_match();
        let st = stats(&outcome, |_| true);
        let fouled_home = outcome
            .stream
            .iter()
            .filter(|e| matches!(e.kind, MatchEventKind::Foul { .. }) && e.side == Side::Home)
            .count() as u32;
        assert_eq!(st.away.fouls, fouled_home);
    }

    #[test]
    fn the_halves_partition_the_match() {
        let (_, outcome) = opening_match();
        let first = stats(&outcome, |m| m < HALF_TIME);
        let second = stats(&outcome, |m| m >= HALF_TIME);
        let whole = stats(&outcome, |_| true);
        assert_eq!(first.home.shots + second.home.shots, whole.home.shots);
        assert_eq!(first.away.touches + second.away.touches, whole.away.touches);
    }

    #[test]
    fn a_long_shot_that_misses_is_the_only_shot_the_reel_drops() {
        let off_long = MatchEventKind::Shot {
            kind: ShotKind::LongShot,
            source: match_engine::ShotSource::Long,
            outcome: ShotOutcome::Off,
        };
        let saved_long = MatchEventKind::Shot {
            kind: ShotKind::LongShot,
            source: match_engine::ShotSource::Long,
            outcome: ShotOutcome::Saved,
        };
        let off_header = MatchEventKind::Shot {
            kind: ShotKind::Header,
            source: match_engine::ShotSource::Cross,
            outcome: ShotOutcome::Off,
        };
        assert!(!is_highlight(&off_long));
        assert!(is_highlight(&saved_long));
        assert!(is_highlight(&off_header));
        assert!(!is_highlight(&MatchEventKind::Pass { success: true }));
        assert!(!is_highlight(&MatchEventKind::Foul { card: None }));
        assert!(is_highlight(&MatchEventKind::Foul {
            card: Some(Card::Yellow)
        }));
    }
}
