//! The title screen and the entry menu.
//!
//! It is the first thing the game says, and until now it said
//! `==========` twice around a name. A management sim lives or dies on the
//! player believing the world on the other side of the prompt is a real place,
//! and that belief starts here, before a single club has been picked.
//!
//! **Colour axis: availability of the choice** (R15) — an entry you cannot
//! take yet (loading with no save on disk) reads `Muted`, everything else
//! reads plain. The `—` and the parenthetical say the same thing in words, so
//! a `NO_COLOR` run loses nothing. The frame is `Emphasis`: structural, not a
//! value on the axis.

use crate::render::sem::{Palette, Sem};
use std::fmt::Write as _;

/// Inner width of the banner frame, in characters.
const W: usize = 54;

/// The banner. Wide enough to feel like a title, narrow enough for an
/// 80-column terminal with room to spare.
///
/// Every line is laid out through `framed`, which pads to a single constant
/// width — a hand-drawn box shears by one character the first time anyone
/// edits the wording, and a sheared title screen is the loudest possible way
/// for a game to look unfinished.
pub fn banner(p: Palette) -> String {
    let mut out = String::new();
    let rule = |ends: (char, char)| {
        p.paint(
            &format!("  {}{}{}", ends.0, "═".repeat(W), ends.1),
            Sem::Emphasis,
        )
    };
    let _ = writeln!(out, "\n{}", rule(('╔', '╗')));
    let _ = writeln!(out, "{}", framed("", Sem::Ok, p));
    let _ = writeln!(
        out,
        "{}",
        framed("    F O O T B A L L   F O R G E", Sem::Emphasis, p)
    );
    let _ = writeln!(
        out,
        "{}",
        framed("    a football management simulation", Sem::Muted, p)
    );
    let _ = writeln!(out, "{}", framed("", Sem::Ok, p));
    let _ = writeln!(out, "{}", rule(('╚', '╝')));
    out
}

/// One framed banner line. **Pad before you paint**: the content is padded to
/// `W` while it is still plain text, and only then does it and the two frame
/// characters get their colour.
fn framed(content: &str, sem: Sem, p: Palette) -> String {
    let pad = " ".repeat(W.saturating_sub(content.chars().count()));
    format!(
        "  {}{}{}{}",
        p.paint("║", Sem::Emphasis),
        p.paint(content, sem),
        pad,
        p.paint("║", Sem::Emphasis)
    )
}

/// The entry menu. `save_present` decides whether loading is offered as a real
/// choice or shown as unavailable — telling the player there is nothing to
/// load *before* they press the key, rather than after.
pub fn menu(save_present: bool, save_path: &str, p: Palette) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n  [1] New game");
    let _ = writeln!(
        out,
        "{}",
        if save_present {
            format!("  [2] Continue — {save_path}")
        } else {
            p.paint(
                &format!("  [2] Continue — no save at ./{save_path}"),
                Sem::Muted,
            )
        }
    );
    let _ = writeln!(out, "  [q] Quit");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_is_a_rectangle() {
        // A frame that shears by one character is the single most obvious way
        // for a title screen to look broken, and it is invisible in a diff.
        let rendered = banner(Palette::PLAIN);
        let lines: Vec<&str> = rendered.trim_matches('\n').lines().collect();
        let width = lines[0].chars().count();
        for line in &lines {
            assert_eq!(line.chars().count(), width, "ragged banner line: {line:?}");
        }
    }

    #[test]
    fn the_missing_save_says_so_in_words_not_only_in_colour() {
        let plain = menu(false, "savegame.fml", Palette::PLAIN);
        assert!(plain.contains("no save"), "{plain}");
        assert!(menu(true, "savegame.fml", Palette::PLAIN).contains("Continue — savegame.fml"));
    }
}
