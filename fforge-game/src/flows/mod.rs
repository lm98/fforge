//! Multi-step interactions.
//!
//! A flow is a state machine over player input: it prompts, loops, and
//! eventually either abandons or turns the player's choices into a `Command`.
//! Flows print as they go and are *not* required to be pure — that is the line
//! between this module and `screens/` (R16/R17). What they still never do is
//! mutate `GameState` directly: every mutation goes through
//! `Session::execute`.

pub mod advance;
pub mod friendly;
pub mod lineup;
pub mod match_view;
pub mod new_game;
pub mod save;
pub mod season;
pub mod subs;
pub mod tactics;
pub mod transfers;
