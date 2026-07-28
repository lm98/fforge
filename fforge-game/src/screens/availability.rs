//! Who can play on Saturday, and who is carrying what.
//!
//! **Colour axis: availability** (R15) — fit / doubtful / injured / suspended.
//! Red is earned twice here: an injured player and a suspended one are both
//! genuinely unavailable, which is exactly the alarm R15 reserves it for.
//!
//! Carriers other than colour: the `Status` column says the same thing in
//! words, and the list is *ordered* unavailable-first, so the players you
//! cannot pick are the ones you read before anything else.
//!
//! **The ban rule is not re-derived here.** `GameState::is_suspended` is the
//! authority (`MATCH_MODEL.md` §12's derived-suspension rule: a ban is never
//! stored, it is computed fresh from `season_cards` every call). This screen
//! asks it, and separately shows the raw card tally as a fact. It deliberately
//! does *not* predict "one more yellow and he's banned" — that would be a
//! second copy of the rule, free to drift from the one that matters.

use crate::render::headline_ca;
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::{Card, Session};
use fforge_domain::PlayerId;
use std::fmt::Write as _;

/// Below this pre-match condition a player is *doubtful* — fit to pick, but
/// carrying accumulated load (`MATCH_MODEL.md` §13). Not a rule the engine
/// enforces; a reading the manager should have.
const DOUBTFUL_BELOW: f64 = 0.85;

pub fn render(session: &Session, p: Palette) -> String {
    let s = &session.state;
    let mut rows: Vec<Row> = s
        .world
        .club_players(s.player_club)
        .map(|player| {
            let status = status_of(session, player.id);
            Row {
                pos: player.natural_role.short().to_string(),
                name: player.name.clone(),
                ca: headline_ca(player),
                condition: s.condition(player.id),
                cards: card_tally(session, player.id),
                status,
            }
        })
        .collect();
    // Unavailable first, then doubtful, then everyone else — the ordering is
    // the non-colour carrier for the axis.
    rows.sort_by(|a, b| {
        a.status
            .rank()
            .cmp(&b.status.rank())
            .then(b.ca.cmp(&a.ca))
            .then(a.name.cmp(&b.name))
    });

    let mut t = Table::new(vec![
        Col::left("Pos", 3),
        Col::left("Name", 20),
        Col::right("CA", 3),
        Col::right("Cond", 5),
        Col::left("Cards", 6),
        Col::left("Status", 0),
    ]);
    for r in &rows {
        t.row_all(
            vec![
                Cell::new(r.pos.clone()),
                Cell::new(r.name.clone()),
                Cell::new(r.ca.to_string()),
                Cell::new(format!("{:.0}%", r.condition * 100.0)),
                Cell::new(r.cards.clone()),
                Cell::new(r.status.label()),
            ],
            r.status.sem(),
        );
    }

    let mut out = format!("\n{}", t.render(p));
    let available = rows.iter().filter(|r| r.status.is_pickable()).count();
    let line = format!(" {available} of {} available to pick.", rows.len());
    let _ = writeln!(
        out,
        "{}",
        // Eleven is the floor that matters: below it there is no legal XI.
        if available < fforge_domain::XI {
            p.paint(&format!("{line} !"), Sem::Bad)
        } else {
            line
        }
    );
    out
}

struct Row {
    pos: String,
    name: String,
    ca: u8,
    condition: f64,
    cards: String,
    status: Status,
}

/// The axis itself, as a value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// Days remaining on a recorded layoff.
    Injured(i64),
    Suspended,
    Doubtful(u8),
    Fit,
}

impl Status {
    /// Sort key — unavailable first.
    fn rank(&self) -> u8 {
        match self {
            Status::Injured(_) => 0,
            Status::Suspended => 1,
            Status::Doubtful(_) => 2,
            Status::Fit => 3,
        }
    }

    fn is_pickable(&self) -> bool {
        matches!(self, Status::Doubtful(_) | Status::Fit)
    }

    fn label(&self) -> String {
        match self {
            Status::Injured(days) => format!("Injured — back in {days} day(s)"),
            Status::Suspended => "Suspended for this matchday".to_string(),
            Status::Doubtful(pct) => format!("Doubtful — {pct}% condition"),
            Status::Fit => "Fit".to_string(),
        }
    }

    fn sem(&self) -> Sem {
        match self {
            Status::Injured(_) | Status::Suspended => Sem::Bad,
            Status::Doubtful(_) => Sem::Warn,
            Status::Fit => Sem::Ok,
        }
    }
}

fn status_of(session: &Session, pid: PlayerId) -> Status {
    let s = &session.state;
    // `injured_until` is the recorded layoff; the availability check itself is
    // `GameState::available`, which reads both this and the derived ban.
    if let Some(until) = s.world.player(pid).injured_until
        && until > s.date
    {
        return Status::Injured(until.days - s.date.days);
    }
    if s.is_suspended(pid) {
        return Status::Suspended;
    }
    let condition = s.condition(pid);
    if condition < DOUBTFUL_BELOW {
        return Status::Doubtful((condition * 100.0).round() as u8);
    }
    Status::Fit
}

/// This season's cards as a compact tally — recorded truth, no rule applied.
fn card_tally(session: &Session, pid: PlayerId) -> String {
    let Some(cards) = session.state.season_cards.get(&pid) else {
        return String::new();
    };
    let yellows = cards.iter().filter(|(_, c)| *c == Card::Yellow).count();
    let reds = cards
        .iter()
        .filter(|(_, c)| matches!(c, Card::Red | Card::SecondYellow))
        .count();
    match (yellows, reds) {
        (0, 0) => String::new(),
        (y, 0) => format!("{y}Y"),
        (0, r) => format!("{r}R"),
        (y, r) => format!("{y}Y {r}R"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_sorts_before_doubtful_before_fit() {
        let mut order = [
            Status::Fit,
            Status::Doubtful(70),
            Status::Suspended,
            Status::Injured(14),
        ];
        order.sort_by_key(|s| s.rank());
        assert_eq!(
            order,
            [
                Status::Injured(14),
                Status::Suspended,
                Status::Doubtful(70),
                Status::Fit
            ]
        );
    }

    #[test]
    fn only_doubtful_and_fit_are_pickable() {
        assert!(Status::Fit.is_pickable());
        assert!(Status::Doubtful(70).is_pickable());
        assert!(!Status::Injured(1).is_pickable());
        assert!(!Status::Suspended.is_pickable());
    }
}
