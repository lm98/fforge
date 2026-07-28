//! The transfer-market menu (`TRANSFER_MODEL.md` §10's pre-commitment model):
//! build a local draft plan, browse targets and your own squad against a
//! frozen valuation snapshot, then submit the whole plan in one shot.
//!
//! **Colour axis: affordability against this club's cash and wage headroom**
//! (R15). Both halves matter and they fail independently — a bargain you cannot
//! pay the wages on is just as dead as one you cannot pay the fee on, and
//! `market::filter_affordable` drops either at resolve time without saying so.
//! That is why the axis is affordability rather than quality: quality is
//! already the sort order.
//!
//! The `Fit` column is the non-colour carrier and names the *blocking* half
//! (`ok`, `fee`, `wage`, `both`), which colour alone could not say anyway.

use crate::Observers;
use crate::input::{prompt_choice, prompt_money, prompt_number, read_line};
use crate::render::sem::{Palette, Sem};
use crate::render::table::{Cell, Col, Table};
use fforge_core::{
    ClubObservation, Command, DevKnobs, MarketContext, Session, TransferDecision, UtilityKnobs,
    ValueKnobs,
    club_ai::{Candidate, SquadMember},
    observe, value_all,
};
use fforge_domain::{Money, PlayerId, Role, World};
use std::collections::BTreeMap;

/// Everything the transfer screens read, computed once per visit to
/// `transfer_flow`: the frozen §2.7 valuation snapshot against the *current*
/// live session and the human club's own `ClubObservation` built from it —
/// the same knobs (`DevKnobs`/`ValueKnobs`/`UtilityKnobs::default()`)
/// `market::resolve_window` itself falls back to. Not live-updated while the
/// player browses; only rebuilt after a submit, matching how a real window
/// only re-prices once, at close.
pub struct TransferContext {
    pub obs: ClubObservation,
    pub valuations: BTreeMap<PlayerId, Money>,
    pub knobs: UtilityKnobs,
}

impl TransferContext {
    /// Cash above the reserve floor — what `market::filter_affordable` will
    /// actually let a bid spend, not the headline balance.
    fn spendable(&self) -> i64 {
        self.obs.balance.0 - self.knobs.cash_reserve_floor.0
    }

    fn wage_room(&self) -> i64 {
        self.obs.wage_budget.0 - self.obs.committed_wages.0
    }

    /// Which half of the affordability gate, if either, blocks this signing.
    fn afford(&self, fee: Money, wage: Money) -> Afford {
        Afford {
            fee_ok: fee.0 <= self.spendable(),
            wage_ok: wage.0 <= self.wage_room(),
        }
    }
}

/// The two independent halves of `market::filter_affordable`'s gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Afford {
    fee_ok: bool,
    wage_ok: bool,
}

impl Afford {
    /// The non-colour carrier: names the blocking half outright.
    fn label(self) -> &'static str {
        match (self.fee_ok, self.wage_ok) {
            (true, true) => "ok",
            (false, true) => "fee",
            (true, false) => "wage",
            (false, false) => "both",
        }
    }

    fn sem(self) -> Sem {
        match (self.fee_ok, self.wage_ok) {
            (true, true) => Sem::Good,
            // One half short is a plan away from possible: sell someone, or
            // bid under the ask.
            (false, true) | (true, false) => Sem::Warn,
            // Out of reach on both counts — recedes rather than alarms. A
            // player you cannot afford is not an emergency.
            (false, false) => Sem::Muted,
        }
    }
}

/// This wage as a share of the club's whole committed wage bill — the "where
/// is the money actually going" reading of the affordability axis, for the
/// screen that lists players you could sell.
///
/// No `Sem::Bad` here on purpose: an expensive player is not an alarm, and R15
/// keeps red for things that are (a breached budget, a suspension). The `Wage`
/// column carries the number itself, so colour is redundant by construction.
fn wage_burden_sem(wage: Money, committed: Money) -> Sem {
    if committed.0 <= 0 {
        return Sem::Ok;
    }
    let share = wage.0 as f64 / committed.0 as f64;
    // A 24-man squad averages ~4% each, so 15% is one player eating the space
    // of nearly four, and under 6% is unremarkable.
    if share >= 0.15 {
        Sem::Warn
    } else if share >= 0.06 {
        Sem::Ok
    } else {
        Sem::Muted
    }
}

pub fn build_transfer_context(session: &Session) -> TransferContext {
    let s = &session.state;
    let dev = DevKnobs::default();
    let vk = ValueKnobs::default();
    let uk = UtilityKnobs::default();
    let ctx = MarketContext::from_world(&s.world, &vk, &s.recent_ratings);
    let valuations = value_all(&s.world, s.date, &ctx, &vk, &dev);
    let obs = observe(&s.world, s.player_club, s.date, &valuations, &dev, &uk);
    TransferContext {
        obs,
        valuations,
        knobs: uk,
    }
}

/// Nothing is recorded until [4] Submit — browsing and editing the draft touch
/// no `Session` state.
pub fn transfer_flow(session: &mut Session, o: &mut Observers, p: Palette) {
    let mut ctx = build_transfer_context(session);
    let mut draft: Vec<TransferDecision> = session.state.pending_transfer_decisions.clone();
    loop {
        print_transfer_header(session, &ctx, &draft, p);
        println!("[1] Browse targets  [2] My squad  [3] Shortlist  [4] Submit  [0] Back");
        match prompt_choice("> ", &["1", "2", "3", "4", "0"]).as_str() {
            "1" => browse_targets_screen(&session.state.world, &ctx, &mut draft, p),
            "2" => squad_transfer_screen(&session.state.world, &ctx, &mut draft, p),
            "3" => shortlist_screen(&session.state.world, &mut draft),
            "4" => {
                submit_draft(session, o, &draft);
                ctx = build_transfer_context(session);
            }
            _ => return,
        }
    }
}

fn print_transfer_header(
    session: &Session,
    ctx: &TransferContext,
    draft: &[TransferDecision],
    p: Palette,
) {
    let s = &session.state;
    let club = s.world.club(s.player_club);
    let spendable = ctx.spendable();
    let wage_room = ctx.wage_room();
    println!("\n=== Transfer market — {} · {} ===", club.name, s.date);
    // The two headroom figures are the axis itself, so they carry its extreme:
    // a negative one means every row below is `both`-blocked until you sell.
    let headroom_sem = |room: i64| if room > 0 { Sem::Ok } else { Sem::Bad };
    println!(
        "  {}   {}",
        p.paint(
            &format!(
                "Cash {} (spendable {}, reserve floor {})",
                ctx.obs.balance,
                Money(spendable),
                ctx.knobs.cash_reserve_floor
            ),
            headroom_sem(spendable)
        ),
        p.paint(
            &format!(
                "Wage headroom {} (budget {} - committed {})",
                Money(wage_room),
                ctx.obs.wage_budget,
                ctx.obs.committed_wages
            ),
            headroom_sem(wage_room)
        )
    );
    let status = if *draft == s.pending_transfer_decisions {
        if draft.is_empty() {
            "nothing submitted"
        } else {
            "matches submitted plan"
        }
    } else {
        "unsubmitted changes — pick [4] to submit"
    };
    println!(
        "  Squad {}/{} (min {})   Draft: {} decision(s) — {status}",
        ctx.obs.squad.len(),
        ctx.knobs.squad_max,
        ctx.knobs.squad_min,
        draft.len()
    );
}

/// Browse candidate signings — every player not already on the human's own
/// books, priced against `ctx`'s frozen valuation snapshot (§2.7), filterable
/// by role. Picking one appends (or replaces) a `Bid` in `draft` at a
/// reservation price the player chooses.
fn browse_targets_screen(
    world: &World,
    ctx: &TransferContext,
    draft: &mut Vec<TransferDecision>,
    palette: Palette,
) {
    let mut role_filter: Option<Role> = None;
    loop {
        let mut candidates: Vec<&Candidate> = ctx
            .obs
            .candidates
            .iter()
            .filter(|c| role_filter.is_none_or(|r| c.role == r))
            .collect();
        candidates.sort_by(|a, b| b.value.0.cmp(&a.value.0).then(a.player.cmp(&b.player)));

        println!(
            "\nFilter: {}",
            role_filter.map(|r| r.name()).unwrap_or("All roles")
        );
        let shown: Vec<&Candidate> = candidates.iter().take(40).copied().collect();
        let mut t = Table::new(vec![
            Col::left("#", 3),
            Col::left("Name", 20),
            Col::left("Pos", 4),
            Col::left("Club", 20),
            Col::right("Value", 12),
            Col::right("Ask price", 12),
            Col::right("Wage", 10),
            Col::left("Fit", 4),
            Col::left("", 0),
        ]);
        for (i, c) in shown.iter().enumerate() {
            let p = world.player(c.player);
            let owner = match c.club {
                Some(cid) => world.club(cid).name.clone(),
                None => "Free agent".to_string(),
            };
            let shortlisted = draft
                .iter()
                .any(|d| matches!(d, TransferDecision::Bid { player, .. } if *player == c.player));
            let afford = ctx.afford(c.asking_price, c.wage);
            t.row_all(
                vec![
                    Cell::new((i + 1).to_string()),
                    Cell::new(p.name.clone()),
                    Cell::new(c.role.short()),
                    Cell::new(owner),
                    Cell::new(c.value.to_string()),
                    Cell::new(c.asking_price.to_string()),
                    Cell::new(c.wage.to_string()),
                    Cell::new(afford.label()),
                    Cell::new(if shortlisted { "(shortlisted)" } else { "" }),
                ],
                afford.sem(),
            );
        }
        print!("{}", t.render(palette));
        if candidates.len() > shown.len() {
            println!(
                "  ...and {} more; narrow with a role filter.",
                candidates.len() - shown.len()
            );
        }
        println!("  [#] bid on a listed player   [f] change role filter   [q] back");
        let input = read_line("> ");
        match input.trim() {
            "q" => return,
            "f" => role_filter = prompt_role_filter(),
            n => match n.parse::<usize>() {
                Ok(i) if (1..=shown.len()).contains(&i) => add_bid(world, shown[i - 1], draft),
                _ => println!("Pick a listed number, 'f', or 'q'."),
            },
        }
    }
}

fn prompt_role_filter() -> Option<Role> {
    println!("\nFilter by role:");
    println!("  [0] All roles");
    for (i, r) in Role::ALL.iter().enumerate() {
        println!("  [{}] {} ({})", i + 1, r.name(), r.short().trim());
    }
    match prompt_number("Role: ", 0, Role::ALL.len()) {
        Some(0) | None => None,
        Some(i) => Some(Role::ALL[i - 1]),
    }
}

fn add_bid(world: &World, c: &Candidate, draft: &mut Vec<TransferDecision>) {
    let p = world.player(c.player);
    println!(
        "\n{} ({}) — value {}, asking {}",
        p.name,
        c.role.short().trim(),
        c.value,
        c.asking_price
    );
    let Some(price) = prompt_money(
        "Reservation price (blank = asking price, q = cancel): ",
        Some(c.asking_price),
    ) else {
        return;
    };
    draft.retain(|d| !matches!(d, TransferDecision::Bid { player, .. } if *player == c.player));
    draft.push(TransferDecision::Bid {
        player: c.player,
        from: c.club,
        role: c.role,
        price,
    });
    println!("Added to shortlist (position {}).", draft.len());
}

/// The human's own squad, with contract expiry and wage next to every
/// player — the numbers resolve-time validation checks (`market::filter_affordable`'s
/// wage-headroom/squad-bounds gate). Picking one toggles a `List` decision
/// for that player in `draft`.
fn squad_transfer_screen(
    world: &World,
    ctx: &TransferContext,
    draft: &mut Vec<TransferDecision>,
    palette: Palette,
) {
    loop {
        let mut squad: Vec<&SquadMember> = ctx.obs.squad.iter().collect();
        squad.sort_by_key(|m| (m.natural_role, std::cmp::Reverse(m.current_ca)));

        let mut t = Table::new(vec![
            Col::left("#", 3),
            Col::left("Name", 20),
            Col::left("Pos", 4),
            Col::right("CA", 3),
            Col::right("Proj", 4),
            Col::right("Wage", 10),
            Col::right("Contract", 9),
            Col::right("Ask price", 12),
            Col::left("", 0),
        ]);
        for (i, m) in squad.iter().enumerate() {
            let p = world.player(m.player);
            let value = ctx.valuations.get(&m.player).copied().unwrap_or(Money(0));
            let ask = Money((value.0 as f64 * ctx.knobs.asking_markup).round() as i64);
            let listed = draft
                .iter()
                .any(|d| matches!(d, TransferDecision::List { player } if *player == m.player));
            t.row_all(
                vec![
                    Cell::new((i + 1).to_string()),
                    Cell::new(p.name.clone()),
                    Cell::new(m.natural_role.short()),
                    Cell::new(m.current_ca.to_string()),
                    Cell::new(m.projected_ca.to_string()),
                    Cell::new(m.wage.to_string()),
                    Cell::new(format!("{:.1}y", m.years_left_on_contract)),
                    Cell::new(ask.to_string()),
                    Cell::new(if listed { "(listed)" } else { "" }),
                ],
                // The same affordability axis, read from the selling side:
                // where the wage bill is actually going, and therefore which
                // sale would buy back the most headroom.
                wage_burden_sem(m.wage, ctx.obs.committed_wages),
            );
        }
        print!("{}", t.render(palette));
        println!("  [#] toggle list-for-sale   [q] back");
        let input = read_line("> ");
        match input.trim() {
            "q" => return,
            n => match n.parse::<usize>() {
                Ok(i) if (1..=squad.len()).contains(&i) => toggle_list(squad[i - 1].player, draft),
                _ => println!("Pick a listed number or 'q'."),
            },
        }
    }
}

fn toggle_list(player: PlayerId, draft: &mut Vec<TransferDecision>) {
    if let Some(pos) = draft
        .iter()
        .position(|d| matches!(d, TransferDecision::List { player: p } if *p == player))
    {
        draft.remove(pos);
        println!("Removed from sell list.");
    } else {
        draft.push(TransferDecision::List { player });
        println!("Added to sell list.");
    }
}

/// Review, reorder, and edit the draft before submitting. Order is priority:
/// `market::resolve_window` attempts the first still-biddable `Bid` in the
/// list each round, so moving an entry up raises its priority.
fn shortlist_screen(world: &World, draft: &mut Vec<TransferDecision>) {
    loop {
        if draft.is_empty() {
            println!("\nShortlist is empty.");
        } else {
            println!("\nShortlist (priority order — first affordable entry wins each round):");
            for (i, d) in draft.iter().enumerate() {
                println!("  {}. {}", i + 1, decision_summary(world, *d));
            }
        }
        println!("  [d N] drop entry N   [u N] move entry N up   [c] clear all   [q] back");
        let input = read_line("> ");
        match input.trim() {
            "q" => return,
            "c" => {
                draft.clear();
                println!("Shortlist cleared.");
            }
            other => {
                let mut parts = other.split_whitespace();
                let cmd = parts.next();
                let idx = parts.next().and_then(|n| n.parse::<usize>().ok());
                match (cmd, idx) {
                    (Some("d"), Some(i)) if (1..=draft.len()).contains(&i) => {
                        draft.remove(i - 1);
                    }
                    (Some("u"), Some(i)) if (2..=draft.len()).contains(&i) => {
                        draft.swap(i - 1, i - 2);
                    }
                    _ => println!("Commands: 'd N' drop, 'u N' move up, 'c' clear, 'q' back."),
                }
            }
        }
    }
}

fn decision_summary(world: &World, d: TransferDecision) -> String {
    match d {
        TransferDecision::Bid {
            player,
            price,
            role,
            from,
        } => {
            let p = world.player(player);
            let owner = match from {
                Some(cid) => world.club(cid).name.clone(),
                None => "a free agent".to_string(),
            };
            format!(
                "Bid {price} for {} ({}, from {owner})",
                p.name,
                role.short().trim()
            )
        }
        TransferDecision::List { player } => {
            format!("List {} for sale", world.player(player).name)
        }
    }
}

fn submit_draft(session: &mut Session, o: &mut Observers, draft: &[TransferDecision]) {
    match session.execute(
        Command::SubmitTransferDecision(draft.to_vec()),
        &mut o.all(),
    ) {
        Ok(_) => println!(
            "\nShortlist submitted: {} decision(s) pending for the next window close.",
            draft.len()
        ),
        Err(e) => println!("\nRejected: {e}"),
    }
}
