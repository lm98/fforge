//! The calibration knob table (`MATCH_MODEL.md` §8), transcribed verbatim
//! from the fitted `Knobs` dataclass in `match_model_prototype.ipynb`
//! (the throwaway Python shape-finder — this Rust struct is the port
//! target, not a re-guess). Field names and values match the notebook
//! 1:1 so the two stay diffable against each other.
//!
//! The notebook's own `report()` over this default table reads: goals/game
//! 2.69 (target ~2.6), home/draw/away 41/28/31% (target ~45/26/29%),
//! shots/game 27.6 (target ~25), wide-origin goal share 25% (target
//! 25–35%) — a fitted starting point, not a finished calibration; the Rust
//! calibration harness (`MATCH_MODEL.md` §10) is a separate deliverable.

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
            b_beat: -1.7,
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
        }
    }
}