//! The only functions in the binary that touch stdin.
//!
//! Every prompt in the game funnels through `read_line`, so there is exactly
//! one place that knows how input is read, trimmed, and echoed. Screens never
//! call in here — a screen is a pure function of state (R16); reading input is
//! a flow's job.

use fforge_domain::Money;
use std::io::{self, Write};

/// Prints `prompt` (no newline), flushes, and returns the trimmed line.
/// A closed/erroring stdin reads as an empty line rather than panicking, so a
/// piped run terminates instead of looping forever.
pub fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut buf = String::new();
    if io::stdin().read_line(&mut buf).is_err() {
        return String::new();
    }
    buf.trim().to_string()
}

/// Loops until the player types one of `allowed`.
pub fn prompt_choice(prompt: &str, allowed: &[&str]) -> String {
    loop {
        let input = read_line(prompt);
        if allowed.contains(&input.as_str()) {
            return input;
        }
        println!("Options: {}", allowed.join(", "));
    }
}

/// Like [`prompt_choice`], but for the main menu, where a bare `enter` (the
/// empty string) is a real choice and echoing the whole option list back is
/// noise — the menu is already on screen right above the prompt.
pub fn prompt_menu(prompt: &str, allowed: &[&str]) -> String {
    loop {
        let input = read_line(prompt);
        if allowed.contains(&input.as_str()) {
            return input;
        }
        println!("Not a menu key. Press enter to advance the matchday.");
    }
}

/// Loops until the player types a number in `lo..=hi`; `q` aborts (`None`).
pub fn prompt_number(prompt: &str, lo: usize, hi: usize) -> Option<usize> {
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

/// Loops until the player types a non-negative amount; blank takes `default`
/// when there is one, `q` aborts (`None`).
pub fn prompt_money(prompt: &str, default: Option<Money>) -> Option<Money> {
    loop {
        let input = read_line(prompt);
        let trimmed = input.trim();
        if trimmed == "q" {
            return None;
        }
        if trimmed.is_empty() {
            if let Some(d) = default {
                return Some(d);
            }
            println!("Enter an amount (or 'q' to cancel).");
            continue;
        }
        match trimmed.parse::<i64>() {
            Ok(n) if n >= 0 => return Some(Money(n)),
            _ => println!("Enter a non-negative whole number (or 'q' to cancel)."),
        }
    }
}

/// The new-game seed prompt. Blank falls back to the wall clock — one of this
/// crate's two sanctioned clock reads, and safe because the chosen seed is
/// immediately recorded in `Event::GameStarted`, so replay never re-derives it.
pub fn prompt_seed() -> u64 {
    let raw = read_line("World seed (blank = random): ");
    if raw.trim().is_empty() {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xF00D)
    } else {
        raw.trim().parse().unwrap_or_else(|_| {
            // Non-numeric seeds are hashed FNV-style so "juventus" works too.
            raw.trim().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
            })
        })
    }
}
