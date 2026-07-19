//! Seeded world generation. Lives at the **edge**: it runs once at new-game
//! time and its *output* (the `World`) is recorded into `GameStarted` — the
//! fold never re-derives it, so worldgen can evolve freely without breaking
//! saves (the record-resolved-values principle).

use crate::development::{resolve_coaching, resolve_dev_profile, DevKnobs};
use crate::rng::{derive_stream, Rng};
use crate::schedule::double_round_robin;
use fforge_domain::{
    best_role, Attribute, Attributes, Character, Club, ClubId, Competition, CompetitionId,
    Contract, Finances, Fixture, GameDate, Money, Player, PlayerId, Role, Staff, StaffId,
    StaffRole, World, NUM_ATTRIBUTES, ROLE_WEIGHTS,
};
use fforge_domain::date::DAYS_PER_YEAR;
use std::collections::BTreeMap;

pub struct WorldGenConfig {
    pub num_clubs: usize,
    pub start_year: i32,
    pub league_name: String,
}

impl Default for WorldGenConfig {
    fn default() -> Self {
        WorldGenConfig {
            num_clubs: 20,
            start_year: 2026,
            league_name: "Prima Divisione".to_string(),
        }
    }
}

const WORLDGEN_STREAM: u64 = 0x574F_524C_4447_454E; // "WORLDGEN"

/// Squad template: role → headcount. 24 players per club.
const SQUAD_TEMPLATE: [(Role, usize); 8] = [
    (Role::Gk, 3),
    (Role::Cb, 4),
    (Role::Fb, 4),
    (Role::Dm, 2),
    (Role::Cm, 3),
    (Role::Am, 2),
    (Role::W, 3),
    (Role::St, 3),
];

pub fn generate(seed: u64, cfg: &WorldGenConfig) -> (World, Vec<Fixture>, GameDate) {
    assert!(cfg.num_clubs >= 2 && cfg.num_clubs.is_multiple_of(2));
    let mut rng = derive_stream(seed, WORLDGEN_STREAM);
    let dev_knobs = DevKnobs::default();
    let start_date = GameDate::from_year_day(cfg.start_year, 220); // late-summer kickoff

    // Club quality anchors, evenly spread then shuffled: a league with a
    // clear top, middle, and relegation-fodder bottom.
    let mut qualities: Vec<f64> = (0..cfg.num_clubs)
        .map(|i| 48.0 + 26.0 * i as f64 / (cfg.num_clubs - 1) as f64)
        .collect();
    rng.shuffle(&mut qualities);

    let mut players = BTreeMap::new();
    let mut clubs = BTreeMap::new();
    let mut staff = BTreeMap::new();
    let mut club_ids = Vec::with_capacity(cfg.num_clubs);
    let mut next_player = 0u32;
    let mut used_club_names: Vec<String> = Vec::new();

    for (ci, &quality) in qualities.iter().enumerate() {
        let club_id = ClubId(ci as u16);
        club_ids.push(club_id);
        let name = unique_club_name(&mut rng, &mut used_club_names);

        let mut squad = Vec::new();
        for &(role, count) in &SQUAD_TEMPLATE {
            for _ in 0..count {
                let id = PlayerId(next_player);
                next_player += 1;
                let player = gen_player(&mut rng, id, role, quality, start_date, &dev_knobs);
                squad.push(id);
                players.insert(id, player);
            }
        }
        let coaching_milli = resolve_coaching(&mut rng, quality, &dev_knobs);

        let manager_id = StaffId(ci as u32);
        staff.insert(
            manager_id,
            Staff {
                id: manager_id,
                name: person_name(&mut rng),
                role: StaffRole::Manager,
                club: Some(club_id),
            },
        );

        // Reputation and finances ride in the World snapshot (TRANSFER_MODEL.md
        // §3.1) — resolved from the same quality anchor the squad was, so a
        // top club is reputable *and* rich, no new event or migration. The
        // wage budget is set above the just-resolved wage bill so the league
        // starts solvent (wage bill ≤ budget is a hard §3.1 invariant), and
        // both money figures scale with reputation.
        let reputation = resolve_reputation(&mut rng, quality);
        let wage_bill: i64 = squad
            .iter()
            .filter_map(|pid| players[pid].contract.as_ref())
            .map(|c| c.wage.0)
            .sum();
        let finances = resolve_finances(&mut rng, reputation, wage_bill);

        clubs.insert(
            club_id,
            Club {
                id: club_id,
                name,
                players: squad,
                coaching_milli,
                finances,
                reputation,
            },
        );
    }

    let competition = Competition {
        id: CompetitionId(0),
        name: cfg.league_name.clone(),
        clubs: club_ids.clone(),
    };
    let schedule = double_round_robin(&club_ids);

    (
        World {
            players,
            clubs,
            staff,
            competition,
        },
        schedule,
        start_date,
    )
}

fn gen_player(
    rng: &mut Rng,
    id: PlayerId,
    role: Role,
    club_quality: f64,
    today: GameDate,
    dev_knobs: &DevKnobs,
) -> Player {
    // Age ~ triangular 16..=36, centered mid-20s.
    let age = 16 + ((rng.f64() + rng.f64()) * 10.0) as i32;
    let birth = today
        .add_days(-(age as i64) * fforge_domain::date::DAYS_PER_YEAR)
        .add_days(-(rng.below(365) as i64));

    // Player quality center around the club's anchor; youth discounted a bit
    // (their headroom lives in PA instead).
    let youth_discount = if age < 21 { (21 - age) as f64 * 2.0 } else { 0.0 };
    let base = rng.normal(club_quality - youth_discount, 5.0).clamp(28.0, 92.0);

    // Shape attributes by the role-weighting table: weight 5 ⇒ well above the
    // player's base, weight 1 ⇒ well below, weight 0 ⇒ untrained floor.
    let mut values = [0u8; NUM_ATTRIBUTES];
    for attr in Attribute::ALL {
        let w = ROLE_WEIGHTS.weight(role, attr);
        let v = if w == 0 {
            rng.range_i32(3, 18) as f64
        } else {
            rng.normal(base + (w as f64 - 3.0) * 4.5, 4.5)
        };
        values[attr.index()] = v.clamp(1.0, 96.0) as u8;
    }
    let attributes = Attributes::new(values);

    // PA: young players get real headroom; veterans are what they are.
    let (_, best_ca) = best_role(&attributes, &ROLE_WEIGHTS);
    let headroom = if age < 24 {
        (24 - age) * 2 + rng.range_i32(0, 8)
    } else {
        rng.range_i32(0, 3)
    };
    let potential = (best_ca as i32 + headroom).clamp(best_ca as i32, 97) as u8;

    let character = Character {
        potential,
        determination: rng.range_i32(20, 95) as u8,
        professionalism: rng.range_i32(20, 95) as u8,
        consistency: rng.range_i32(25, 90) as u8,
        injury_proneness: rng.range_i32(5, 85) as u8,
        leadership: rng.range_i32(10, 90) as u8,
    };

    // Once-resolved development trajectory (DEVELOPMENT_MODEL.md §2.3), derived
    // from character + seeded noise and recorded in the World snapshot.
    let development =
        resolve_dev_profile(rng, character.determination, character.professionalism, dev_knobs);

    // Employment terms (TRANSFER_MODEL.md §3.1): every t=0 player is contracted
    // (no free agents at kickoff). Wage scales with quality; the expiry spreads
    // 1–5 years out, correlated with youth and quality so the first window has
    // natural expiry pressure rather than a uniform cliff.
    let contract = Some(resolve_contract(rng, age, best_ca, today));

    Player {
        id,
        name: person_name(rng),
        birth,
        natural_role: role,
        attributes,
        character,
        development,
        contract,
        retired: false,
    }
}

/// Resolve one player's opening contract (`TRANSFER_MODEL.md` §3.1). Wage is a
/// convex function of quality — stars cost far more than squad fillers — and
/// the length runs 1–5 years, longer for young, good players so expiries do
/// not all fall on the same day. `expires` gets a random day-of-year on top of
/// the whole-year length so the window has a spread, not a cliff.
fn resolve_contract(rng: &mut Rng, age: i32, best_ca: u8, today: GameDate) -> Contract {
    let wage = wage_for_quality(rng, best_ca);

    // Length in whole years, 1..=5. Young players and good players earn the
    // longer deals; old or weak players get held on short terms.
    let youth_bonus = ((24 - age).clamp(-4, 8)) as f64 / 4.0; // +2.0 (teen) .. -1.0 (veteran)
    let quality_bonus = (best_ca as f64 - 58.0) / 12.0; // ~ +3 (elite) .. -4 (poor)
    let length = (3.0 + youth_bonus + quality_bonus + rng.normal(0.0, 0.6))
        .round()
        .clamp(1.0, 5.0) as i64;

    let expires = today
        .add_days(length * DAYS_PER_YEAR)
        .add_days(rng.below(DAYS_PER_YEAR as u32) as i64);
    Contract { wage, expires }
}

/// Annual wage from a player's headline CA — convex, so a CA-88 star earns a
/// large multiple of a CA-40 reserve, seeding a league near its own wage
/// equilibrium (`TRANSFER_MODEL.md` §3.1). Whole currency units.
fn wage_for_quality(rng: &mut Rng, best_ca: u8) -> Money {
    let ca = best_ca as f64;
    let base = 20_000.0 + 3_000_000.0 * (ca / 90.0).powi(3);
    let jitter = rng.normal(1.0, 0.12).clamp(0.6, 1.5);
    Money((base * jitter).round() as i64)
}

/// Club reputation (0–100) from the quality anchor (`TRANSFER_MODEL.md` §3.1).
/// Monotone in quality with light noise, so reputation tracks the anchor (the
/// correlation the worldgen test asserts) without being a mechanical copy.
fn resolve_reputation(rng: &mut Rng, quality: f64) -> u8 {
    rng.normal(quality, 3.0).round().clamp(1.0, 99.0) as u8
}

/// Club finances from reputation and the resolved wage bill
/// (`TRANSFER_MODEL.md` §3.1). Both figures scale with reputation; the wage
/// budget is additionally floored above the committed wage bill, so the league
/// starts solvent — committed wages ≤ `wage_budget` is a hard §3.1 invariant.
fn resolve_finances(rng: &mut Rng, reputation: u8, wage_bill: i64) -> Finances {
    let rep = reputation as f64;

    // Budget headroom over the current wage bill: 15–40%, so there is room to
    // sign but not a bottomless pot. Guarantees wage_bill ≤ wage_budget.
    let headroom = rng.normal(1.28, 0.06).clamp(1.15, 1.40);
    let wage_budget = ((wage_bill as f64) * headroom).round() as i64;

    // Cash on hand scales convexly with reputation (rich get richer), with a
    // floor so even a minnow opens with something in the bank.
    let balance =
        (500_000.0 + 40_000.0 * rep * (rep / 50.0)) * rng.normal(1.0, 0.1).clamp(0.7, 1.3);

    Finances {
        balance: Money(balance.round() as i64),
        wage_budget: Money(wage_budget),
    }
}

// ---------- name generation (flavor only; all seeded) ----------

const FIRST_NAMES: &[&str] = &[
    "Luca", "Marco", "Andrea", "Matteo", "Davide", "Giorgio", "Paolo", "Sandro", "Enzo", "Dario",
    "Bruno", "Franco", "Nicola", "Sergio", "Tommaso", "Aldo", "Pietro", "Emil", "Jonas", "Karl",
    "Sven", "Anders", "Milan", "Ivan", "Josip", "Luka", "Marko", "Petar", "Diego", "Rafael",
    "Thiago", "Bruno", "João", "Pedro", "Nuno", "Rui", "Sami", "Kofi", "Yaya", "Ousmane",
    "Ibrahim", "Amadou", "Kenji", "Hiro", "Sota", "Owen", "Harry", "Callum",
];

const LAST_NAMES: &[&str] = &[
    "Rossi", "Bianchi", "Ferrari", "Colombo", "Ricci", "Marino", "Greco", "Conti", "Gallo",
    "Fontana", "Moretti", "Barbieri", "Santoro", "Rinaldi", "Vitale", "Longo", "Serra", "Farina",
    "Berg", "Lund", "Dahl", "Novak", "Horvat", "Kovac", "Babic", "Silva", "Santos", "Costa",
    "Pereira", "Almeida", "Carvalho", "Mensah", "Diallo", "Traoré", "Keita", "Tanaka", "Sato",
    "Mori", "Ward", "Hughes", "Walsh", "Kane", "Duarte", "Vega", "Morales", "Iglesias", "Reyes",
    "Ortega",
];

fn person_name(rng: &mut Rng) -> String {
    let first = FIRST_NAMES[rng.below(FIRST_NAMES.len() as u32) as usize];
    let last = LAST_NAMES[rng.below(LAST_NAMES.len() as u32) as usize];
    format!("{first} {last}")
}

const CLUB_PREFIXES: &[&str] = &[
    "AC", "FC", "US", "Real", "Sporting", "Atlético", "Union", "Inter", "Olimpia", "Racing",
    "Dinamo", "Virtus",
];

const CITY_STEMS: &[&str] = &[
    "Vald", "Mont", "Torr", "Fior", "Cast", "Port", "Riv", "Sald", "Ver", "Bell", "Camp", "Sor",
    "Lav", "Ner", "Pral", "Ost",
];

const CITY_ENDS: &[&str] = &[
    "emona", "averde", "isola", "entino", "ora", "ana", "etto", "iano", "aro", "onte", "urnia",
    "essa",
];

fn unique_club_name(rng: &mut Rng, used: &mut Vec<String>) -> String {
    loop {
        let prefix = CLUB_PREFIXES[rng.below(CLUB_PREFIXES.len() as u32) as usize];
        let city = format!(
            "{}{}",
            CITY_STEMS[rng.below(CITY_STEMS.len() as u32) as usize],
            CITY_ENDS[rng.below(CITY_ENDS.len() as u32) as usize]
        );
        let name = format!("{prefix} {city}");
        if !used.contains(&name) {
            used.push(name.clone());
            return name;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fforge_domain::best_role;

    /// Mean headline CA of a club's squad — a proxy for the (internal,
    /// unshuffled-then-shuffled) quality anchor the club was generated from.
    fn squad_mean_ca(world: &World, club: ClubId) -> f64 {
        let squad = &world.club(club).players;
        let sum: f64 = squad
            .iter()
            .map(|&pid| best_role(&world.player(pid).attributes, &ROLE_WEIGHTS).1 as f64)
            .sum();
        sum / squad.len() as f64
    }

    #[test]
    fn every_worldgen_player_has_a_contract() {
        // TRANSFER_MODEL.md §3.1: no free agents at kickoff — every t=0 player
        // is under contract, so the first transfer window has real terms to
        // reason about rather than a pool of unattached players.
        let (world, _s, _d) = generate(11, &WorldGenConfig::default());
        for player in world.players.values() {
            assert!(
                player.contract.is_some(),
                "player {} was generated without a contract",
                player.id
            );
        }
    }

    #[test]
    fn club_of_agrees_with_the_players_index_for_every_worldgen_player() {
        // The reverse index (TRANSFER_MODEL.md §3) must agree with the forward
        // one on a real generated world, not just a hand-built one.
        let (world, _s, _d) = generate(3, &WorldGenConfig::default());
        for (&cid, club) in &world.clubs {
            for &pid in &club.players {
                assert_eq!(world.club_of(pid), Some(cid));
            }
        }
    }

    #[test]
    fn wage_bills_start_below_wage_budgets() {
        // The hard §3.1 solvency invariant: committed annual wages (Σ over the
        // squad's contracts) ≤ wage_budget for every club, so the league opens
        // inside its own wage structure rather than already over-committed.
        let (world, _s, _d) = generate(5, &WorldGenConfig::default());
        for club in world.clubs.values() {
            let wage_bill: i64 = club
                .players
                .iter()
                .filter_map(|pid| world.player(*pid).contract.as_ref())
                .map(|c| c.wage.0)
                .sum();
            assert!(
                wage_bill <= club.finances.wage_budget.0,
                "{}: wage bill {} exceeds wage budget {}",
                club.name,
                wage_bill,
                club.finances.wage_budget.0
            );
        }
    }

    #[test]
    fn reputation_correlates_with_squad_quality() {
        // TRANSFER_MODEL.md §3.1: reputation is resolved from the quality
        // anchor, so it must track squad quality — a top squad is a reputable
        // club. Checked as a Pearson correlation over the full 20-club league,
        // with a wide band (the resolve adds light noise on purpose).
        let (world, _s, _d) = generate(9, &WorldGenConfig::default());
        let points: Vec<(f64, f64)> = world
            .clubs
            .values()
            .map(|c| (c.reputation as f64, squad_mean_ca(&world, c.id)))
            .collect();
        let n = points.len() as f64;
        let mean_x = points.iter().map(|p| p.0).sum::<f64>() / n;
        let mean_y = points.iter().map(|p| p.1).sum::<f64>() / n;
        let mut cov = 0.0;
        let mut var_x = 0.0;
        let mut var_y = 0.0;
        for (x, y) in &points {
            cov += (x - mean_x) * (y - mean_y);
            var_x += (x - mean_x).powi(2);
            var_y += (y - mean_y).powi(2);
        }
        let r = cov / (var_x.sqrt() * var_y.sqrt());
        assert!(
            r > 0.5,
            "reputation should correlate with squad quality; Pearson r = {r:.3}"
        );
    }

    #[test]
    fn world_round_trips_through_json_with_the_new_fields() {
        // The whole snapshot — including Player.contract and Club.finances /
        // reputation — survives a JSON round-trip exactly (float-free domain,
        // §3). This is what GameStarted records; a lossy field here would
        // silently corrupt saves.
        let (world, _s, _d) = generate(42, &WorldGenConfig::default());
        let json = serde_json::to_string(&world).unwrap();
        let back: World = serde_json::from_str(&json).unwrap();
        assert_eq!(back, world);
    }
}