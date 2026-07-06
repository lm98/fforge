//! `Session` glues the pieces: it owns the append-only log and the folded
//! state, routes commands through `step`, and notifies observers. Save/load
//! is *literally* serialize/replay the log — there is no separate save-game
//! format to drift out of sync.

use crate::commands::{step, Command, CommandError};
use crate::event::Event;
use crate::observer::EventObserver;
use crate::state::GameState;
use std::io::{BufRead, Write};
use std::path::Path;

pub struct Session {
    pub state: GameState,
    pub log: Vec<Event>,
}

impl Session {
    /// Start a session from a log (a fresh `[GameStarted]` or a loaded save).
    /// Observers see every event, replayed ones included, so derived consumers
    /// (telemetry, traces) rebuild for free.
    pub fn from_events(log: Vec<Event>, observers: &mut [&mut dyn EventObserver]) -> Session {
        let state = GameState::replay(&log);
        for event in &log {
            for obs in observers.iter_mut() {
                obs.on_event(event);
            }
        }
        Session { state, log }
    }

    /// Validate + produce events, append them, fold them, notify observers.
    /// Returns the slice of newly produced events for presentation.
    pub fn execute(
        &mut self,
        command: Command,
        observers: &mut [&mut dyn EventObserver],
    ) -> Result<&[Event], CommandError> {
        let events = step(&self.state, command)?;
        let start = self.log.len();
        for event in events {
            self.state.apply(&event);
            for obs in observers.iter_mut() {
                obs.on_event(&event);
            }
            self.log.push(event);
        }
        Ok(&self.log[start..])
    }
}

/// JSON-lines event log persistence: one event per line, append-friendly,
/// diffable, trivially inspectable. (SQLite — the settled storage choice —
/// arrives when there is queryable state worth indexing, Phase 2 stats; see
/// the delivery notes.)
pub fn save_log(path: &Path, log: &[Event]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    for event in log {
        let line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(writer, "{line}")?;
    }
    writer.flush()
}

pub fn load_log(path: &Path) -> std::io::Result<Vec<Event>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut log = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Event = serde_json::from_str(&line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        log.push(event);
    }
    Ok(log)
}