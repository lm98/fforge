//! `fforge-core::valuation` — the centralized, design-once player value
//! function (`TRANSFER_MODEL.md` §2).
//!
//! `DESIGN.md` §4.3 asks for one valuation function reused by the club AI's
//! buy/sell utility, the market's reservation prices, the Phase-5 agent
//! observations, and the management UI. This is that function. It **consumes**
//! two already-built, already-calibrated artifacts rather than rivalling them:
//! `ROLE_WEIGHTS` (via `current_ability`/`best_role`) and the development
//! envelope (via `crate::development`'s growth law). Its shape (§2.1):
//!
//! ```text
//! value = V0 · exp(β · (ca_eff − ca_ref)) · contract_mult · scarcity_mult
//! ca_eff = Σ_{t=0..H} δᵗ · project_ca(p, t) / Σ_{t=0..H} δᵗ
//! ```
//!
//! **Purity (the `play_match`/`tick_changes` bar).** `value` is a pure function
//! of `(world, player, today, ctx, knobs)` — no RNG, no wall clock, no I/O. It
//! is Layer 2: it prices, it does not decide. It knows nothing of clubs' buy/sell
//! logic (Layer 3) or the market (`TRANSFER_MODEL.md` §5–§6); those are consumers.
//!
//! **Pricing the career, not the day (§2.3).** `ca_eff` is a discounted mean of
//! *projected* future best-role CA, not today's CA. The projection runs the
//! `DEVELOPMENT_MODEL.md` §2 growth law forward with jitter off — reusing
//! `development::attr_rate` **directly**, so there is one law and no second,
//! coarser integrator to drift from the real monthly one. Age depreciation, PA
//! headroom, the plasticity window, and `DevCategory`-dependent aging (a
//! physically-reliant winger depreciating faster than a technical centre-back)
//! all fall out of that shared machinery for free — none of it is re-encoded here.
//!
//! **Hidden information is correct (§2.6).** The projection reads
//! `Character::potential` and `DevProfile`, both hidden. `value()` is *ground
//! truth* — the `info` channel; Phase-5 scouting fog is a wrapper that degrades
//! the inputs, not a change to this function.

use crate::development::{
    self, DevKnobs, NUM_CATEGORIES, attr_rate, category_peaks, norms_by_role, role_ceiling_consts,
};
use fforge_domain::{
    Attribute, Contract, FORMATIONS, GameDate, Money, NUM_ATTRIBUTES, NUM_ROLES, ROLE_WEIGHTS, Role,
    World, best_role,
};
use std::collections::BTreeMap;

const DAYS_PER_YEAR: f64 = fforge_domain::date::DAYS_PER_YEAR as f64;

/// The valuation knob table (`TRANSFER_MODEL.md` §9) — a **plausibility-picked
/// starting point**, documented as such in the explicit sense of
/// `MATCH_MODEL.md`'s `ELO_SCALE_S`: these are modelling choices, *not* fitted
/// results. The §11 market harness re-fits them (as `b_beat` and `env_phys`
/// were re-fit); `beta` and the not-yet-built revenue term are the two that
/// most need it. Sibling of `DevKnobs` and `match_engine::Knobs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueKnobs {
    /// CA at which value equals `v0` (§9).
    pub ca_ref: f64,
    /// Value anchor at `ca_ref`, in whole currency units.
    pub v0: f64,
    /// Exponential rate: `+ln2/beta` CA points doubles value. `ln2/6` ⇒ every
    /// 6 CA points doubles (§2.2 — CA is an interval scale, so exponential, not
    /// a power law).
    pub beta: f64,
    /// Projection horizon in years (§2.3): `ca_eff` averages `project_ca` over
    /// `t = 0..=horizon_years`.
    pub horizon_years: u32,
    /// Annual discount `δ` on future ability (§2.1).
    pub discount: f64,
    /// Years remaining at/above which the contract discount vanishes (`T`, §2.4).
    pub contract_full_years: f64,
    /// Discount at zero years remaining (`c`, §2.4): `contract_mult` floors at
    /// `1 − c`.
    pub contract_max_discount: f64,
    /// Scarcity multiplier bounds (§2.4) — bounded deliberately: an unbounded
    /// scarcity term is an inflation engine.
    pub scarcity_min: f64,
    pub scarcity_max: f64,
}

impl Default for ValueKnobs {
    fn default() -> Self {
        ValueKnobs {
            ca_ref: 60.0,
            v0: 1_500_000.0,
            beta: std::f64::consts::LN_2 / 6.0,
            horizon_years: 8,
            discount: 0.88,
            contract_full_years: 3.0,
            contract_max_discount: 0.60,
            scarcity_min: 0.85,
            scarcity_max: 1.20,
        }
    }
}

/// The market-state inputs valuation needs beyond the player themself — today
/// just the per-role scarcity multiplier (`TRANSFER_MODEL.md` §2.4). Held
/// separately from `ValueKnobs` because it is *world*-derived, not a tuning
/// constant, and because §2.7's frozen-snapshot rule wants one context computed
/// once per window and shared by every club's valuations. It carries no club
/// decisions — Layer 2 stays decision-free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MarketContext {
    /// League-wide scarcity multiplier per role, already bounded to
    /// `[scarcity_min, scarcity_max]`.
    scarcity: [f64; NUM_ROLES],
}

impl MarketContext {
    /// The neutral context: scarcity 1.0 for every role. The right baseline for
    /// isolating the CA/age/contract behaviour of `value`, and a fine stand-in
    /// wherever league drift is not being modelled.
    pub fn neutral() -> Self {
        MarketContext {
            scarcity: [1.0; NUM_ROLES],
        }
    }

    /// Compute league-wide scarcity (§2.4): role-capable **supply** (players by
    /// best role) against formation-implied **demand** (the mean role headcount
    /// across `FORMATIONS`, scaled by the number of clubs), as a share ratio
    /// bounded to `[scarcity_min, scarcity_max]`. A role in short supply prices
    /// up; an abundant one prices down. Pure function of the world — near-1.0
    /// while the league still mirrors its uniform starting template, its job is
    /// to react to youth-intake drift over decades (§2.4, §8).
    pub fn from_world(world: &World, knobs: &ValueKnobs) -> Self {
        // Formation-implied demand: mean slots per role across the formations,
        // times the number of clubs (each club fields one XI).
        let mut demand = [0.0f64; NUM_ROLES];
        for formation in FORMATIONS.iter() {
            for &role in formation.slots.iter() {
                demand[role.index()] += 1.0;
            }
        }
        let clubs = world.clubs.len() as f64;
        for d in demand.iter_mut() {
            *d = *d / FORMATIONS.len() as f64 * clubs;
        }

        // Supply: every player counted under their current best role.
        let mut supply = [0.0f64; NUM_ROLES];
        for player in world.players.values() {
            let (role, _) = best_role(&player.attributes, &ROLE_WEIGHTS);
            supply[role.index()] += 1.0;
        }

        let total_demand: f64 = demand.iter().sum();
        let total_supply: f64 = supply.iter().sum();
        let mut scarcity = [1.0f64; NUM_ROLES];
        for role in Role::ALL {
            let demand_share = if total_demand > 0.0 {
                demand[role.index()] / total_demand
            } else {
                0.0
            };
            let supply_share = if total_supply > 0.0 {
                supply[role.index()] / total_supply
            } else {
                0.0
            };
            let raw = if supply_share > 0.0 {
                demand_share / supply_share
            } else {
                knobs.scarcity_max // no one can play the role: maximally scarce
            };
            scarcity[role.index()] = raw.clamp(knobs.scarcity_min, knobs.scarcity_max);
        }
        MarketContext { scarcity }
    }

    #[inline]
    fn for_role(&self, role: Role) -> f64 {
        self.scarcity[role.index()]
    }
}

/// Knob-derived, player-independent tables the projection reuses. The
/// `DEVELOPMENT_MODEL.md` §2.2 `NORM`s, the per-category envelope peaks, and the
/// role ceiling constants depend only on `DevKnobs`, not on any player, so they
/// are computed **once** and shared across every player in a window — the
/// knob-level analogue of §2.7's frozen-snapshot cache.
struct DevTables {
    norms: [f64; NUM_ROLES],
    peaks: [f64; NUM_CATEGORIES],
    ceiling_consts: [f64; NUM_ROLES],
}

impl DevTables {
    fn new(knobs: &DevKnobs) -> Self {
        DevTables {
            norms: norms_by_role(knobs),
            peaks: category_peaks(knobs),
            ceiling_consts: role_ceiling_consts(),
        }
    }
}

#[inline]
fn attrs_to_domain(cur: &[f64; NUM_ATTRIBUTES]) -> fforge_domain::Attributes {
    let mut vals = [0u8; NUM_ATTRIBUTES];
    for attr in Attribute::ALL {
        vals[attr.index()] = cur[attr.index()].round().clamp(0.0, 100.0) as u8;
    }
    fforge_domain::Attributes::new(vals)
}

/// Project a player's **best-role CA `years_ahead` years from `today`**, running
/// the `DEVELOPMENT_MODEL.md` §2 growth law forward from their current
/// attributes (`TRANSFER_MODEL.md` §2.3).
///
/// Faithful to the real monthly law by construction: it steps in the same
/// 30-day periods the development fold uses and calls the *same*
/// `development::attr_rate`, only with the noise removed and two neutral
/// projection assumptions — **minutes = regular (1.0)** and **coaching = 1.0**,
/// both independent of the holding club (§2.3's documented counterfactuals: a
/// fee prices what a player is worth to a club that will play and coach him,
/// not what he is worth on his present bench under his present academy; this
/// also keeps valuation club-independent, which §5's simultaneity requires).
/// Jitter and the integer ±1 quantization are the *recording* layer (§5) and are
/// intentionally absent — the projection accumulates the continuous rate in
/// float so month-sized sub-unit growth is not rounded to a standstill.
///
/// `today` is required (unlike the §2.3 sketch's signature): the growth law is
/// age-dependent — envelope, plasticity, and the aging peaks are all functions
/// of age — so the projection must know where on the arc the player stands now.
pub fn project_ca(
    world: &World,
    player: fforge_domain::PlayerId,
    today: GameDate,
    years_ahead: u32,
    knobs: &DevKnobs,
) -> u8 {
    project_ca_with(world, player, today, years_ahead, knobs, &DevTables::new(knobs))
}

/// Project many players at once, building the knob-derived `DevTables`
/// (the `norms_by_role`/`category_peaks` grid scans) **once** rather than
/// once per player. `project_ca` above is the right call for a single
/// lookup; a caller projecting a whole squad or league (`club_ai::observe`'s
/// per-squad-member succession-risk projection is the motivating case) should
/// use this instead — the projected *value* is identical either way, this
/// only removes redundant, purely knob-derived recomputation.
pub fn project_ca_batch(
    world: &World,
    players: impl IntoIterator<Item = fforge_domain::PlayerId>,
    today: GameDate,
    years_ahead: u32,
    knobs: &DevKnobs,
) -> BTreeMap<fforge_domain::PlayerId, u8> {
    let tables = DevTables::new(knobs);
    players
        .into_iter()
        .map(|pid| {
            (
                pid,
                project_ca_with(world, pid, today, years_ahead, knobs, &tables),
            )
        })
        .collect()
}

fn project_ca_with(
    world: &World,
    player: fforge_domain::PlayerId,
    today: GameDate,
    years_ahead: u32,
    knobs: &DevKnobs,
    tables: &DevTables,
) -> u8 {
    let p = world.player(player);
    if years_ahead == 0 {
        return best_role(&p.attributes, &ROLE_WEIGHTS).1;
    }

    let dt = knobs.dt();
    let phi = p.development.bloomer_phase();
    let e = p.development.efficiency();
    let pa = p.character.potential as f64;
    let prof = p.character.professionalism as f64;
    // Professionalism flattens physical aging (§3), exactly as `tick_changes`
    // resolves it — the pro ages well.
    let phys_lmax = knobs.env_phys.lmax * (1.0 - knobs.prof_aging_coeff * (prof - 50.0) / 50.0);
    let age0 = (today.days - p.birth.days) as f64 / DAYS_PER_YEAR;

    let mut cur = [0.0f64; NUM_ATTRIBUTES];
    for attr in Attribute::ALL {
        cur[attr.index()] = p.attributes.get(attr) as f64;
    }

    // Step in 30-day periods to the target year, integrating the shared law.
    let steps = (years_ahead as f64 * DAYS_PER_YEAR / development::DEV_TICK_PERIOD_DAYS as f64)
        .round() as u32;
    for step in 0..steps {
        // Age at the start of the step (as `tick_changes` uses the tick date).
        let y = age0 + step as f64 * dt - phi;
        // Best role can shift as attributes develop; re-derive each period,
        // mirroring `tick_changes` (which recomputes best_role per tick).
        let snapshot = attrs_to_domain(&cur);
        let (role, _) = best_role(&snapshot, &ROLE_WEIGHTS);
        let norm = tables.norms[role.index()];
        let pa_base = pa - knobs.ceil_spread * tables.ceiling_consts[role.index()];
        for attr in Attribute::ALL {
            let rate = attr_rate(
                knobs,
                role,
                attr,
                cur[attr.index()],
                norm,
                pa_base,
                e,
                1.0, // coaching — neutral of the holding club (§2.3)
                1.0, // minutes — regular; prices what a club that plays him gets (§2.3)
                y,
                phys_lmax,
                &tables.peaks,
            );
            cur[attr.index()] = (cur[attr.index()] + rate * dt).clamp(0.0, 100.0);
        }
    }
    best_role(&attrs_to_domain(&cur), &ROLE_WEIGHTS).1
}

#[inline]
fn year_steps(years_ahead: u32) -> u32 {
    (years_ahead as f64 * DAYS_PER_YEAR / development::DEV_TICK_PERIOD_DAYS as f64).round() as u32
}

/// Project a player's best-role CA at **every** whole year from `0` to
/// `max_years_ahead`, in one forward integration. `value_with`'s `ca_eff` sum
/// needs exactly this range (`t = 0..=horizon_years`); since each year's
/// trajectory is a strict prefix of the next one's (the law only ever steps
/// forward from the current attributes), integrating once and snapshotting at
/// each year boundary yields results **identical** to calling
/// `project_ca_with` independently per year — this only removes the redundant
/// re-integration of the same early steps `horizon_years + 1` times over.
fn project_ca_series(
    world: &World,
    player: fforge_domain::PlayerId,
    today: GameDate,
    max_years_ahead: u32,
    knobs: &DevKnobs,
    tables: &DevTables,
) -> Vec<u8> {
    let p = world.player(player);
    let start_ca = best_role(&p.attributes, &ROLE_WEIGHTS).1;
    if max_years_ahead == 0 {
        return vec![start_ca];
    }

    let dt = knobs.dt();
    let phi = p.development.bloomer_phase();
    let e = p.development.efficiency();
    let pa = p.character.potential as f64;
    let prof = p.character.professionalism as f64;
    let phys_lmax = knobs.env_phys.lmax * (1.0 - knobs.prof_aging_coeff * (prof - 50.0) / 50.0);
    let age0 = (today.days - p.birth.days) as f64 / DAYS_PER_YEAR;

    let mut cur = [0.0f64; NUM_ATTRIBUTES];
    for attr in Attribute::ALL {
        cur[attr.index()] = p.attributes.get(attr) as f64;
    }

    let mut result = vec![start_ca];
    let total_steps = year_steps(max_years_ahead);
    let mut next_year = 1u32;
    let mut next_year_steps = year_steps(next_year);

    for step in 0..total_steps {
        let y = age0 + step as f64 * dt - phi;
        let snapshot = attrs_to_domain(&cur);
        let (role, _) = best_role(&snapshot, &ROLE_WEIGHTS);
        let norm = tables.norms[role.index()];
        let pa_base = pa - knobs.ceil_spread * tables.ceiling_consts[role.index()];
        for attr in Attribute::ALL {
            let rate = attr_rate(
                knobs,
                role,
                attr,
                cur[attr.index()],
                norm,
                pa_base,
                e,
                1.0,
                1.0,
                y,
                phys_lmax,
                &tables.peaks,
            );
            cur[attr.index()] = (cur[attr.index()] + rate * dt).clamp(0.0, 100.0);
        }
        // A year boundary (possibly more than one, at short horizons) may
        // land on this exact step — record every one reached.
        let reached = step + 1;
        while next_year <= max_years_ahead && reached >= next_year_steps {
            result.push(best_role(&attrs_to_domain(&cur), &ROLE_WEIGHTS).1);
            next_year += 1;
            if next_year <= max_years_ahead {
                next_year_steps = year_steps(next_year);
            }
        }
    }
    result
}

/// The contract multiplier (§2.4): `1 − c·(1 − min(yrs_left, T)/T)`. Full value
/// at `T`+ years, `1 − c` in the final months. This is the mechanism behind the
/// sell-now-or-lose-him decision. A free agent (`None`) is treated as zero years
/// remaining — the floor — since there is no contract term left to protect.
fn contract_multiplier(contract: Option<Contract>, today: GameDate, knobs: &ValueKnobs) -> f64 {
    let yrs_left = match contract {
        Some(c) => ((c.expires.days - today.days) as f64 / DAYS_PER_YEAR).max(0.0),
        None => 0.0,
    };
    let t = knobs.contract_full_years;
    1.0 - knobs.contract_max_discount * (1.0 - yrs_left.min(t) / t)
}

/// Price one player (`TRANSFER_MODEL.md` §2.1). Pure; see the module docs for
/// the shape and the purity bar. `knobs` is the pricing curve; `dev` is the
/// development law the projection runs (both are needed — `value` composes the
/// two design-once artifacts, and the §2.3 sketch's single-`knobs` signature
/// elides the `DevKnobs` the projection cannot do without).
pub fn value(
    world: &World,
    player: fforge_domain::PlayerId,
    today: GameDate,
    ctx: &MarketContext,
    knobs: &ValueKnobs,
    dev: &DevKnobs,
) -> Money {
    value_with(world, player, today, ctx, knobs, dev, &DevTables::new(dev))
}

fn value_with(
    world: &World,
    player: fforge_domain::PlayerId,
    today: GameDate,
    ctx: &MarketContext,
    knobs: &ValueKnobs,
    dev: &DevKnobs,
    tables: &DevTables,
) -> Money {
    // ca_eff: the δ-discounted mean of projected CA over the horizon (§2.1) —
    // pricing the whole career profile, with the convexity below applied to it.
    // One integration covers every `t` in the sum (`project_ca_series`):
    // identical numbers to calling `project_ca_with` per `t`, without
    // re-integrating the same early years `horizon_years + 1` times over.
    let series = project_ca_series(world, player, today, knobs.horizon_years, dev, tables);
    let mut num = 0.0;
    let mut den = 0.0;
    let mut disc = 1.0;
    for &ca in &series {
        num += disc * ca as f64;
        den += disc;
        disc *= knobs.discount;
    }
    let ca_eff = num / den;

    // One curve: exponential convexity (§2.2) applied to the career-mean CA.
    let base = knobs.v0 * (knobs.beta * (ca_eff - knobs.ca_ref)).exp();

    let p = world.player(player);
    let contract_mult = contract_multiplier(p.contract, today, knobs);
    let (role, _) = best_role(&p.attributes, &ROLE_WEIGHTS);
    let scarcity_mult = ctx.for_role(role);

    Money((base * contract_mult * scarcity_mult).round() as i64)
}

/// Value **every** player against one frozen snapshot into a
/// `BTreeMap<PlayerId, Money>` (`TRANSFER_MODEL.md` §2.7). This is not only the
/// once-per-window optimisation: it *guarantees* every club prices against the
/// same world, which §5's simultaneous market clearing requires. The
/// knob-derived tables are built once and shared across the whole league.
pub fn value_all(
    world: &World,
    today: GameDate,
    ctx: &MarketContext,
    knobs: &ValueKnobs,
    dev: &DevKnobs,
) -> BTreeMap<fforge_domain::PlayerId, Money> {
    let tables = DevTables::new(dev);
    world
        .players
        .keys()
        .map(|&pid| (pid, value_with(world, pid, today, ctx, knobs, dev, &tables)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fforge_domain::{
        Attributes, Character, Competition, CompetitionId, DevProfile, Player, PlayerId,
    };
    use std::collections::BTreeMap;

    const TODAY: GameDate = GameDate { days: 2026 * 365 };

    /// A player whose attributes are shaped toward `role` around `base` (the way
    /// `worldgen` shapes them), at a given age and PA. Reading the actual
    /// best-role CA off the result lets tests compare against *its own* current
    /// CA without needing the shaping to hit an exact number. `E` = the default
    /// mean, `φ` = 0, average professionalism (so no aging adjustment).
    fn shaped_player(id: u32, role: Role, base: i32, age_years: i64, pa: u8) -> Player {
        let mut vals = [0u8; NUM_ATTRIBUTES];
        for attr in Attribute::ALL {
            let w = ROLE_WEIGHTS.weight(role, attr) as i32;
            vals[attr.index()] = (base + (w - 3) * 5).clamp(1, 99) as u8;
        }
        Player {
            id: PlayerId(id),
            name: "Test".to_string(),
            birth: TODAY.add_days(-age_years * 365),
            natural_role: role,
            attributes: Attributes::new(vals),
            character: Character {
                potential: pa,
                determination: 50,
                professionalism: 50,
                consistency: 50,
                injury_proneness: 50,
                leadership: 50,
            },
            development: DevProfile {
                efficiency_milli: 720,
                bloomer_phase_centi: 0,
            },
            contract: Some(Contract {
                wage: Money(1_000),
                expires: TODAY.add_days(4 * 365), // full-value (3+ yr) contract
            }),
            retired: false,
        }
    }

    /// A minimal world holding just the given players — valuation reads only
    /// `world.player(pid)` (and, for scarcity, the club/player counts, which the
    /// tests below use `MarketContext::neutral()` to sidestep).
    fn mini_world(players: Vec<Player>) -> World {
        let mut map = BTreeMap::new();
        for p in players {
            map.insert(p.id, p);
        }
        World {
            players: map,
            clubs: BTreeMap::new(),
            staff: BTreeMap::new(),
            competition: Competition {
                id: CompetitionId(0),
                name: "Test".to_string(),
                clubs: Vec::new(),
            },
        }
    }

    fn current_ca(world: &World, id: u32) -> u8 {
        best_role(&world.player(PlayerId(id)).attributes, &ROLE_WEIGHTS).1
    }

    /// Overwrite a player's PA in place — a couple of tests pin PA to the
    /// current CA to isolate aging-envelope effects from PA-gap growth/decline.
    fn set_pa(world: &mut World, id: u32, pa: u8) {
        if let Some(p) = world.players.get_mut(&PlayerId(id)) {
            p.character.potential = pa;
        }
    }

    #[test]
    fn value_is_monotonic_in_ca_at_fixed_age() {
        // §2.2: the base curve is strictly increasing in ability. At a fixed age
        // and PA, a uniformly stronger player is worth strictly more.
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::neutral();
        let mut last = Money(i64::MIN);
        let mut last_ca = 0u8;
        for base in [45, 60, 75] {
            let world = mini_world(vec![shaped_player(0, Role::Cm, base, 24, 99)]);
            let v = value(&world, PlayerId(0), TODAY, &ctx, &vk, &dev);
            let ca = current_ca(&world, 0);
            assert!(
                v.0 > last.0,
                "value must rise with CA: CA {ca} priced {} <= CA {last_ca} priced {}",
                v.0,
                last.0
            );
            last = v;
            last_ca = ca;
        }
    }

    #[test]
    fn wonderkid_prices_above_and_veteran_below_their_current_ca() {
        // The §2.3 crossover. A benchmark = the value of a hypothetical player
        // whose *effective* CA equals their current CA (base curve only, since
        // both test players carry a full contract and we price at neutral
        // scarcity). Pricing above the benchmark ⟺ ca_eff > current CA.
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::neutral();
        let benchmark = |ca: u8| vk.v0 * (vk.beta * (ca as f64 - vk.ca_ref)).exp();

        // 19-year-old winger with real PA headroom: prices above current CA.
        let wk = mini_world(vec![shaped_player(0, Role::W, 55, 19, 88)]);
        let wk_ca = current_ca(&wk, 0);
        let wk_val = value(&wk, PlayerId(0), TODAY, &ctx, &vk, &dev).0 as f64;
        assert!(
            wk_val > benchmark(wk_ca),
            "wonderkid (CA {wk_ca}) priced {wk_val:.0} should exceed its \
             current-CA benchmark {:.0}",
            benchmark(wk_ca)
        );

        // 33-year-old winger, no headroom (PA = current CA): prices below.
        let mut vet = mini_world(vec![shaped_player(0, Role::W, 82, 33, 60)]);
        let vet_ca = current_ca(&vet, 0);
        // Pin PA to exactly the current CA so only decline can move ca_eff.
        set_pa(&mut vet, 0, vet_ca);
        let vet_val = value(&vet, PlayerId(0), TODAY, &ctx, &vk, &dev).0 as f64;
        assert!(
            vet_val < benchmark(vet_ca),
            "veteran (CA {vet_ca}) priced {vet_val:.0} should fall below its \
             current-CA benchmark {:.0}",
            benchmark(vet_ca)
        );
    }

    #[test]
    fn contract_multiplier_hits_full_value_and_the_floor() {
        // §2.4: 1.0 at 3+ years, 1 − c at zero years, and the shape between.
        let vk = ValueKnobs::default();
        let expires_in = |yrs: f64| Contract {
            wage: Money(0),
            expires: TODAY.add_days((yrs * 365.0) as i64),
        };
        let m = |yrs: f64| contract_multiplier(Some(expires_in(yrs)), TODAY, &vk);
        assert!((m(5.0) - 1.0).abs() < 1e-9, "5 years should be full value");
        assert!((m(3.0) - 1.0).abs() < 1e-9, "3 years should be full value");
        assert!(
            (m(0.0) - (1.0 - vk.contract_max_discount)).abs() < 1e-9,
            "zero years should hit the 1 − c floor"
        );
        assert!(
            (m(1.0) - 0.6).abs() < 1e-9,
            "one year: 1 − 0.6·(1 − 1/3) = 0.6"
        );
        // A free agent has no term left to protect: the floor.
        assert!(
            (contract_multiplier(None, TODAY, &vk) - (1.0 - vk.contract_max_discount)).abs() < 1e-9,
            "a free agent prices at the contract floor"
        );
    }

    #[test]
    fn physical_role_depreciates_faster_than_a_technical_one() {
        // The emergent `DevCategory` property (§2.3): at equal age and comparable
        // CA, a physically-reliant winger loses more CA over the next years than
        // a technical centre-back — asserted, not special-cased. It falls out of
        // the shared growth law reading each attribute's category envelope.
        let dev = DevKnobs::default();
        let mut winger = mini_world(vec![shaped_player(0, Role::W, 80, 32, 99)]);
        let mut cb = mini_world(vec![shaped_player(0, Role::Cb, 80, 32, 99)]);

        // Pin PA to each player's current CA so the ONLY downward force is the
        // aging envelope (no PA-gap decline to swamp the category difference) —
        // isolating the emergent `DevCategory` effect the test is about.
        let w_now = current_ca(&winger, 0) as i32;
        let cb_now = current_ca(&cb, 0) as i32;
        set_pa(&mut winger, 0, w_now as u8);
        set_pa(&mut cb, 0, cb_now as u8);
        let w_future = project_ca(&winger, PlayerId(0), TODAY, 4, &dev) as i32;
        let cb_future = project_ca(&cb, PlayerId(0), TODAY, 4, &dev) as i32;

        let w_drop = w_now - w_future;
        let cb_drop = cb_now - cb_future;
        assert!(
            w_drop > cb_drop,
            "winger should decline faster: winger {w_now}->{w_future} (drop {w_drop}) vs \
             centre-back {cb_now}->{cb_future} (drop {cb_drop})"
        );
    }

    #[test]
    fn scarcity_from_a_starting_world_stays_within_bounds() {
        // §2.4: the scarcity term is bounded to [0.85, 1.20] deliberately — an
        // unbounded one is an inflation engine. On a fresh league it reflects the
        // template-vs-formation role mix, always inside the band.
        let cfg = crate::WorldGenConfig::default();
        let (world, _s, _d) = crate::worldgen::generate(1, &cfg);
        let vk = ValueKnobs::default();
        let ctx = MarketContext::from_world(&world, &vk);
        for role in Role::ALL {
            let s = ctx.for_role(role);
            assert!(
                (vk.scarcity_min..=vk.scarcity_max).contains(&s),
                "scarcity for {role:?} = {s} out of bounds"
            );
        }
    }

    #[test]
    fn value_all_matches_pointwise_value_and_is_pure() {
        // §2.7: the per-window cache must agree with pointwise `value` against
        // the same frozen snapshot, and valuation must be deterministic (pure).
        let cfg = crate::WorldGenConfig {
            num_clubs: 4,
            ..Default::default()
        };
        let (world, _s, start) = crate::worldgen::generate(3, &cfg);
        let dev = DevKnobs::default();
        let vk = ValueKnobs::default();
        let ctx = MarketContext::from_world(&world, &vk);
        let cache = value_all(&world, start, &ctx, &vk, &dev);
        let cache2 = value_all(&world, start, &ctx, &vk, &dev);
        assert_eq!(cache, cache2, "valuation must be deterministic");
        for (&pid, &v) in &cache {
            assert_eq!(v, value(&world, pid, start, &ctx, &vk, &dev));
            assert!(v.0 > 0, "a contracted league player should price positive");
        }
    }
}
