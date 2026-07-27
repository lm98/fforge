//! Read-only renders.
//!
//! **Every function in here is a pure function of `Session`/telemetry state
//! that returns a `String`** (R16). Nothing under `screens/` prints, prompts,
//! or mutates — `main` and the flows do the I/O. That is what makes the whole
//! presentation layer snapshot-testable, and it is the same discipline
//! `MatchEvent::commentary` already followed: build the string in a pure
//! function, let the caller do the I/O.
//!
//! Multi-step interactions live in `flows/` instead; they are state machines
//! over input and are not required to be pure.
//!
//! Each screen returns a string that already ends in a newline, so callers
//! print it with `print!`, not `println!`.

pub mod fixtures;
pub mod header;
pub mod season_end;
pub mod squad;
pub mod stats;
pub mod table;

#[cfg(test)]
mod tests;
