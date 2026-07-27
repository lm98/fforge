# fforge-game

Layer 5 (per DESIGN.md) of the fforge workspace: the CLI binary, consuming both
`fforge-domain` and `fforge-core`. It is a thin presentation shell over
`fforge-core::Session` — it renders screens, reads menu input, turns choices into
`Command`s, and prints the resulting `Event`s.

## Module layout — split by role, not by feature (Batch 4 R17)

```
main.rs        entry, game_loop, the menu
screens/       read-only renders — each is a pure fn returning String (R16)
flows/         multi-step interactions (new game/load, lineup, transfers, advance, friendly)
render/        shared formatting + derived readings (the Sem colour vocabulary joins at U2)
input.rs       read_line, prompt_choice, prompt_number, prompt_money, prompt_seed
snapshots/     committed screen snapshots (see Testing)
```

**Screens are pure; flows are not.** A screen is a function of state that returns a
`String` and prints nothing — that is what makes the presentation layer snapshot-testable,
and it is the same discipline `MatchEvent::commentary` already followed one layer down
(build the string in a pure function, let the caller do the I/O). A flow is a state machine
over player input: it prompts, loops, and eventually turns choices into a `Command`.
`main` prints screens with `print!` (every screen's string already ends in a newline).

| Module | Holds |
|---|---|
| `main` | the entry menu, `game_loop`, `SAVE_PATH` |
| `screens::{header, squad, table, fixtures, stats, season_end}` | one `render()` per screen, all pure |
| `flows::new_game` | `new_game_flow` (seed → worldgen → club pick → `GameStarted`), `load_flow` |
| `flows::lineup` | formation + XI picker, submits `Command::SubmitLineup` |
| `flows::transfers` | the `TRANSFER_MODEL.md` §10 pre-commitment UI: a local `Vec<TransferDecision>` draft built against one frozen `club_ai::observe`/`valuation::value_all` snapshot (rebuilt on entry and after a submit only), browsable by target/own-squad/shortlist, submitted in one shot via `Command::SubmitTransferDecision`. Cash and wage headroom stay in the header, since those are what `market::filter_affordable`'s resolve-time gate silently drops a plan on |
| `flows::advance` | `player_match_preview` on the pre-advance state → `Command::AdvanceMatchday` → the match view, that matchday's results, and any transfer window this advance closed |
| `flows::match_view` | the Phase 2 "humble text match view" (`DESIGN.md` §9): `commentary_lines` is pure, `print_humble_text_view` adds the pacing and the skip-on-keypress raw-mode handling |
| `flows::friendly` | an unrecorded friendly between any two clubs (no `Command`, no `Event`) rendered through the match view. **Kept rather than deleted (R17)** — U6's tactics picker makes it a genuine tactics sandbox; wired back into the menu at U7, `#[allow(dead_code)]` until then |
| `flows::save` | `do_save` — saving is literally "serialize the event log" |
| `render` | `ordinal`, `result_line`, `headline_ca`, `club_avg_ca`, `table_position` |
| `input` | the only functions that touch stdin |

## Hard constraints — never violate these

1. **This crate is the only place allowed to touch stdin/stdout and the wall clock.**
   `fforge-domain` and `fforge-core` must stay pure (see their own CLAUDE.md files). Two
   sanctioned wall-clock exceptions: `input::prompt_seed`'s fallback to `SystemTime::now()`
   when the player leaves the seed blank (the chosen seed is immediately recorded in
   `Event::GameStarted`, so replay/`fforge-core` never re-touches the clock), and
   `flows::friendly`'s ad-hoc RNG seed (a friendly is never recorded — no `Event`, nothing
   for replay to reproduce). Any new randomness or timestamp need must be sourced here and
   passed in as data, never added to `fforge-core`/`fforge-domain`.
   `print_humble_text_view`'s terminal raw-mode toggling is the same kind of edge-only
   concern — it's always paired (`enable_raw_mode`/`disable_raw_mode`) around the loop so
   canonical mode is restored before the next `read_line`-based prompt.
2. **All game-state mutation goes through `Session::execute`.** Nothing here mutates
   `GameState` fields directly — it builds a `Command`, calls `execute`, and renders
   whatever `Event`s or error comes back. This keeps the CLI a pure consumer of the
   event-sourced core.
3. **Screens never print, prompt, or mutate.** If a new screen needs to ask something, the
   asking half belongs in a flow. A screen that prints cannot be snapshot-tested, and the
   snapshots are the only thing standing between this crate and silent formatting rot.

## Testing

Snapshot tests live in `screens/tests.rs`, with the expected output committed as plain
`.txt` files under `snapshots/` (no test framework, no new dependency — `assert_eq!`
against `std::fs::read_to_string`). They run against one fixed seed, so the world, the
fixtures, and every result are reproducible.

```
cargo test -p fforge-game                       # verify
UPDATE_SNAPSHOTS=1 cargo test -p fforge-game    # regenerate, then READ THE DIFF
```

`no_ansi_escapes_when_colour_is_disabled` is the load-bearing one: with colour disabled —
which is every screen's state under `NO_COLOR`, `--no-color`, or a non-tty stdout — no
screen may emit an ANSI escape. That single test protects every piped consumer, CI logs
included.

Interactive flows are still verified by hand: `cargo run -p fforge-game` and walk the
affected flow.
