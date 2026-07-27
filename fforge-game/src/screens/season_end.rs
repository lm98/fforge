//! The end-of-season wrap: final table, champion, where you finished, and the
//! season's telemetry.

use crate::render::ordinal;
use crate::render::table_position;
use crate::screens::{stats, table};
use fforge_core::{SeasonTelemetry, Session};
use std::fmt::Write as _;

pub fn render(session: &Session, telemetry: &SeasonTelemetry) -> String {
    let s = &session.state;
    let mut out = String::new();
    let _ = writeln!(out, "\n================ SEASON OVER ================");
    out.push_str(&table::render(session));
    if let Some(champ) = s.champion {
        let _ = writeln!(out, "\nChampions: {}", s.world.club(champ).name);
    }
    let pos = table_position(session, s.player_club);
    let _ = writeln!(
        out,
        "You finished {} with {}.",
        ordinal(pos),
        s.world.club(s.player_club).name
    );
    out.push_str(&stats::render(telemetry));
    // The core can roll a season over (`Command::StartNextSeason`, and the
    // Phase-3 development fold rides on it); the CLI has not wired that up
    // yet, so a run still ends here.
    let _ = writeln!(
        out,
        "(This run ends here — season rollover isn't wired into the CLI yet.)"
    );
    out
}
