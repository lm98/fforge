//! The sim calendar. Game-time derives from the event stream (hard
//! invariant), so this is a pure value type: a day counter over flat 365-day
//! sim years. No leap years, no wall clock — a sim calendar, not a civil one.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const DAYS_PER_YEAR: i64 = 365;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GameDate {
    /// Days since sim-epoch (year 0, day 0).
    pub days: i64,
}

impl GameDate {
    pub fn from_year_day(year: i32, day_of_year: u16) -> Self {
        GameDate {
            days: year as i64 * DAYS_PER_YEAR + day_of_year as i64,
        }
    }

    pub fn year(self) -> i32 {
        (self.days.div_euclid(DAYS_PER_YEAR)) as i32
    }

    pub fn day_of_year(self) -> u16 {
        self.days.rem_euclid(DAYS_PER_YEAR) as u16
    }

    #[must_use]
    pub fn add_days(self, d: i64) -> Self {
        GameDate { days: self.days + d }
    }

    /// Whole sim-years elapsed since `birth` — a player's age.
    pub fn years_since(self, birth: GameDate) -> i32 {
        ((self.days - birth.days) / DAYS_PER_YEAR) as i32
    }
}

impl fmt::Display for GameDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, day {:03}", self.year(), self.day_of_year())
    }
}