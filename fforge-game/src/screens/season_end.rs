//! The end-of-season wrap: final table, champion, where you finished, and the
//! season's telemetry.
//!
//! **Colour axis: `Mine`**, inherited from the final table this screen embeds
//! (R15). The champion line and your own finishing line are the two facts worth
//! finding here, and both are about identity, not quality.

use crate::render::ordinal;
use crate::render::sem::{Palette, Sem};
use crate::render::table_position;
use crate::screens::{stats, table};
use fforge_core::{SeasonTelemetry, Session};
use std::fmt::Write as _;

pub fn render(session: &Session, telemetry: &SeasonTelemetry, p: Palette) -> String {
    let s = &session.state;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            "================ SEASON OVER ================",
            Sem::Emphasis
        )
    );
    out.push_str(&table::render(session, p));
    if let Some(champ) = s.champion {
        let champion_is_you = champ == s.player_club;
        let _ = writeln!(
            out,
            "\n{}",
            p.paint(
                &format!("Champions: {}", s.world.club(champ).name),
                if champion_is_you { Sem::Mine } else { Sem::Ok }
            )
        );
    }
    let pos = table_position(session, s.player_club);
    let _ = writeln!(
        out,
        "{}",
        p.paint(
            &format!(
                "You finished {} with {}.",
                ordinal(pos),
                s.world.club(s.player_club).name
            ),
            Sem::Mine
        )
    );
    out.push_str(&stats::render(telemetry));
    // The core can roll a season over (`Command::StartNextSeason`, and the
    // Phase-3 development fold rides on it); the CLI has not wired that up
    // yet, so a run still ends here.
    let _ = writeln!(
        out,
        "{}",
        p.paint(
            "(This run ends here — season rollover isn't wired into the CLI yet.)",
            Sem::Muted
        )
    );
    out
}
