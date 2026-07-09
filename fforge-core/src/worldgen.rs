//! Seeded world generation. Lives at the **edge**: it runs once at new-game
//! time and its *output* (the `World`) is recorded into `GameStarted` — the
//! fold never re-derives it, so worldgen can evolve freely without breaking
//! saves (the record-resolved-values principle).

use crate::rng::{derive_stream, Rng};
use crate::schedule::double_round_robin;
use fforge_domain::{
    best_role, Attribute, Attributes, Character, Club, ClubId, Competition, CompetitionId,
    Fixture, GameDate, Player, PlayerId, Role, Staff, StaffId, StaffRole, World, NUM_ATTRIBUTES,
    ROLE_WEIGHTS,
};
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
                let player = gen_player(&mut rng, id, role, quality, start_date);
                squad.push(id);
                players.insert(id, player);
            }
        }

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

        clubs.insert(
            club_id,
            Club {
                id: club_id,
                name,
                players: squad,
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

fn gen_player(rng: &mut Rng, id: PlayerId, role: Role, club_quality: f64, today: GameDate) -> Player {
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

    Player {
        id,
        name: person_name(rng),
        birth,
        natural_role: role,
        attributes,
        character,
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