//! **The one and only place `Sem` becomes a colour** (R15).
//!
//! Screens ask for `Sem::Warn`; nothing outside this module names a colour, a
//! `crossterm::style` type, or an ANSI code. Raw colour calls scattered across
//! screens are how a codebase ends up with green meaning "healthy" on one
//! screen and "selected" on another.
//!
//! **Colour is redundant, never load-bearing.** Every coloured distinction a
//! screen draws must also be carried by a glyph, a column, or an ordering. Two
//! reasons, and the second is the one people forget: red/green is the obvious
//! encoding for good/bad and the worst available choice for the ~8% of men
//! with red–green colour deficiency; and `NO_COLOR`, piped output, and CI logs
//! strip colour entirely. If colour is redundant, all three cases degrade to
//! merely plainer rather than to information loss — which is exactly what
//! `screens::tests::no_ansi_escapes_when_colour_is_disabled` pins.
//!
//! **No global state.** The resolved policy is a [`Palette`] value threaded
//! from `main` into the screens, not an ambient `static`. That is the same
//! instinct the core applies to RNG and the clock: an impure source is
//! resolved once at the edge and passed in as data. It also means the snapshot
//! tests can render both ways in the same process without racing.

use crossterm::style::{Attribute, Attributes, Color, ContentStyle};
use std::io::IsTerminal;

/// The semantic vocabulary. **One axis per screen** — a screen states the axis
/// its colour encodes in a comment at the top, and uses only the shades of
/// that one axis. A player list where colour means ability on one screen and
/// contract status on another is worse than no colour at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sem {
    /// The positive end of the screen's axis (comfortable, in form, affordable).
    Good,
    /// Unremarkable — deliberately renders plain, so a screen's default state
    /// costs no ink.
    Ok,
    /// The negative end of the screen's axis: worth looking at, not yet an
    /// alarm (tight headroom, contract running down, doubtful fitness).
    Warn,
    /// A genuine alarm — the budget is breached, the player is suspended, the
    /// role is uncovered. Red is reserved for exactly this.
    Bad,
    /// Present but not the point: absent values, parenthetical detail.
    Muted,
    /// Structurally important regardless of value — a header, a total.
    Emphasis,
    /// "This one is yours." The only semantic the league table uses, because
    /// the table's meaning is already positional.
    Mine,
}

impl Sem {
    /// The style this semantic renders as. Deliberately private-by-convention:
    /// callers go through [`Palette::paint`], which is what respects the
    /// no-colour policy.
    ///
    /// **Blue/orange, not red/green** (R15). The diverging pair survives the
    /// common colour deficiencies and reads on both light and dark terminals;
    /// red is held back for [`Sem::Bad`], where a glyph carries it anyway. The
    /// 256-colour indices are used rather than the 16 named colours because
    /// the named ones are re-mapped by every terminal theme, and a palette
    /// that shifts per-theme is a palette that cannot be reasoned about.
    fn style(self) -> ContentStyle {
        let mut s = ContentStyle::new();
        match self {
            // 33 = #0087ff. Legible on white and on black.
            Sem::Good => s.foreground_color = Some(Color::AnsiValue(33)),
            Sem::Ok => {}
            // 208 = #ff8700, the orange half of the diverging pair.
            Sem::Warn => s.foreground_color = Some(Color::AnsiValue(208)),
            // 160 = #d70000. Alarms only.
            Sem::Bad => s.foreground_color = Some(Color::AnsiValue(160)),
            // 244 = #808080, mid grey — recedes on both backgrounds.
            Sem::Muted => s.foreground_color = Some(Color::AnsiValue(244)),
            Sem::Emphasis => s.attributes = Attributes::from(Attribute::Bold),
            // Shares Good's hue, bold. Safe *because* of the one-axis-per-screen
            // rule: a screen whose axis is `Mine` never also draws `Good`, so
            // the two can never appear together and be confused.
            Sem::Mine => {
                s.foreground_color = Some(Color::AnsiValue(33));
                s.attributes = Attributes::from(Attribute::Bold);
            }
        }
        s
    }
}

/// The resolved colour policy, threaded from `main` into the screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    enabled: bool,
}

impl Palette {
    /// Colour off. The state under `NO_COLOR`, `--no-color`, or a piped
    /// stdout — and the state every snapshot test renders in. Production
    /// always goes through [`Palette::from_environment`], so these two
    /// constructors are reached only from tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const PLAIN: Palette = Palette { enabled: false };

    /// Colour on. See [`Palette::PLAIN`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub const COLOURED: Palette = Palette { enabled: true };

    /// Resolve the policy from the process environment — the one impure
    /// function in this module, called once, at the edge, by `main`.
    pub fn from_environment(args: &[String]) -> Palette {
        // R15 reads `NO_COLOR` as "any value", which is stricter than
        // no-color.org's "present and non-empty" and never wrong in the
        // direction that matters: an empty `NO_COLOR` still means no colour.
        let no_color_env = std::env::var_os("NO_COLOR").is_some();
        let no_color_flag = args.iter().any(|a| a == "--no-color");
        // The escape hatch for `| less -R` and friends, where stdout is a pipe
        // but the eventual consumer does understand escapes.
        let force = args.iter().any(|a| a == "--color");
        Palette {
            enabled: resolve(
                no_color_env,
                no_color_flag,
                force,
                std::io::stdout().is_terminal(),
            ),
        }
    }

    /// Apply `sem` to already-laid-out text.
    ///
    /// **Pad before you paint.** An escape sequence has zero visual width but
    /// several bytes of it, so `format!("{:<20}", palette.paint(..))` pads to
    /// the wrong place. `render::table` exists so no screen has to remember
    /// that; if you are calling `paint` outside a table helper, pad first.
    pub fn paint(self, text: &str, sem: Sem) -> String {
        if !self.enabled || sem == Sem::Ok {
            return text.to_string();
        }
        sem.style().apply(text).to_string()
    }
}

/// The colour decision, factored out of the environment read so it can be
/// tested without a terminal.
fn resolve(no_color_env: bool, no_color_flag: bool, force: bool, is_tty: bool) -> bool {
    if no_color_env || no_color_flag {
        // An explicit "off" always wins, `--color` included: the point of
        // `NO_COLOR` is that the user does not have to audit every tool's
        // flags.
        return false;
    }
    force || is_tty
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Sem; 7] = [
        Sem::Good,
        Sem::Ok,
        Sem::Warn,
        Sem::Bad,
        Sem::Muted,
        Sem::Emphasis,
        Sem::Mine,
    ];

    #[test]
    fn the_plain_palette_is_the_identity() {
        for sem in ALL {
            assert_eq!(Palette::PLAIN.paint("Kofi Barbieri", sem), "Kofi Barbieri");
        }
    }

    #[test]
    fn ok_costs_no_ink_even_with_colour_on() {
        assert_eq!(
            Palette::COLOURED.paint("nothing to see", Sem::Ok),
            "nothing to see"
        );
    }

    #[test]
    fn every_other_semantic_is_distinguishable_with_colour_on() {
        let painted: Vec<String> = ALL
            .iter()
            .filter(|s| **s != Sem::Ok)
            .map(|&s| Palette::COLOURED.paint("x", s))
            .collect();
        for p in &painted {
            assert!(p.contains('\u{1b}'), "expected an escape in {p:?}");
            assert!(p.contains('x'), "expected the content to survive in {p:?}");
        }
        for (i, a) in painted.iter().enumerate() {
            for b in painted.iter().skip(i + 1) {
                assert_ne!(a, b, "two semantics render identically");
            }
        }
    }

    #[test]
    fn an_explicit_off_beats_everything_including_a_tty() {
        assert!(!resolve(true, false, false, true));
        assert!(!resolve(false, true, false, true));
        // ...and beats an explicit --color, which is the whole point of NO_COLOR.
        assert!(!resolve(true, false, true, true));
    }

    #[test]
    fn a_pipe_gets_no_colour_unless_forced() {
        assert!(!resolve(false, false, false, false));
        assert!(resolve(false, false, true, false));
    }

    #[test]
    fn a_terminal_gets_colour_by_default() {
        assert!(resolve(false, false, false, true));
    }
}
