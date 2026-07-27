//! League-wide telemetry — the calibration harness's readings, surfaced.

use fforge_core::SeasonTelemetry;
use std::fmt::Write as _;

pub fn render(telemetry: &SeasonTelemetry) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\nLeague-wide telemetry (the calibration harness embryo):"
    );
    let _ = writeln!(out, "  matches played : {}", telemetry.matches);
    let _ = writeln!(out, "  goals per match: {:.2}", telemetry.goals_per_match());
    if telemetry.matches > 0 {
        let _ = writeln!(
            out,
            "  home/draw/away : {:.0}% / {:.0}% / {:.0}%",
            100.0 * telemetry.home_wins as f64 / telemetry.matches as f64,
            100.0 * telemetry.draws as f64 / telemetry.matches as f64,
            100.0 * telemetry.away_wins as f64 / telemetry.matches as f64
        );
    }
    out
}
