//! fforge-harness — the calibration and pathology harnesses.
//!
//! Each module drives the real `fforge-core` pipeline over many seeds and
//! reports emergent statistics against the bands its design note sets. A
//! harness is a pure consumer of the simulation: it reads the public surface
//! of `fforge-core`, prints, and never feeds a reading back into a knob —
//! the re-fit is a human reading the numbers.
//!
//! | Module | Measures | Design note | Runner |
//! |---|---|---|---|
//! | `match_engine` | the match engine and tactics | `MATCH_MODEL.md` §10, `TACTICS_MODEL.md` §5, §8 | `bin/calibrate` |
//! | `career_arc` | player development and PA recoverability | `DEVELOPMENT_MODEL.md` §6, §8 | `bin/career_arc` |
//! | `market` | transfer-market pathologies | `TRANSFER_MODEL.md` §11 | `bin/market` |

#![deny(unsafe_code)]

pub mod career_arc;
pub mod market;
pub mod match_engine;
