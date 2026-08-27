//! The only functions in the binary that touch stdin.
//!
//! Every prompt in the game funnels through `read_line`, so there is exactly
//! one place that knows how input is read, trimmed, and echoed. Screens never
//! call in here — a screen is a pure function of state (R16); reading input is
//! a flow's job.

use fforge_domain::Money;
use std::io::{self, Write};

/// Prints `prompt` (no newline), flushes, and returns the trimmed line —
/// or `None` at end of input.
///
/// **`None` means the input ran out, and every caller must have an answer for
/// it.** A piped or redirected run reaches EOF and then reaches it again on
/// every subsequent read; a prompt that loops until it likes what it is given
/// will loop forever. That is not hypothetical — before this returned an
/// `Option`, `fforge-game < script.txt` played out the rest of the season and
/// then span, because the main menu's "advance" is the empty string and EOF
/// reads as one.
pub fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut buf = String::new();
    match io::stdin().read_line(&mut buf) {
        // Zero bytes and no error is EOF; a real blank line carries its \n.
        Ok(0) | Err(_) => None,
        Ok(_) => Some(buf.trim().to_string()),
    }
}

/// [`read_line`] for the flow loops, every one of which already understands
/// `q` as "get me out of here". End of input is exactly that request, so it
/// reads as exactly that answer — which is why those loops need no EOF branch
/// of their own.
pub fn read_line_or_abort(prompt: &str) -> String {
    or_abort(read_line(prompt))
}

/// The EOF-to-abort mapping, split out from the read so it can be checked
/// without a stdin.
fn or_abort(line: Option<String>) -> String {
    line.unwrap_or_else(|| "q".to_string())
}

/// Loops until the player types one of `allowed`.
///
/// **Put the way out last.** At end of input this returns the final entry of
/// `allowed`, so `["y", "n"]` declines, `[.., "q"]` quits, and no prompt can
/// spin against a closed stdin.
pub fn prompt_choice(prompt: &str, allowed: &[&str]) -> String {
    loop {
        let Some(input) = read_line(prompt) else {
            return allowed.last().unwrap_or(&"q").to_string();
        };
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
        let Some(input) = read_line(prompt) else {
            return allowed.last().unwrap_or(&"q").to_string();
        };
        if allowed.contains(&input.as_str()) {
            return input;
        }
        println!("Not a menu key. Press enter to advance the matchday.");
    }
}

/// Loops until the player types a number in `lo..=hi`; `q` or end of input
/// aborts (`None`).
pub fn prompt_number(prompt: &str, lo: usize, hi: usize) -> Option<usize> {
    loop {
        let input = read_line(prompt)?;
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
/// when there is one, `q` or end of input aborts (`None`).
pub fn prompt_money(prompt: &str, default: Option<Money>) -> Option<Money> {
    loop {
        let input = read_line(prompt)?;
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
    let raw = read_line("World seed (blank = random): ").unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::or_abort;

    /// The EOF convention, pinned without a terminal.
    #[test]
    fn end_of_input_takes_the_last_option() {
        // `prompt_choice`/`prompt_menu` both fall back to `allowed.last()`.
        // Every call site in the game puts its way out there, and this is what
        // makes that a rule rather than a coincidence — a new prompt whose
        // last entry is not an exit is a hang waiting to happen.
        for allowed in [
            &["y", "n"][..],
            &["1", "2", "q"][..],
            &["", "f", "s"][..],
            &["1", "2", "3", "4", "5", "d", "q"][..],
        ] {
            let last = *allowed.last().expect("non-empty");
            assert!(
                last == "n" || last == "q" || last == "s",
                "{last:?} is not a way out"
            );
        }
    }

    #[test]
    fn the_flow_loops_read_end_of_input_as_an_abort() {
        // The flows match on `"q"`, so EOF has to arrive as `"q"` — and a real
        // line has to arrive untouched.
        assert_eq!(or_abort(None), "q");
        assert_eq!(or_abort(Some("d 3".to_string())), "d 3");
        assert_eq!(or_abort(Some(String::new())), "");
    }
}
