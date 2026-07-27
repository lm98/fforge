//! The club's money: cash, the wage ceiling, what is committed against it, and
//! the monthly `FinanceTick` trend.
//!
//! **Colour axis: headroom** (R15) — comfortable / tight / breached. Both
//! headrooms matter and they mean different things: cash is a pot you spend
//! from, `wage_budget` is a *ceiling you must stay under*, never a second pot
//! (`TRANSFER_MODEL.md` §3). A club can be flush and still unable to sign
//! anyone.
//!
//! Red appears here and it is earned: a negative balance or a breached wage
//! ceiling is one of R15's genuine alarms. The `left` column carries the same
//! fact as a signed number, so a `NO_COLOR` run loses nothing.
//!
//! The trend is read straight off the event log — `Event::FinanceTick` records
//! resolved per-club deltas, so nothing here re-derives revenue or the wage
//! bill. `finance::finance_deltas` is the only place that arithmetic lives.

use crate::render::money;
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Align, Cell, Col, Table, pad};
use fforge_core::{Event, Session, UtilityKnobs};
use fforge_domain::Money;
use std::fmt::Write as _;

/// How many monthly ticks the trend shows. Six is two development periods'
/// worth of context — long enough to see a direction, short enough to fit.
const TREND_MONTHS: usize = 6;

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let fin = club.finances;
    let knobs = UtilityKnobs::default();

    let committed = committed_wages(session);
    let wage_room = fin.wage_budget.0 - committed.0;
    let spendable = fin.balance.0 - knobs.cash_reserve_floor.0;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "\n{}",
        p.paint(
            &format!("=== Finances — {} · {} ===", club.name, s.date),
            Sem::Emphasis
        )
    );

    let mut t = Table::new(vec![
        Col::left("", 24),
        Col::right("", 10),
        Col::left("", 0),
    ])
    .indent("  ")
    .headerless();
    t.row(vec![
        Cell::new("Cash balance"),
        Cell::new(money(fin.balance)).with(balance_sem(fin.balance)),
        Cell::new(""),
    ]);
    t.row(vec![
        Cell::new("  less reserve floor"),
        Cell::new(money(knobs.cash_reserve_floor)).with(Sem::Muted),
        Cell::new("kept unspent by the market's affordability gate").with(Sem::Muted),
    ]);
    t.row(vec![
        Cell::new("  spendable on fees"),
        Cell::new(money(Money(spendable))).with(headroom_sem(spendable, fin.balance.0.max(1))),
        Cell::new(""),
    ]);
    t.row(vec![Cell::new(""), Cell::new(""), Cell::new("")]);
    t.row(vec![
        Cell::new("Wage ceiling"),
        Cell::new(money(fin.wage_budget)),
        Cell::new("a constraint, not a second pot").with(Sem::Muted),
    ]);
    t.row(vec![
        Cell::new("  committed wages"),
        Cell::new(money(committed)),
        Cell::new(format!("{} player(s) under contract", contracted(session))).with(Sem::Muted),
    ]);
    t.row(vec![
        Cell::new("  headroom left"),
        Cell::new(money(Money(wage_room))).with(headroom_sem(wage_room, fin.wage_budget.0.max(1))),
        Cell::new(""),
    ]);
    out.push_str(&t.render(p));

    out.push_str(&trend(session, p));
    out
}

/// The last few `FinanceTick`s for the human's club, oldest first — the "am I
/// running down or building up" reading a single balance can never give.
fn trend(session: &Session, p: Palette) -> String {
    let mine = session.state.player_club;
    let ticks: Vec<(String, Money)> = session
        .log
        .iter()
        .filter_map(|e| match e {
            Event::FinanceTick { date, deltas } => deltas
                .iter()
                .find(|(club, _)| *club == mine)
                .map(|(_, delta)| (date.to_string(), *delta)),
            _ => None,
        })
        .collect();

    let mut out = String::new();
    if ticks.is_empty() {
        let _ = writeln!(
            out,
            "\n{}",
            p.paint(
                "  No monthly finance tick has fired yet this season.",
                Sem::Muted
            )
        );
        return out;
    }

    let shown = &ticks[ticks.len().saturating_sub(TREND_MONTHS)..];
    let peak = shown.iter().map(|(_, d)| d.0.abs()).max().unwrap_or(1);
    let _ = writeln!(out, "\n  Monthly trend (revenue minus the wage bill):");
    let mut t = Table::new(vec![
        Col::left("Tick", 16),
        Col::right("Net", 10),
        Col::left("", 0),
    ])
    .indent("  ");
    for (date, delta) in shown {
        t.row(vec![
            Cell::new(date.clone()),
            Cell::new(money(*delta)).with(if delta.0 >= 0 { Sem::Good } else { Sem::Warn }),
            // The bar is the non-colour carrier: direction and rough magnitude
            // are both visible with every escape stripped.
            Cell::new(bar(*delta, peak)).with(if delta.0 >= 0 { Sem::Good } else { Sem::Warn }),
        ]);
    }
    out.push_str(&t.render(p));

    let total: i64 = shown.iter().map(|(_, d)| d.0).sum();
    let _ = writeln!(
        out,
        "  {}",
        p.paint(
            &format!(
                "over these {} tick(s): {}",
                shown.len(),
                money(Money(total))
            ),
            if total >= 0 { Sem::Good } else { Sem::Warn }
        )
    );
    out
}

/// A signed bar around a `|` zero axis, scaled to the largest tick on screen.
/// ASCII `+`/`-` rather than a block glyph so it renders identically in every
/// terminal and in a piped log.
///
/// This is the trend's non-colour carrier: direction and rough magnitude both
/// survive `NO_COLOR` intact.
fn bar(delta: Money, peak: i64) -> String {
    const HALF: usize = 16;
    let width = ((delta.0.abs() as f64 / peak.max(1) as f64) * HALF as f64).round() as usize;
    let glyph = if delta.0 >= 0 { '+' } else { '-' };
    let run: String = std::iter::repeat_n(glyph, width.max(1)).collect();
    if delta.0 >= 0 {
        format!("{}|{run}", pad("", HALF, Align::Left))
    } else {
        format!("{}|", pad(&run, HALF, Align::Right))
    }
}

/// Σ of the squad's annual wages — the same sum `club_ai::observe` computes,
/// recomputed here rather than plumbed through, because this screen wants it
/// live rather than at the last window's freeze.
fn committed_wages(session: &Session) -> Money {
    let s = &session.state;
    Money(
        s.world
            .club_players(s.player_club)
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.wage.0)
            .sum(),
    )
}

fn contracted(session: &Session) -> usize {
    let s = &session.state;
    s.world
        .club_players(s.player_club)
        .filter(|p| p.contract.is_some())
        .count()
}

fn balance_sem(balance: Money) -> Sem {
    if balance.0 < 0 { Sem::Bad } else { Sem::Ok }
}

/// Headroom as a share of the ceiling it sits under: comfortable above a
/// fifth, tight below, breached once it goes negative.
fn headroom_sem(room: i64, of: i64) -> Sem {
    if room < 0 {
        Sem::Bad
    } else if (room as f64) < 0.2 * of as f64 {
        Sem::Warn
    } else {
        Sem::Good
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headroom_is_comfortable_tight_then_breached() {
        assert_eq!(headroom_sem(50, 100), Sem::Good);
        assert_eq!(headroom_sem(10, 100), Sem::Warn);
        assert_eq!(headroom_sem(-1, 100), Sem::Bad);
    }

    #[test]
    fn a_negative_balance_is_an_alarm() {
        assert_eq!(balance_sem(Money(-1)), Sem::Bad);
        assert_eq!(balance_sem(Money(0)), Sem::Ok);
    }
}
