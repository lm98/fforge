//! The calibration knob table (`MATCH_MODEL.md` §8). Field names and every
//! value except `b_beat` are transcribed verbatim from the fitted `Knobs`
//! dataclass in `match_model_prototype.ipynb` (the throwaway Python
//! shape-finder — this Rust struct is the port target, not a re-guess), so
//! the two stay diffable against each other.
//!
//! `b_beat` (beat-the-keeper bias) is the one Rust-side re-tune, landed by
//! the calibration harness (`bin/calibrate.rs`, `MATCH_MODEL.md` §10):
//! real `worldgen`'s attribute distribution differs enough from the
//! notebook's own synthetic squad generator that the notebook's fitted
//! -1.7 under-converted against real inputs (~7% vs the ~10% target) while
//! every other aggregate — shots/game, on-target rate, wide-origin share —
//! already landed on target. `b_beat` only gates the second of the two
//! chained shot sigmoids (beat-the-keeper, given on-target), so raising it
//! moves conversion without disturbing the already-correct on-target rate
//! or shot volume — confirmed empirically (`bin/calibrate.rs -- --seeds
//! 16`): shots/game stayed ~25.6, on-target stayed ~33.2%, across the whole
//! sweep. Pooled over 16 seeds at -1.05: goals/game 2.58 (target ~2.6),
//! home/draw/away 43/26/31% (target ~45/26/29%), conversion 10.1% (target
//! ~10%), wide-origin goal share 26.7% (target 25–35%) — this is now a
//! finished, real-`worldgen`-calibrated point, not just the notebook's.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knobs {
    // --- clock ---
    /// Match-minutes advanced per possession step.
    pub delta: f64,

    // --- logistic resolver: p = sigmoid(k*(atk-def)/s + bias) ---
    /// Attribute-difference sensitivity (open-play contests).
    pub k: f64,
    /// Scale — normalizes the 0..100 attribute range.
    pub s: f64,
    /// Additive edge to the home side's attacking contests.
    pub home_bias: f64,

    // --- per-action base rates (bias term) ---
    pub b_pass: f64,
    pub b_takeon: f64,
    pub b_cross_delivery: f64,

    // --- shot resolver: two sigmoids (on-target, then beat-keeper) ---
    pub k_ontarget: f64,
    pub k_gk: f64,
    pub b_ontarget: f64,
    pub b_beat: f64,
    /// Of shots that miss the on-target sigmoid, the share narrated as
    /// "off" vs "blocked" — a cosmetic split only (both transition to the
    /// same opponent-`Def` outcome; `MATCH_MODEL.md` §3's transition table
    /// treats them as one branch, §9's stream schema wants them as two
    /// distinct, narratable outcomes).
    pub p_off_frac: f64,
    /// Save → parried rebound (follow-up shot) vs collected.
    pub p_rebound: f64,
    /// Rebound chances are scrappy.
    pub q_rebound: f64,

    // --- chance quality by arrival (added into both shot sigmoids) ---
    pub q_through: f64,
    pub q_dribble: f64,
    pub q_cutback: f64,
    pub q_header: f64,
    pub q_long: f64,

    // --- transition splits ---
    /// Completed build-up pass advances `Def` → `Mid` (else retain `Def`).
    pub p_def_advance: f64,
    /// Completed `Mid` action advances to the final third.
    pub p_mid_advance: f64,
    /// Of advances, share going wide (`AttW`) vs central (`AttC`).
    pub p_wide: f64,
    /// Through-ball reaches the box.
    pub p_attc_penetrate: f64,
    /// `AttC` take-on reaches the box.
    pub p_attc_dribble_box: f64,
    /// `AttW` take-on becomes a cutback chance.
    pub p_attw_cutback: f64,
    /// `AttW` take-on cuts inside to `AttC` instead.
    pub p_attw_cut_inside: f64,

    // --- action-selection base weights (modulated by the actor's attributes) ---
    pub w_pass_mid: f64,
    pub w_takeon_mid: f64,
    pub w_pass_attc: f64,
    pub w_takeon_attc: f64,
    pub w_longshot_attc: f64,
    pub w_cross_attw: f64,
    pub w_takeon_attw: f64,
    pub w_pass_attw: f64,

    // --- fatigue: effective attr *= 1 - drop, drop grows over 90' ---
    /// Max drop at 90' for a 0-stamina, low-work-rate player.
    pub fatigue_base: f64,
    /// How much Work Rate accelerates fatigue.
    pub fatigue_wr: f64,

    // --- resolution support term: blend actor with team quality ---
    /// 0 = pure actor, 1 = pure team mean.
    pub support: f64,

    // --- consistency: per-match performance-variance scale (MATCH_MODEL.md §17, T8) ---
    /// `σ(consistency) = consistency_sigma_max · (1 − consistency/100)` — the
    /// per-match multiplier's standard deviation. Identity `0.0` (§2.1): every
    /// side's multiplier is then exactly `1.0`, reproducing the pre-2e engine
    /// bit-for-bit.
    pub consistency_sigma_max: f64,
    /// Clamp band for the per-match multiplier — "a bad day is a bad day,
    /// not a different player" (§2.9).
    pub consistency_mult_min: f64,
    pub consistency_mult_max: f64,

    // --- injuries: two hazard channels (MATCH_MODEL.md §14, T10) ---
    /// Global scale on every injury-hazard probability below. Identity `0.0`
    /// (§2.1): no injury ever fires, reproducing the pre-2e engine
    /// bit-for-bit regardless of every other injury knob's value.
    pub injury_rate: f64,
    /// Contact channel: base per-event probability (a failed take-on, or a
    /// headed shot's aerial duel), before `injury_rate`/proneness/intensity
    /// scale it.
    pub injury_base_contact: f64,
    /// How much hidden Injury-proneness scales the contact-channel
    /// probability: at proneness 100, `1 + injury_contact_prone_scale`×; at
    /// 0, `1 − injury_contact_prone_scale`×.
    pub injury_contact_prone_scale: f64,
    /// Professionalism's discount on the contact-channel probability (the
    /// schema's "aging/injury resistance") — same `±(x−50)/50` shape as
    /// `prof_aging_coeff`, but smaller: a modest effect, not aging's main one.
    pub injury_contact_prof_discount: f64,
    /// How much a low `condition` (`MATCH_MODEL.md` §13) deepens contact-
    /// channel risk: at `condition = 0`, the multiplier is `1 +
    /// injury_contact_condition_scale`; at `condition = 1`, exactly `1`.
    pub injury_contact_condition_scale: f64,
    /// How much the *other* player's Aggression raises contact intensity
    /// (the tackler's recklessness raises the tackled player's risk): at
    /// Aggression 100, `1 + injury_aggression_scale`×; at 0, `1 −
    /// injury_aggression_scale`×.
    pub injury_aggression_scale: f64,

    /// Ambient channel: base per-match probability (already integrating a
    /// per-minute hazard over 90', so it is rolled once per player per
    /// match rather than once per minute — see `resolve::build_xi`'s doc
    /// comment) before `injury_rate`/condition/age scale it.
    pub injury_ambient_base: f64,
    /// How much a low `condition` raises ambient risk: at `condition = 0`,
    /// the multiplier is `1 + injury_ambient_condition_scale`; at
    /// `condition = 1`, exactly `1` — "gives §13 teeth" (§14).
    pub injury_ambient_condition_scale: f64,
    /// Per year of age past `injury_age_anchor`, the ambient multiplier
    /// grows by this fraction (older legs break down more from wear alone).
    pub injury_ambient_age_scale: f64,
    pub injury_age_anchor: f64,

    /// Effective-attribute multiplier applied to an injured player for the
    /// remainder of the match, from their injury's onset minute onward
    /// (§2.5: "continues at reduced effectiveness").
    pub injury_impairment_mult: f64,

    // --- severity: a skewed categorical draw, MATCH_MODEL.md §14 ---
    /// Cumulative probabilities for Knock / Minor / Moderate (Severe is the
    /// remainder) — skewed hard toward the small end.
    pub injury_knock_prob: f64,
    pub injury_minor_cum_prob: f64,
    pub injury_moderate_cum_prob: f64,
    /// Day ranges per category, `[min, max)`, interpolated by a second,
    /// independent uniform draw.
    pub injury_knock_days: [f64; 2],
    pub injury_minor_days: [f64; 2],
    pub injury_moderate_days: [f64; 2],
    pub injury_severe_days: [f64; 2],

    // --- fouls & cards (MATCH_MODEL.md §15, T11) ---
    /// Global scale on the foul-check probability. Identity `0.0` (§2.1): no
    /// foul ever fires, reproducing the pre-2e engine bit-for-bit regardless
    /// of every other foul knob's value.
    pub foul_rate: f64,
    /// `p_foul = foul_rate * sigmoid(foul_base + ...)` — the sigmoid's bias
    /// term (schema §6 #8's signature: ↑ Aggression, ↓ Composure/Decisions).
    pub foul_base: f64,
    /// How much the defender's Aggression raises `p_foul`'s logit: at
    /// Aggression 100, `+foul_aggression_scale`; at 0, `-foul_aggression_scale`.
    pub foul_aggression_scale: f64,
    /// How much the defender's Composure/Decisions blend lowers `p_foul`'s
    /// logit — same `±(x-50)/50` shape, subtracted.
    pub foul_composure_scale: f64,
    /// The High-press modulator (§2.6): scales with the defending side's own
    /// `SideEffects::fatigue_mult` press-exertion multiplier (identity
    /// `1.0` → no term).
    pub foul_press_scale: f64,
    /// The tired-legs modulator (§2.6): scales with `1 - fatigue_mult`, so a
    /// more fatigued defender fouls more.
    pub foul_fatigue_scale: f64,
    /// Given a foul fires *and the defender has no card yet this match*,
    /// the base probability it draws a first yellow (of the severity
    /// roll's `[0,1)` range; the remainder below `foul_red_base +
    /// foul_yellow_base` draws no card at all).
    pub foul_yellow_base: f64,
    /// Given a foul fires, the flat probability it draws a straight red
    /// (checked before the yellow/second-yellow band, so it is never
    /// shadowed by them) — independent of whether the defender already has
    /// a yellow.
    pub foul_red_base: f64,
    /// Given a foul fires *and the defender already has a yellow this
    /// match*, the flat probability it draws a second yellow (dismissal).
    /// Deliberately its own knob rather than reusing `foul_yellow_base`
    /// plus the repeat/aggression bumps below: a player already cautioned
    /// referees more strictly and tends to foul more carefully, so a second
    /// bookable act is rarer per-foul than the first, not more common —
    /// reusing the fresh-yellow formula let repeat fouling by one
    /// aggressive player snowball into an implausible per-match red rate.
    pub foul_second_yellow_base: f64,
    /// Per prior foul this match (the referee's own patience, already free
    /// state), how much a *first* booking's yellow probability rises —
    /// does not apply once the defender already has a yellow (see
    /// `foul_second_yellow_base`).
    pub foul_repeat_scale: f64,
    /// How much the defender's Aggression additionally raises the yellow
    /// probability specifically (on top of `foul_aggression_scale`'s effect
    /// on whether a foul happens at all) — same `±(x-50)/50` shape.
    pub foul_yellow_aggression_scale: f64,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs {
            delta: 0.11,
            k: 1.0,
            s: 12.0,
            home_bias: 0.52,
            b_pass: 1.35,
            b_takeon: -0.15,
            b_cross_delivery: -1.3,
            k_ontarget: 0.9,
            k_gk: 0.9,
            b_ontarget: -0.9,
            b_beat: -1.05,
            p_off_frac: 0.5,
            p_rebound: 0.08,
            q_rebound: -0.6,
            q_through: 0.56,
            q_dribble: 0.02,
            q_cutback: 0.6,
            q_header: -0.45,
            q_long: -1.8,
            p_def_advance: 0.55,
            p_mid_advance: 0.2,
            p_wide: 0.34,
            p_attc_penetrate: 0.08,
            p_attc_dribble_box: 0.06,
            p_attw_cutback: 0.08,
            p_attw_cut_inside: 0.30,
            w_pass_mid: 0.85,
            w_takeon_mid: 0.15,
            w_pass_attc: 0.58,
            w_takeon_attc: 0.27,
            w_longshot_attc: 0.05,
            w_cross_attw: 0.35,
            w_takeon_attw: 0.35,
            w_pass_attw: 0.20,
            fatigue_base: 0.12,
            fatigue_wr: 0.5,
            support: 0.25,
            // T8: plausibility-picked, not yet B3.9-fitted. At worldgen's
            // consistency range (25..90), sigma runs from ~0.19 (worst,
            // consistency=25) to ~0.025 (best, consistency=90) — a visibly
            // wider match-to-match spread for the least consistent players.
            consistency_sigma_max: 0.25,
            consistency_mult_min: 0.7,
            consistency_mult_max: 1.3,
            // T10: plausibility-picked against the real engine's own event
            // rates (probed via `bin/calibrate`'s per-match shot/take-on
            // counts), not yet B3.9-fitted. Split roughly 60/40 contact/
            // ambient so both channels are visibly active, then magnitude-
            // scaled once against a real pooled season
            // (`a_pooled_seasons_injury_count_lands_in_the_documented_band`,
            // 6 seeds): the first pick (base_contact 0.006, ambient_base
            // 0.0025) read 3.88 match-missing (days_out >= 7) injuries/club/
            // season, well above §14's 1.5-2.5 target; scaling both down by
            // ~0.515 landed at 2.14 — inside the target band.
            injury_rate: 1.0,
            injury_base_contact: 0.0031,
            injury_contact_prone_scale: 0.6,
            injury_contact_prof_discount: 0.2,
            injury_contact_condition_scale: 0.5,
            injury_aggression_scale: 0.5,
            injury_ambient_base: 0.0013,
            injury_ambient_condition_scale: 0.6,
            injury_ambient_age_scale: 0.02,
            injury_age_anchor: 28.0,
            injury_impairment_mult: 0.6,
            // Skewed hard toward the small end (§14): Knock 55%, Minor 30%,
            // Moderate 12%, Severe 3% (the cumulative remainder).
            injury_knock_prob: 0.55,
            injury_minor_cum_prob: 0.85,
            injury_moderate_cum_prob: 0.97,
            injury_knock_days: [0.0, 3.0],
            injury_minor_days: [7.0, 21.0],
            injury_moderate_days: [28.0, 56.0],
            injury_severe_days: [90.0, 180.0],

            // T11: plausibility-picked, then magnitude-scaled against a real
            // pooled season the same way T10's injury knobs were
            // (`a_season_lands_close_to_the_documented_card_band`) — see
            // MATCH_MODEL.md §15's "T11 finding" for the exact before/after
            // numbers.
            foul_rate: 1.0,
            foul_base: -2.5,
            foul_aggression_scale: 0.6,
            foul_composure_scale: 0.4,
            foul_press_scale: 0.3,
            foul_fatigue_scale: 0.3,
            // §15: "p_yellow ≈ 0.15-0.20 of fouls", "straight red ... rare,
            // ≈ 0.01".
            foul_yellow_base: 0.20,
            foul_red_base: 0.002,
            foul_second_yellow_base: 0.012,
            foul_repeat_scale: 0.03,
            foul_yellow_aggression_scale: 0.15,
        }
    }
}
