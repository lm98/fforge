//! Match condition (`MATCH_MODEL.md` §13, batch-3 T9): the between-match
//! fatigue carryover, **derived** from a player's recent-appearance window —
//! never stored on `Player`, never its own event (`ATTRIBUTE_SCHEMA.md` §4's
//! CA principle applied to fatigue). `GameState::condition` is the one path
//! to a value; there is no field to desync.
//!
//! Each recent appearance leaves a decaying "load" debt; recovery clears it
//! at a per-day rate set by hidden Natural Fitness (faster) and age (a little
//! slower past `age_anchor`). Condition is `1.0 − Σ debts`, floored so a
//! congested run depresses but never erases a player's contribution.

use fforge_domain::GameDate;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConditionKnobs {
    /// Load debt one match appearance leaves, before recovery.
    pub drain_per_match: f64,
    /// Baseline recovery per day at Natural Fitness 50, age at `age_anchor`.
    pub recovery_base: f64,
    /// How much Natural Fitness scales recovery: at NF 100, recovery is
    /// `recovery_base * (1 + recovery_nf)`; at NF 0, `recovery_base * (1 -
    /// recovery_nf)`.
    pub recovery_nf: f64,
    /// Recovery lost per year of age past `age_anchor`.
    pub recovery_age: f64,
    pub age_anchor: f64,
    /// The age penalty never drops the recovery rate's multiplier below this.
    pub recovery_age_floor: f64,
    /// Condition never reads below this, however congested the run.
    pub condition_floor: f64,
}

impl Default for ConditionKnobs {
    fn default() -> Self {
        // Plausibility-picked (sibling of `DevKnobs`/`ValueKnobs`), tuned so
        // the doc's own honesty note holds: an average-NF player clears one
        // match's debt in under a week (drain 0.12 / recovery 0.03 per day =
        // 4 days), while a low-NF player still carries a small residual into
        // next week's fixture (7 * 0.012 = 0.084 < 0.12) — the "low 90s"
        // reading §13 predicts.
        ConditionKnobs {
            drain_per_match: 0.12,
            recovery_base: 0.03,
            recovery_nf: 0.6,
            recovery_age: 0.02,
            age_anchor: 28.0,
            recovery_age_floor: 0.5,
            condition_floor: 0.5,
        }
    }
}

/// Pre-match condition, `[condition_floor, 1.0]`. `recent` is the player's
/// `GameState::recent_appearances` slice — already pruned to
/// `CONDITION_WINDOW_DAYS`, so this never has to bound it again. Empty
/// `recent` (no appearance in the window) always reads exactly `1.0`.
pub fn condition(
    recent: &[GameDate],
    as_of: GameDate,
    natural_fitness: u8,
    age_years: i32,
    k: &ConditionKnobs,
) -> f64 {
    if recent.is_empty() {
        return 1.0;
    }
    let nf_mult = 1.0 + k.recovery_nf * (natural_fitness as f64 - 50.0) / 50.0;
    let age_mult = (1.0 - k.recovery_age * (age_years as f64 - k.age_anchor).max(0.0))
        .max(k.recovery_age_floor);
    let recovery_per_day = (k.recovery_base * nf_mult * age_mult).max(0.0);

    let debt: f64 = recent
        .iter()
        .map(|&d| {
            let days_since = (as_of.days - d.days) as f64;
            (k.drain_per_match - recovery_per_day * days_since).max(0.0)
        })
        .sum();

    (1.0 - debt).clamp(k.condition_floor, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(days: i64) -> GameDate {
        GameDate { days }
    }

    #[test]
    fn no_recent_appearances_reads_exactly_one() {
        let k = ConditionKnobs::default();
        assert_eq!(condition(&[], date(1000), 50, 25, &k), 1.0);
    }

    #[test]
    fn a_fresh_appearance_depresses_condition_below_one() {
        let k = ConditionKnobs::default();
        let c = condition(&[date(1000)], date(1000), 50, 25, &k);
        assert!(c < 1.0, "condition {c} should dip right after a match");
    }

    #[test]
    fn condition_recovers_toward_one_over_days() {
        let k = ConditionKnobs::default();
        let just_played = condition(&[date(1000)], date(1000), 50, 25, &k);
        let a_week_later = condition(&[date(1000)], date(1007), 50, 25, &k);
        assert!(
            a_week_later > just_played,
            "condition should recover with days elapsed: {just_played} -> {a_week_later}"
        );
        assert_eq!(
            a_week_later, 1.0,
            "an average-NF player should fully clear one match's debt within a week"
        );
    }

    #[test]
    fn a_congested_run_stacks_debt_deeper_than_one_match() {
        let k = ConditionKnobs::default();
        let one_match = condition(&[date(998)], date(1000), 50, 25, &k);
        let three_in_a_week =
            condition(&[date(994), date(997), date(1000)], date(1000), 50, 25, &k);
        assert!(
            three_in_a_week < one_match,
            "a congested run must read lower than a single appearance: {three_in_a_week} vs {one_match}"
        );
    }

    #[test]
    fn higher_natural_fitness_recovers_faster() {
        let k = ConditionKnobs::default();
        let low_nf = condition(&[date(998)], date(1000), 0, 25, &k);
        let high_nf = condition(&[date(998)], date(1000), 100, 25, &k);
        assert!(
            high_nf > low_nf,
            "high Natural Fitness should recover faster: {low_nf} vs {high_nf}"
        );
    }

    #[test]
    fn condition_never_drops_below_its_floor() {
        let k = ConditionKnobs::default();
        // An implausibly dense run of appearances every single day.
        let recent: Vec<GameDate> = (980..=1000).map(date).collect();
        let c = condition(&recent, date(1000), 0, 40, &k);
        assert!(
            (k.condition_floor..=1.0).contains(&c),
            "condition {c} must stay within [{}, 1.0]",
            k.condition_floor
        );
    }
}
