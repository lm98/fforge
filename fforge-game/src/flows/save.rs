//! Saving is literally "serialize the event log" — there is no separate
//! save-game format to drift out of sync (`fforge_core::session`).

use crate::SAVE_PATH;
use fforge_core::{Session, save_log};
use std::path::Path;

pub fn do_save(session: &Session) {
    match save_log(Path::new(SAVE_PATH), &session.log) {
        Ok(()) => println!("Saved to ./{SAVE_PATH} ({} events).", session.log.len()),
        Err(e) => println!("Save failed: {e}"),
    }
}
